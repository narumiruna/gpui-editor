//! Replay the SHA-512-pinned YAML wrapper with a configured JSON body.

use std::{ptr, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Text, Transaction,
    commands::CommentTokens,
    language::{
        HighlightTag, LanguageData,
        lezer::{LANGUAGE_DATA, LanguageFacet, NodeProp, ParseError, style_tags},
        yaml::frontmatter_language::{YamlFrontmatterDocument, YamlFrontmatterLanguage},
    },
    language_data::{NativeSyntax, SyntaxUpdateError, json_language},
};
use serde_json::{Value, json};

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn check_document(doc: &YamlFrontmatterDocument, key: &NodeProp<&'static str>, row: &Value) {
    let label = &row["label"];
    let body = doc.body_range();
    assert_eq!(doc.text().as_str(), row["source"], "{label}");
    assert_eq!(json!([body.start, body.end]), row["body"], "{label}");
    assert_eq!(
        doc.frontmatter().is_some(),
        !row["yamlProbe"].is_null(),
        "{label}"
    );
    let NativeSyntax::Json(syntax) = doc.body() else {
        panic!("expected JSON body: {label}");
    };
    assert_eq!(
        syntax.text().as_str(),
        doc.text().slice(body.clone()).unwrap()
    );
    let nodes: Vec<_> = syntax
        .nodes()
        .map(|node| {
            let ty = node.node_type();
            let range = node.range();
            json!([
                ty.name(),
                body.start + range.start,
                body.start + range.end,
                ty.prop(key)
            ])
        })
        .collect();
    assert_eq!(json!(nodes), row["nodes"], "{label}");
    let spans: Vec<_> = doc
        .highlights(body)
        .unwrap()
        .flat_map(|span| {
            span.tags
                .iter()
                .map(move |tag| (span.range.clone(), tag.name()))
        })
        .filter(|(_, tag)| matches!(*tag, "atom" | "keyword"))
        .map(|(range, tag)| json!([range.start, range.end, tag]))
        .collect();
    assert_eq!(json!(spans), row["highlights"], "{label}");
}

