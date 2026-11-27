use std::sync::Arc;

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorState, Extension,
    Text, Transaction,
    language::{
        HighlightLayer::Active,
        HighlightProperties, HighlightStyle, HighlightTag,
        HighlightTheme::Light,
        LanguageData, highlighting_for_context,
        lezer::ParseError,
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
            markdown_snapshot, markdown_with_parser,
        },
        wast::{
            LANGUAGE_DATA, NodeProp, SUBLANGUAGES, WastDataFacet, WastSublanguage,
            WastSublanguageMode, style_tags,
        },
    },
    language_data::{NativeLanguage, SyntaxUpdateError, WastLanguage, wast_language},
};
use serde_json::{Value, json};

fn rows() -> impl Iterator<Item = Value> {
    include_str!("fixtures/markdown_configured_wast.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn parser(named: WastLanguage, fallback: WastLanguage) -> MarkdownParserConfig {
    MarkdownParserConfig {
        code_languages: Some(MarkdownCodeLanguages::Configured {
            resolve: Arc::new(move |name| {
                (name == "wat").then(|| MarkdownCodeParser::Wast(named.clone()))
            }),
            fallback: Some(MarkdownCodeParser::Wast(fallback)),
        }),
        ..Default::default()
    }
}

fn install(named: WastLanguage, fallback: WastLanguage) -> Extension {
    markdown_with_parser(MarkdownConfig::default(), parser(named, fallback))
}

#[test]
fn detached_editor_body_retains_strict_parser_policy_and_row_identity() {
    use gpui_editor_codemirror::language::StreamSettings;
    let property = NodeProp::new();
    let strict = wast_language()
        .with_node_props(&[property.add(|row| row.is_top().then_some(true))])
        .configure(Some(true), Some("strict child"));
    let parser = MarkdownCodeParser::Wast(strict.clone());
    assert!(parser == MarkdownCodeParser::Wast(strict.clone()));
    assert!(parser != MarkdownCodeParser::Wast(strict.configure(None, None)));
    let mut document = parser
        .parse(
            Text::new("(func $a)\r\n;;字\r(func)"),
            StreamSettings::default(),
        )
        .unwrap();
    let context = document.highlight_context(&(0..1)).unwrap();
    let change = |range, insert| ChangeSet::single(Change::new(range, insert)).unwrap();
    document.apply_changes(&change(7..8, "b")).unwrap();
    assert_eq!(document.highlight_context(&(0..1)), Some(context));
    let accepted = document.text().clone();
    assert!(matches!(
        document.apply_changes(&change(8..9, "")),
        Err(SyntaxUpdateError::StrictSyntax(_))
    ));
    assert_eq!(document.text(), &accepted);
    assert_eq!(document.highlight_context(&(0..1)), Some(context));
}

#[test]
fn reindent_input_reparses_with_the_configured_wast_strategy() {
    use gpui_editor_codemirror::{
        EditorSelection,
        commands::Indentation,
        language::{
            StreamSettings,
            wast::{INDENT, WastIndentStrategy},
        },
    };
    fn typed_close(line: &str) -> bool {
        line.trim_start() == ")"
    }
    let root_data = WastDataFacet::new(Some(
        LanguageData::default().with("indentOnInput", typed_close as fn(&str) -> bool),
    ));
    let strategy: WastIndentStrategy = Arc::new(|_| Some(7));
    let parser = MarkdownCodeParser::Wast(wast_language().with_node_props(&[
        LANGUAGE_DATA.add(move |row| row.is_top().then(|| Some(root_data.clone()))),
        INDENT.add(move |row| row.is("App").then(|| strategy.clone())),
    ]));
    let source = "(func\r    ";
    let document = parser
        .parse(Text::new(source), StreamSettings::default())
        .unwrap();
    let mut state = EditorState::new(source);
    state
        .update(
            Transaction::default()
                .with_selection(EditorSelection::single(source.len(), source.len())),
        )
        .unwrap();
    let input = state
        .replace_selection(")")
        .unwrap()
        .with_user_event("input.type");
    let edit = document
        .reindent_input(&state, input, Indentation::default())
        .unwrap();
    state.update(edit).unwrap();
    assert_eq!(state.text().as_str(), "(func\r       )");
}

#[test]
fn configured_markdown_mounts_match_pinned_data_regions_and_highlights() {
    let stock = wast_language();
    let marker = NodeProp::new();
    let named = stock
        .with_node_props(&[
            marker.add(|row| row.is_top().then_some(true)),
            style_tags([("Number", vec![HighlightTag::new("bool")])]).unwrap(),
        ])
        .configure(None, Some("named"));
    let facet = WastDataFacet::new(Some(LanguageData::default().with("probe", "default")));
    let fallback = stock.with_node_props(&[LANGUAGE_DATA.add({
        let facet = facet.clone();
        move |row| row.is_top().then(|| Some(facet.clone()))
    })]);
    let on_scope = HighlightStyle::define(&[]).with_all(HighlightProperties {
        bold: true,
        ..Default::default()
    });
    let scopes = [
        (
            "named-row",
            on_scope
                .with_wast_node_type_scope(named.node_types().find(|row| row.is_top()).unwrap()),
        ),
        (
            "stock-row",
            on_scope
                .with_wast_node_type_scope(stock.node_types().find(|row| row.is_top()).unwrap()),
        ),
        (
            "stock-facet",
            on_scope.with_wast_facet_scope(stock.data_facet()),
        ),
        ("fallback-facet", on_scope.with_wast_facet_scope(&facet)),
    ];

    let mut rows = rows();
    let configured = rows.next().unwrap();
    let control = rows.next().unwrap();
    let source = configured["source"].as_str().unwrap();
    let slot = Compartment::new();
    let retained = EditorState::with_extensions(
        source,
        Extension::group([
            slot.of(install(named, fallback)),
            stock.data(LanguageData::default().with("probe", "stock")),
            facet.of(LanguageData::default().with("probe", "override")),
        ]),
    )
    .unwrap();
    let retained_tree = markdown_snapshot(&retained).unwrap();
    let mut current = retained.clone();
    current
        .update(
            Transaction::default()
                .with_effects([slot.reconfigure(install(stock.clone(), stock.clone()))]),
        )
        .unwrap();
    assert_eq!(retained.text(), current.text());
    assert!(!Arc::ptr_eq(
        &retained_tree,
        &markdown_snapshot(&current).unwrap()
    ));
    assert!(Arc::ptr_eq(
        &retained_tree,
        &markdown_snapshot(&retained).unwrap()
    ));

    for (state, row) in [(&current, &control), (&retained, &configured)] {
        let snapshot = markdown_snapshot(state).unwrap();
        assert_eq!(snapshot.text().as_str(), row["source"].as_str().unwrap());
        let regions = stock.find_regions(state);
        assert_eq!(snapshot.wast_regions(), regions);
        assert_eq!(
            json!(
                regions
                    .iter()
                    .map(|range| [range.start, range.end])
                    .collect::<Vec<_>>()
            ),
            row["regions"]
        );
        let gap = source.find("> λ").unwrap();
        assert_eq!(regions[0].end, gap);
        assert_eq!(regions[1].start, gap + 2);
        assert!(snapshot.highlight_context(&(gap - 2..gap + 3)).is_none());
        for at in [gap - 1, source.find('😀').unwrap() + 1] {
            assert!(
                snapshot
                    .markdown_language_presence_at(at, Assoc::After)
                    .is_none()
            );
            assert!(!stock.is_active_at(state, at, Assoc::After));
        }
        for expected in row["probes"].as_array().unwrap() {
            let at = serde_json::from_value::<usize>(expected[0].clone()).unwrap();
            for (side, expected) in [(Assoc::Before, &expected[1]), (Assoc::After, &expected[2])] {
                let commonmark = snapshot.markdown_language_presence_at(at, side)
                    == Some(NativeLanguage::Markdown);
                let data = state.language_data_at::<&str>("probe", at, side);
                assert_eq!(
                    json!([stock.is_active_at(state, at, side), commonmark, data]),
                    *expected,
                    "{}: {at} {side:?}",
                    row["step"]
                );
            }
        }
        let spans: Vec<_> = snapshot
            .highlights(0..source.len())
            .unwrap()
            .into_iter()
            .filter_map(|span| {
                let tag = span
                    .tags
                    .iter()
                    .map(HighlightTag::name)
                    .find(|tag| matches!(*tag, "number" | "bool" | "blockComment"))?;
                let context = snapshot.highlight_context(&span.range);
                let mut classes = vec![tag];
                classes.extend(scopes.iter().filter_map(|&(name, style)| {
                    highlighting_for_context(&[Active(style)], span.tags, context, Light)
                        .bold
                        .then_some(name)
                }));
                Some(json!([span.range.start, span.range.end, classes]))
            })
            .collect();
        assert_eq!(json!(spans), row["spans"], "{}", row["step"]);
    }
    let fallback_at = source.find("34").unwrap() + 1;
    assert_eq!(
        retained_tree.language_at(fallback_at, Assoc::After),
        Some(NativeLanguage::Wast)
    );
    let data = retained_tree.data_at(fallback_at, Assoc::After);
    assert_eq!(data[0].get::<&str>("probe"), Some(&"default"));
}

#[test]
fn mounted_sublanguage_replaces_stock_data_without_changing_presence() {
    let stock = wast_language();
    let source = "前 😀\r\n\r\n> ```wat\r\n> (module\r\n> 12)\r\n> ```";
    let body = source.find("(module").unwrap()..source.find(')').unwrap() + 1;
    let facet = WastDataFacet::new(Some(LanguageData::default().with("probe", "root")));
    let rule = WastSublanguage::new(WastSublanguageMode::Replace, facet, move |node, _| {
        node.node_type().is_top() && node.range() == body
    });
    let named = stock
        .with_node_props(&[SUBLANGUAGES.add(move |row| row.is_top().then(|| vec![rule.clone()]))]);
    let state = EditorState::with_extensions(
        source,
        Extension::group([
            install(named, stock.clone()),
            stock.data(LanguageData::default().with("probe", "stock")),
        ]),
    )
    .unwrap();
    let number = source.find("12").unwrap() + 1;
    let gap = source.find("> 12").unwrap() + 1;
    let data = |at| state.language_data_at::<&str>("probe", at, Assoc::After);
    assert_eq!(data(number), ["root"]);
    assert_eq!(data(source.find("module").unwrap()), ["root"]);
    assert!(data(gap).is_empty());
    assert!(stock.is_active_at(&state, number, Assoc::After));
}

#[test]
fn strict_mount_failures_use_outer_bytes_and_do_not_publish_on_error() {
    let stock = wast_language();
    let strict = stock.configure(Some(true), Some("strict"));
    let assert_error = |error, expected| {
        let EditError::Configuration(ConfigurationError::StrictSyntax(
            ParseError::UnexpectedToken { offset, .. },
        )) = error
        else {
            panic!("expected strict syntax error, got {error:?}");
        };
        assert_eq!(offset, expected);
    };
    for row in rows().skip(2) {
        let source = row["source"].as_str().unwrap();
        let expected = serde_json::from_value::<usize>(row["errorAt"].clone()).unwrap();
        let extension = if row["step"] == "strict fallback" {
            install(stock.clone(), strict.clone())
        } else {
            // A failure after named selection must not try the permissive fallback.
            install(strict.clone(), stock.clone())
        };
        assert_error(
            EditError::Configuration(
                EditorState::with_extensions(source, extension.clone())
                    .err()
                    .unwrap(),
            ),
            expected,
        );
        let slot = Compartment::new();
        let mut state =
            EditorState::with_extensions(source, slot.of(install(stock.clone(), stock.clone())))
                .unwrap();
        let retained = markdown_snapshot(&state).unwrap();
        assert!(!retained.wast_regions().is_empty());
        assert_error(
            state
                .update(Transaction::default().with_effects([slot.reconfigure(extension)]))
                .err()
                .unwrap(),
            expected,
        );
        assert!(Arc::ptr_eq(&retained, &markdown_snapshot(&state).unwrap()));
        assert_eq!(state.text().as_str(), source);
    }

    let source = "前 😀\r\n\r\n> ```wat\r\n> ;; 😀\r\n> (func)\r\n> ```";
    let at = source.find("(func)").unwrap();
    let changes = ChangeSet::single(Change::new(at..at + 6, ")")).unwrap();
    let mut state = EditorState::with_extensions(source, install(strict.clone(), strict)).unwrap();
    let retained = markdown_snapshot(&state).unwrap();
    assert_error(
        state
            .update(Transaction::new(changes.clone()))
            .err()
            .unwrap(),
        at,
    );
    assert_eq!(state.text().as_str(), source);
    assert!(Arc::ptr_eq(&retained, &markdown_snapshot(&state).unwrap()));
    drop(state);
    let mut standalone = Arc::try_unwrap(retained).ok().unwrap();
    assert!(
        matches!(standalone.apply_changes(&changes), Err(SyntaxUpdateError::StrictSyntax(ParseError::UnexpectedToken { offset, .. })) if offset == at)
    );
    assert_eq!(standalone.text().as_str(), source);
}
