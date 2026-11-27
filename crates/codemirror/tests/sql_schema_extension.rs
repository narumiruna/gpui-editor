use std::sync::Arc;

use gpui_editor_codemirror::{
    Change, ChangeSet, Compartment, EditorState, Extension, Text, Transaction,
    language::sql::{
        NativeSqlSchemaExtension, SqlDialect, SqlDocument, SqlNamespace, SqlSchemaConfig,
    },
};

fn register(dialect: SqlDialect, schema: &SqlNamespace) -> Extension {
    NativeSqlSchemaExtension::new(
        dialect,
        SqlSchemaConfig {
            schema: Some(schema),
            ..Default::default()
        },
    )
    .extension()
}

#[test]
fn registered_sources_match_the_pinned_dialect_and_quoted_ranges() {
    use SqlDialect::*;

    // Expected source results from sql({dialect, schema}) in lang-sql 6.10.0.
    let root = SqlNamespace::Namespaces(vec![
        ("select".into(), SqlNamespace::Completions(vec![])),
        (
            "acct.items".into(),
            SqlNamespace::Completions(vec!["order number".into(), "organization".into()]),
        ),
    ]);
    for (dialect, quote, typed_quote) in [
        (StandardSql, '"', Some(('"', '"'))),
        (PostgreSql, '"', Some(('"', '"'))),
        (MySql, '`', Some(('`', '`'))),
        (MariaSql, '`', Some(('`', '`'))),
        (MsSql, '"', Some(('[', ']'))),
        (Sqlite, '`', Some(('`', '`'))),
        (Cassandra, '"', Some(('"', '"'))),
        (PlSql, '"', None),
    ] {
        let state = EditorState::with_extensions("", register(dialect, &root)).unwrap();
        let source = NativeSqlSchemaExtension::source_for(&state, dialect).unwrap();
        let parse = |text| SqlDocument::parse_with_dialect(Text::new(text), dialect);
        let result = source.complete(&parse("sel".into()), 3, false).unwrap();
        assert_eq!(result.options[0].label, "select", "{dialect:?}");
        assert_eq!(result.options[0].kind.as_deref(), Some("type"));
        let result = source
            .complete(&parse("select acct.items.or".into()), 20, false)
            .unwrap();
        assert_eq!((result.from, result.to), (18, None), "{dialect:?}");
        assert_eq!(result.options[0].label, "order number");
        assert_eq!(result.options[0].kind.as_deref(), Some("property"));
        assert_eq!(
            result.options[0].insert,
            Some(format!("{quote}order number{quote}"))
        );
        assert_eq!(result.options[1].label, "organization");
        assert_eq!(result.options[1].insert, None);

        let (open, close) = typed_quote.unwrap_or(('"', '"'));
        let doc = parse(format!("-- 😀\r\nselect acct.items.{open}or{close}"));
        if typed_quote.is_some() {
            let result = source.complete(&doc, 30, false).unwrap();
            assert_eq!((result.from, result.to), (27, Some(31)), "{dialect:?}");
            assert_eq!(
                result.options[0].label,
                format!("{open}order number{close}")
            );
            assert_eq!(result.options[0].insert, None);
        } else {
            // PLSQL treats double quotes as a string in the pinned parser.
            assert!(source.complete(&doc, 30, false).is_none());
            let result = source.complete(&doc, 30, true).unwrap();
            assert_eq!((result.from, result.to), (30, None));
            assert_eq!(result.options[0].label, "select");
        }
    }
}

#[test]
fn configuration_tracks_precedence_and_removal_independently_for_each_dialect() {
    use SqlDialect::{MySql, StandardSql};

    let compartment = Compartment::new();
    let root = SqlNamespace::Completions(vec!["old".into()]);
    let old = register(StandardSql, &root);
    let other = register(MySql, &root);
    let empty = NativeSqlSchemaExtension::new(StandardSql, SqlSchemaConfig::default()).extension();
    let mut state = EditorState::with_extensions(
        "",
        Extension::group([compartment.of(old.clone()), empty, other]),
    )
    .unwrap();
    let get = NativeSqlSchemaExtension::source_for;
    let original = get(&state, StandardSql).unwrap();
    let other = get(&state, MySql).unwrap();
    state
        .update(Transaction::new(
            ChangeSet::single(Change::new(0..0, "sel")).unwrap(),
        ))
        .unwrap();
    assert!(Arc::ptr_eq(&original, &get(&state, StandardSql).unwrap()));

    let new = register(StandardSql, &SqlNamespace::Completions(vec!["new".into()]));
    for (extension, label) in [
        (Extension::group([old.clone(), new.clone()]), Some("old")),
        (Extension::group([new, old.clone()]), Some("new")),
        (Extension::default(), None),
        (old, Some("old")),
    ] {
        state
            .update(Transaction::default().with_effects([compartment.reconfigure(extension)]))
            .unwrap();
        let source = get(&state, StandardSql);
        let doc = SqlDocument::parse(Text::default());
        assert_eq!(
            source
                .as_ref()
                .map(|source| source.complete(&doc, 0, true).unwrap().options[0]
                    .label
                    .clone()),
            label.map(str::to_owned)
        );
        assert!(Arc::ptr_eq(&other, &get(&state, MySql).unwrap()));
    }
    assert!(!Arc::ptr_eq(&original, &get(&state, StandardSql).unwrap()));
}
