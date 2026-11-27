//! Replay CodeMirror transactions captured by `fixtures/wast_oracle.mjs`.

use std::{num::NonZeroUsize, ops::Range, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, EditorState, TAB_SIZE, Text, Transaction,
    commands::CommentTokens,
    language::{
        INDENT_UNIT, IndentOptions, IndentStyle,
        fold::foldable,
        get_indentation,
        wast::{WastDocument, WastNodeKind},
    },
    language_data::{
        CloseBracketData, LANGUAGES, LanguageDescription, NativeLanguage, NativeSyntax,
        SupportedDocument,
    },
};
use serde_json::{Value, json};

fn records() -> impl Iterator<Item = Value> {
    include_str!("fixtures/wast_oracle.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
}

fn number(value: &Value) -> usize {
    value.as_u64().unwrap().try_into().unwrap()
}

fn factory() -> gpui_editor_codemirror::language_data::NativeLanguageSupport {
    let entry = LanguageDescription::match_filename(LANGUAGES, "script.wast").unwrap();
    let wat = LanguageDescription::match_filename(LANGUAGES, "module.wat").unwrap();
    assert_eq!(entry.name, wat.name);
    let support = *entry.native_support_load().load().unwrap();
    assert_eq!(support, NativeLanguage::Wast.support());
    support
}

fn wast(doc: &SupportedDocument) -> &WastDocument {
    let NativeSyntax::Wast(wast) = doc.syntax() else {
        panic!("expected WAST syntax");
    };
    wast
}

fn signature(rows: Vec<Value>) -> Value {
    let hash = serde_json::to_vec(&rows)
        .unwrap()
        .into_iter()
        .fold(2_166_136_261_u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        });
    json!([rows.len(), hash])
}

fn line_ranges(source: &str) -> impl Iterator<Item = Range<usize>> {
    let mut start = 0;
    source
        .match_indices(['\r', '\n'])
        .filter(|(at, eol)| *eol == "\r" || !source[..*at].ends_with('\r'))
        .map(move |(at, _)| {
            let line = start..at;
            start = at
                + if source[at..].starts_with("\r\n") {
                    2
                } else {
                    1
                };
            line
        })
        .chain(std::iter::once_with(move || {
            // Include the final line, even after a trailing line break.
            let start = source.rfind(['\r', '\n']).map_or(0, |at| at + 1);
            start..source.len()
        }))
}

fn snapshot(state: &EditorState, doc: &SupportedDocument) -> Value {
    assert_eq!(state.text(), doc.text());
    let mut ancestors = Vec::new();
    let mut nodes = Vec::new();
    let mut errors = 0;
    for node in wast(doc).nodes() {
        while ancestors.last() == Some(&0) {
            ancestors.pop();
        }
        let range = node.range();
        nodes.push(json!([
            ancestors.len(),
            node.kind().name(),
            range.start,
            range.end
        ]));
        errors += usize::from(node.kind() == WastNodeKind::Error);
        if let Some(left) = ancestors.last_mut() {
            *left -= 1;
        }
        ancestors.push(node.children().len());
    }
    let source = state.text().as_str();
    let highlights = doc
        .highlights(0..source.len())
        .unwrap()
        .into_iter()
        .map(|span| {
            let [tag] = span.tags else { panic!("{span:?}") };
            assert!(tag.modifiers().is_empty());
            json!([span.range.start, span.range.end, tag.name()])
        })
        .collect();
    let indents = source
        .char_indices()
        .map(|(at, _)| at)
        .chain([source.len()])
        .filter(|&at| !(source[..at].ends_with('\r') && source[at..].starts_with('\n')))
        .map(|at| {
            let indent = |options| get_indentation(state, at, options, Some(doc.syntax())).unwrap();
            let simulated = IndentOptions {
                simulate_break: Some(at),
                ..Default::default()
            };
            let double = IndentOptions {
                simulate_double_break: true,
                ..simulated
            };
            let override_first = |line| (line == 0).then_some(7);
            let overridden = IndentOptions {
                override_indentation: Some(&override_first),
                ..Default::default()
            };
            json!([
                at,
                indent(IndentOptions::default()),
                indent(simulated),
                indent(double),
                indent(overridden)
            ])
        })
        .collect();
    let folds = line_ranges(source)
        .map(|line| {
            let fold = foldable(state, line.start, line.end, |_, start, end| {
                doc.syntax().foldable(start, end)
            });
            json!([
                line.start,
                line.end,
                fold.map(|range| [range.start, range.end])
            ])
        })
        .collect();
    json!({"errors": errors, "nodes": signature(nodes), "highlights": signature(highlights),
           "indents": signature(indents), "folds": signature(folds)})
}