#[test]
fn configured_json_matches_pinned_wrapper_through_edits_and_reconfiguration() {
    let key = NodeProp::new();
    let facet = LanguageFacet::new(Some(LanguageData::default().with("probe", "base")));
    let languages = [("strict", "atom"), ("recover", "keyword")].map(|(mode, tag)| {
        let root_data = facet.clone();
        let parser = json_language()
            .configure(Some(mode == "strict"), Some(mode))
            .with_node_props(&[
                key.add(move |ty| (ty.name() == "Array").then_some(mode)),
                LANGUAGE_DATA.add(move |ty| ty.is_top().then(|| Some(root_data.clone()))),
                style_tags([("PropertyName", vec![HighlightTag::new(tag)])]).unwrap(),
            ]);
        YamlFrontmatterLanguage::with_json(parser)
    });
    let fields = ["strict", "recover"].map(|mode| {
        languages[usize::from(mode == "recover")]
            .state_field(facet.of(LanguageData::default().with("probe", mode)))
    });
    let active =
        |state: &EditorState, mode: usize| state.context().field(&fields[mode]).unwrap().unwrap();
    let slot = Compartment::new();
    let data = Compartment::new();
    let lines = include_str!("fixtures/yaml_frontmatter_configured.jsonl").lines();
    assert_eq!(lines.clone().count(), 18);
    let mut rows = lines.map(|line| serde_json::from_str::<Value>(line).unwrap());
    let start = rows.next().unwrap();
    let input = start["source"].as_str().unwrap();
    let mut mode = 0;
    let mut state = EditorState::with_extensions(
        input,
        Extension::group([
            data.of(facet.of(LanguageData::default().with("probe", "override"))),
            slot.of(fields[mode].extension()),
        ]),
    )
    .unwrap();
    let mut direct = languages[mode].parse(Text::new(input)).unwrap();
    for row in std::iter::once(start).chain(rows) {
        let label = &row["label"];
        let previous = active(&state, mode);
        let next_mode = row["mode"]
            .as_str()
            .map_or(mode, |mode| usize::from(mode == "recover"));
        let changes = row["edit"]
            .as_array()
            .map_or_else(ChangeSet::default, |edit| {
                ChangeSet::single(Change::new(
                    byte(&edit[0])..byte(&edit[1]),
                    edit[2].as_str().unwrap(),
                ))
                .unwrap()
            });
        let mut transaction = Transaction::new(changes.clone());
        if row["selection"].is_number() {
            let at = byte(&row["selection"]);
            transaction = transaction.with_selection(EditorSelection::single(at, at));
        }
        if next_mode != mode {
            transaction =
                transaction.with_effects([slot.reconfigure(fields[next_mode].extension())]);
        } else if row["removeData"] == true {
            transaction = transaction.with_effects([data.reconfigure(Extension::default())]);
        }
        if row["rejected"] == true {
            let attempted = Text::new(row["attempted"].as_str().unwrap());
            let Some(SyntaxUpdateError::StrictSyntax(
                error @ ParseError::UnexpectedToken { offset, .. },
            )) = languages[next_mode].parse(attempted).err()
            else {
                panic!("expected a strict JSON error: {label}");
            };
            assert_eq!(offset, byte(&row["stop"]), "{label}");
            assert_eq!(
                EditorState::with_extensions(
                    row["attempted"].as_str().unwrap(),
                    fields[next_mode].extension(),
                )
                .err(),
                Some(ConfigurationError::StrictSyntax(error)),
                "state creation: {label}"
            );
            assert_eq!(
                state.update(transaction),
                Err(EditError::Configuration(ConfigurationError::StrictSyntax(
                    error
                ))),
                "{label}"
            );
            assert!(Arc::ptr_eq(&previous, &active(&state, mode)), "{label}");
            if next_mode == mode {
                assert_eq!(
                    direct.apply_changes(&changes),
                    Err(SyntaxUpdateError::StrictSyntax(error)),
                    "{label}"
                );
            }
            assert_eq!(direct.text(), state.text(), "{label}");
            continue;
        }
        if label != "install" {
            state
                .update(transaction)
                .unwrap_or_else(|error| panic!("{label}: {error:?}"));
        }
        let current = active(&state, next_mode);
        let doc = current.as_ref().as_ref().unwrap();
        if next_mode == mode {
            direct.apply_changes(&changes).unwrap();
            if changes.is_empty() {
                assert!(Arc::ptr_eq(&previous, &current), "{label}");
            }
        } else {
            direct = languages[next_mode].parse(state.text().clone()).unwrap();
            assert!(
                state.context().field(&fields[mode]).unwrap().is_none(),
                "{label}"
            );
        }
        if label == "front edit shifts unchanged body" {
            let old = previous.as_ref().as_ref().unwrap();
            assert!(ptr::eq(old.body(), doc.body()));
            assert_ne!(old.body_range().start, doc.body_range().start);
        }
        check_document(doc, &key, &row);
        check_document(&direct, &key, &row);
        if !row["yamlProbe"].is_null() {
            let tokens = state.language_data_at::<CommentTokens>(
                "commentTokens",
                byte(&row["yamlProbe"]),
                Assoc::Before,
            );
            assert_eq!(tokens[0].line, Some("#"), "{label}");
        }
        for site in row["metadata"].as_array().unwrap() {
            let at = byte(&site[0]);
            for (side, expected) in [(Assoc::Before, &site[1]), (Assoc::After, &site[2])] {
                let actual = state.language_data_at::<&str>("probe", at, side);
                assert_eq!(json!(actual), *expected, "{label}: {at} {side:?}");
            }
        }
        mode = next_mode;
    }
}
