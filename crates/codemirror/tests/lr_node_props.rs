use std::{ptr, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorState, Extension, Transaction,
    language::{
        LanguageData,
        lezer::{
            CLOSED_BY, GROUP, ISOLATE, Isolation, LANGUAGE_DATA, LanguageFacet, NodeProp,
            NodePropSource, NodeType, OPENED_BY,
        },
        markdown::{
            MarkdownCodeLanguages, MarkdownCodeParser, MarkdownConfig, MarkdownParserConfig,
            markdown_snapshot, markdown_with_parser,
        },
    },
    language_data::{
        NativeSyntax, SupportedDocument, attached_language_snapshot, json_language, lezer_language,
    },
};
use serde_json::{Value, json};

macro_rules! with_lr {
    ($parser:expr, $handle:ident => $action:expr) => {
        match $parser {
            MarkdownCodeParser::Json($handle) => $action,
            MarkdownCodeParser::Lezer($handle) => $action,
            _ => unreachable!("expected a configured LR handle"),
        }
    };
}

#[path = "lr_node_props/bidi.rs"]
mod bidi;
#[path = "lr_node_props/brackets.rs"]
mod brackets;
#[path = "lr_node_props/folds.rs"]
mod folds;
#[path = "lr_node_props/highlight_scopes.rs"]
mod highlight_scopes;
#[path = "lr_node_props/style_tags.rs"]
mod style_tags;