#[test]
fn edited_wast_snapshots_match_pinned_editor_transactions() {
    let support = factory();
    let mut state = EditorState::new("");
    let mut doc = support.parse(Text::new("")).unwrap();
    let mut previous_errors = 0;
    for row in records().filter(|row| row.get("expected").is_some()) {
        let name = row["name"].as_str().unwrap();
        let expected_errors = number(&row["expected"]["errors"]);
        if let Some(source) = row["source"].as_str() {
            let settings = [
                TAB_SIZE.of(NonZeroUsize::new(8).unwrap()),
                INDENT_UNIT.of(IndentStyle::parse("    ").unwrap()),
            ];
            state = EditorState::with_extensions(
                source,
                gpui_editor_codemirror::Extension::group(settings),
            )
            .unwrap();
            doc = support.parse(state.text().clone()).unwrap();
        } else {
            let changes = ChangeSet::new(
                row["changes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|edit| {
                        Change::new(
                            number(&edit[0])..number(&edit[1]),
                            edit[2].as_str().unwrap(),
                        )
                    })
                    .collect(),
            )
            .unwrap();
            doc.apply_changes(&changes).unwrap();
            state.update(Transaction::new(changes)).unwrap();
            let wast = wast(&doc);
            if expected_errors > 0 {
                assert_eq!(doc.reuse_stats(), Default::default(), "{name}");
                assert_eq!(wast.reused_suffix_actions(), 0, "{name}");
                if name == "repair comment and remove trailing close" {
                    // This step still needs recovery. Unlike upstream, native
                    // recovery does not share any published tree records.
                    assert_eq!(number(&row["shared"]), 17);
                }
            } else if previous_errors > 0 {
                let stats = doc.reuse_stats();
                assert!(stats.nodes_reused > 0, "{name}");
                assert_eq!(wast.reused_suffix_actions(), 0, "{name}");
                if name == "repair late EOF and change data" {
                    // This is the first strict tree after the 17-object step.
                    assert_eq!((stats.resumed_at, stats.nodes_reused), (2_979, 635));
                }
            } else {
                assert!(doc.reuse_stats().nodes_reused > 100, "{name}");
                assert!(wast.reused_suffix_actions() > 100, "{name}");
            }
        }
        let updated = snapshot(&state, &doc);
        let fresh = support.parse(state.text().clone()).unwrap();
        assert_eq!(updated, snapshot(&state, &fresh), "{name}: fresh parse");
        for key in ["errors", "nodes", "highlights", "indents", "folds"] {
            assert_eq!(updated[key], row["expected"][key], "{name}: {key}");
        }
        previous_errors = expected_errors;
    }
}

#[test]
fn catalog_support_installs_pinned_language_data_for_each_snapshot() {
    let row = records()
        .find(|row| row["name"] == "support installation")
        .unwrap();
    let source = row["source"].as_str().unwrap();
    let support = factory();
    assert_eq!(support.completion_source_capacity(), 0);
    let doc = Arc::new(support.parse(Text::new(source)).unwrap());
    let language = Compartment::new();
    let mut state =
        EditorState::with_extensions(source, language.of(doc.language_data_extension())).unwrap();
    check_data(&state, &doc, &row["initial"]);

    let insertion = ChangeSet::single(Change::new(0..0, "\n")).unwrap();
    state.update(Transaction::new(insertion)).unwrap();
    // Unlike LanguageSupport in the pinned editor, the native provider is
    // scoped to one snapshot. The host must refresh it after a text edit.
    assert!(
        state
            .language_data_at::<CommentTokens>("commentTokens", 1, Assoc::After)
            .is_empty()
    );
    assert!(
        state
            .language_data_at::<CloseBracketData>("closeBrackets", 1, Assoc::After)
            .is_empty()
    );
    let updated = Arc::new(support.parse(state.text().clone()).unwrap());
    let refresh = language.reconfigure(updated.language_data_extension());
    state
        .update(Transaction::default().with_effects([refresh]))
        .unwrap();
    check_data(&state, &updated, &row["edited"]);
}

fn check_data(state: &EditorState, doc: &SupportedDocument, expected: &Value) {
    let source = state.text().as_str();
    let wast = wast(doc);
    assert_eq!(expected["regions"], json!([[0, wast.root().range().end]]));
    let props: Vec<_> = wast
        .nodes()
        .filter_map(|node| {
            let kind = node.kind();
            let isolate = kind.isolates_direction().then_some("auto");
            let matching = kind.matching_delimiter().map(|kind| [kind.name()]);
            let opened_by = matching.filter(|_| kind == WastNodeKind::CloseParen);
            let closed_by = matching.filter(|_| kind == WastNodeKind::OpenParen);
            let group = kind.is_expression().then_some(["Expression"]);
            let range = node.range();
            (isolate.is_some() || matching.is_some() || group.is_some()).then(|| {
                json!([
                    kind.name(),
                    range.start,
                    range.end,
                    isolate,
                    opened_by,
                    closed_by,
                    group
                ])
            })
        })
        .collect();
    assert_eq!(json!(props), expected["props"]);
    let app = wast
        .root()
        .children()
        .find(|node| node.kind() == WastNodeKind::App)
        .unwrap();
    let (open, close) = (
        app.children().next().unwrap(),
        app.children().next_back().unwrap(),
    );
    assert_eq!(open.kind().matching_delimiter(), Some(close.kind()));
    assert_eq!(
        json!([
            [open.range().start, open.range().end],
            [close.range().start, close.range().end]
        ]),
        expected["bracket"]
    );
    for at in [0, source.find("export").unwrap(), source.len()] {
        for side in [Assoc::Before, Assoc::After] {
            assert_eq!(doc.language_at(at, side), Some(NativeLanguage::Wast));
            let comments = state.language_data_at::<CommentTokens>("commentTokens", at, side);
            let [comments] = comments.as_slice() else {
                panic!("expected WAST comments")
            };
            let (open, close) = comments.block.unwrap();
            let brackets = state.language_data_at::<CloseBracketData>("closeBrackets", at, side);
            let [brackets] = brackets.as_slice() else {
                panic!("expected WAST brackets")
            };
            assert!(brackets.string_prefixes.is_empty());
            assert_eq!(
                json!([[{"line": comments.line, "block": {"open": open, "close": close}}],
                       [{"brackets": brackets.brackets}]]),
                json!([expected["data"][0], expected["data"][1]])
            );
            assert_eq!(
                doc.completions(at, false).len(),
                expected["data"][2].as_array().unwrap().len()
            );
            assert_eq!(
                state
                    .language_data_at::<fn(&str) -> bool>("indentOnInput", at, side)
                    .len(),
                expected["data"][3].as_array().unwrap().len()
            );
            assert_eq!(
                json!(state.language_data_at::<&str>("wordChars", at, side)),
                expected["data"][4]
            );
        }
    }
}