fn rows(kind: &str) -> impl Iterator<Item = Value> {
    include_str!("fixtures/lr_node_props.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(move |row| row["kind"] == kind)
}

fn handles(row: &Value) -> [MarkdownCodeParser; 2] {
    let json = MarkdownCodeParser::Json(json_language().clone());
    let lezer = MarkdownCodeParser::Lezer(lezer_language().clone());
    match row["name"].as_str().unwrap() {
        "json" => [json, lezer],
        "lezer" => [lezer, json],
        _ => unreachable!(),
    }
}

#[test]
fn stock_lr_rows_match_the_pinned_grammar_terms_and_properties() {
    for row in rows("rows") {
        let [stock, _] = handles(&row);
        let types = with_lr!(&stock, handle => handle.node_types().map(|ty| {
            json!([ty.term(), ty.name(), ty.is_top(), ty.is_skipped(), ty.is_error(),
                ty.is_anonymous(), ty.prop(&GROUP), ty.prop(&OPENED_BY), ty.prop(&CLOSED_BY),
                ty.prop(&ISOLATE).copied().flatten().map(|direction| match direction {
                    Isolation::Auto => "auto", Isolation::Ltr => "ltr", Isolation::Rtl => "rtl",
                })])
        }).collect::<Vec<_>>());
        assert_eq!(json!(types), row["types"], "{}", row["name"]);
    }
}

fn document_nodes(doc: &SupportedDocument) -> Vec<(&NodeType, std::ops::Range<usize>)> {
    match doc.syntax() {
        NativeSyntax::Json(doc) => doc
            .nodes()
            .map(|node| (node.node_type(), node.range()))
            .collect(),
        NativeSyntax::LezerGrammar(doc) => doc
            .nodes()
            .map(|node| (node.node_type().unwrap(), node.range()))
            .collect(),
        _ => unreachable!(),
    }
}

fn with_props(handle: &MarkdownCodeParser, sources: &[NodePropSource]) -> MarkdownCodeParser {
    match handle {
        MarkdownCodeParser::Json(handle) => {
            MarkdownCodeParser::Json(handle.with_node_props(sources))
        }
        MarkdownCodeParser::Lezer(handle) => {
            MarkdownCodeParser::Lezer(handle.with_node_props(sources))
        }
        _ => unreachable!(),
    }
}

fn property_row(
    ty: &NodeType,
    range: std::ops::Range<usize>,
    key: &NodeProp<String>,
    seen: &NodeProp<String>,
) -> Value {
    json!([
        ty.term(),
        range.start,
        range.end,
        ty.prop(key),
        ty.prop(seen)
    ])
}

#[test]
fn raw_and_editor_views_share_one_property_table_and_keep_it_on_edits() {
    for row in rows("values") {
        let [stock, _] = handles(&row);
        let source = row["source"].as_str().unwrap();
        let target = if row["target"] == "Array" {
            "Array"
        } else {
            "Body"
        };
        let key = NodeProp::<String>::with_combine(|old, next| format!("{old}+{next}"));
        let seen = NodeProp::<String>::new();
        let contribution = |text| key.add(move |ty| ty.is(target).then(|| String::from(text)));
        let prior = with_props(&stock, &[contribution("seed")]);
        let sources = [
            contribution("first"),
            contribution("second"),
            seen.add({
                let key = key.clone();
                move |ty| ty.prop(&key).cloned()
            }),
        ];
        let parser = with_props(&prior, &sources);
        let stock_rows = with_lr!(&stock, handle => handle.node_types().collect::<Vec<_>>());
        let types = with_lr!(&parser, handle => handle.node_types().collect::<Vec<_>>());
        for (ty, stock) in types.iter().zip(stock_rows) {
            assert_eq!(ptr::eq(*ty, stock), !ty.is(target));
        }
        let raw = with_lr!(&parser, handle => handle.parse(source).unwrap());
        let mut pending = vec![raw.root()];
        let mut raw_rows = Vec::new();
        while let Some(node) = pending.pop() {
            let ty = node.node_type().unwrap();
            assert!(ptr::eq(ty, types[usize::from(node.term())]));
            raw_rows.push(property_row(ty, node.range(), &key, &seen));
            pending.extend(node.children().rev());
        }
        assert_eq!(json!(raw_rows), row["raw"]);
        for strict in [false, true] {
            let extension = with_lr!(&parser, handle => handle.configure(Some(strict), Some("renamed")).extension());
            let mut state = EditorState::with_extensions(source, extension).unwrap();
            let original = attached_language_snapshot(&state).unwrap();
            let inspect = |doc: &SupportedDocument| {
                document_nodes(doc)
                    .into_iter()
                    .map(|(ty, range)| {
                        assert!(ptr::eq(ty, types[usize::from(ty.term())]));
                        property_row(ty, range, &key, &seen)
                    })
                    .collect::<Vec<_>>()
            };
            let at = source.find('名').unwrap();
            let change = ChangeSet::single(Change::new(at..at + '名'.len_utf8(), "鏡")).unwrap();
            state.update(Transaction::new(change)).unwrap();
            let edited = attached_language_snapshot(&state).unwrap();
            assert_eq!(json!(edited.text().as_str()), row["edited"]);
            assert_eq!(json!(inspect(&edited)), row["after"]);
            assert_eq!(json!(inspect(&original)), row["editor"]);
        }
    }
}

#[test]
fn root_facet_controls_data_and_mount_presence_without_changing_the_handle() {
    for row in rows("facet") {
        let [stock, other] = handles(&row);
        let original_facet = with_lr!(&stock, handle => handle.data_facet());
        let other_facet = with_lr!(&other, handle => handle.data_facet());
        let alternate = LanguageFacet::new(Some(LanguageData::default().with("probe", "default")));
        let root = match row["caseName"].as_str().unwrap() {
            "replacement" => Some(alternate.clone()),
            "cross" => Some(other_facet.clone()),
            "absent" => None,
            _ => unreachable!(),
        };
        let sources = [LANGUAGE_DATA.add({
            let root = root.clone();
            move |ty| ty.is_top().then(|| root.clone())
        })];
        let selected = with_props(&stock, &sources);
        assert!(with_lr!(&selected, handle => handle.data_facet()) == original_facet);
        let source = row["source"].as_str().unwrap();
        let raw = with_lr!(&selected, handle => handle.parse(source).unwrap());
        assert!(raw.root().prop(&LANGUAGE_DATA) == Some(&root));
        let unchanged = with_lr!(&stock, handle => handle.parse(source).unwrap());
        assert!(
            unchanged
                .root()
                .prop(&LANGUAGE_DATA)
                .and_then(Option::as_ref)
                == Some(original_facet)
        );
        let slot = Compartment::new();
        let data = || {
            Extension::group([
                slot.of(alternate.of(LanguageData::default().with("probe", "override"))),
                original_facet.of(LanguageData::default().with("probe", "stock")),
                other_facet.of(LanguageData::default().with("probe", "other")),
            ])
        };
        let make_mount = || {
            markdown_with_parser(
                MarkdownConfig::default(),
                MarkdownParserConfig {
                    code_languages: Some(MarkdownCodeLanguages::Configured {
                        resolve: Arc::new({
                            let selected = selected.clone();
                            move |label| (label == "key").then(|| selected.clone())
                        }),
                        fallback: None,
                    }),
                    ..Default::default()
                },
            )
        };
        let query =
            |state: &EditorState,
             section: &str,
             snapshot: fn(&EditorState) -> Option<Arc<SupportedDocument>>| {
                let expected = &row[section];
                let at = usize::try_from(expected["at"].as_u64().unwrap()).unwrap();
                let describe = |state: &EditorState, pos, side| {
                    state.language_data_at::<&str>("probe", pos, side)
                };
                let original = snapshot(state).unwrap();
                let defaults: Vec<_> = original
                    .data_at(at, Assoc::After)
                    .iter()
                    .filter_map(|entry| entry.get::<&str>("probe").copied())
                    .collect();
                let wanted = if row["caseName"] == "replacement" {
                    &["default"][..]
                } else {
                    &[]
                };
                assert_eq!(defaults, wanted);
                let mut next = state.clone();
                next.update(
                    Transaction::default().with_effects([slot.reconfigure(Extension::default())]),
                )
                .unwrap();
                assert!(Arc::ptr_eq(&original, &snapshot(&next).unwrap()));
                for side in [Assoc::Before, Assoc::After] {
                    assert_eq!(json!(describe(state, at, side)), expected["probe"]);
                    assert_eq!(json!(describe(&next, at, side)), expected["removed"]);
                }
                let active = |parser: &MarkdownCodeParser| {
                    [Assoc::Before, Assoc::After].map(
                        |side| with_lr!(parser, handle => handle.is_active_at(state, at, side)),
                    )
                };
                let region = |parser: &MarkdownCodeParser| with_lr!(parser, handle => handle.find_regions(state).into_iter().map(|range| [range.start, range.end]).collect::<Vec<_>>());
                assert_eq!(json!([active(&stock), active(&other)]), expected["active"]);
                assert_eq!(json!([region(&stock), region(&other)]), expected["regions"]);
            };
        let whole = EditorState::with_extensions(
            row["source"].as_str().unwrap(),
            Extension::group([with_lr!(&selected, handle => handle.extension()), data()]),
        )
        .unwrap();
        query(&whole, "whole", attached_language_snapshot);
        let mount = EditorState::with_extensions(
            row["mountedSource"].as_str().unwrap(),
            Extension::group([make_mount(), data()]),
        )
        .unwrap();
        query(&mount, "mount", markdown_snapshot);
    }
}
