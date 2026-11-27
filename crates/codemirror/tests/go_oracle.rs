//! Replay the pinned Go package against native state and services. The Node
//! generator verifies the archives and resolved code before it imports them.

use std::{num::NonZeroUsize, ops::Range};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, EditorState, Extension, TAB_SIZE, Transaction,
    commands::CommentTokens,
    language::{
        INDENT_UNIT, IndentOptions, IndentStyle, TagModifier, fold::foldable_in_state,
        get_indentation, go::GoNodeKind,
    },
    language_data::{
        CloseBracketData, GoCompletionSource, NativeLanguage, NativeSyntax,
        attached_language_snapshot, go_extension,
    },
};
use serde_json::{Value, json};

#[test]
fn go_state_and_services_across_edits() {
    let open = |source: &str| {
        EditorState::with_extensions(
            source,
            Extension::group([
                go_extension(),
                TAB_SIZE.of(NonZeroUsize::new(8).unwrap()),
                INDENT_UNIT.of(IndentStyle::parse("    ").unwrap()),
            ]),
        )
        .unwrap()
    };
    let mut state = open("");
    let mut failures = Vec::new();
    for line in include_str!("fixtures/go_oracle.jsonl").lines() {
        let step: Value = serde_json::from_str(line).unwrap();
        let name = step["name"].as_str().unwrap();
        if let Some(source) = step["source"].as_str() {
            state = open(source);
        } else {
            let edits: Vec<(usize, usize, String)> =
                serde_json::from_value(step["changes"].clone()).unwrap();
            let changes = ChangeSet::new(
                edits
                    .into_iter()
                    .map(|(from, to, insert)| Change::new(from..to, insert))
                    .collect(),
            )
            .unwrap();
            state.update(Transaction::new(changes)).unwrap();
        }
        let next = snapshot(&state);
        let fresh = open(state.text().as_str());
        assert_eq!(next, snapshot(&fresh), "fresh: {name}");
        let keys = next
            .as_object()
            .unwrap()
            .keys()
            .filter(|&key| next[key] != step["expected"][key])
            .cloned()
            .collect::<Vec<_>>();
        if !keys.is_empty() {
            failures.push(format!("{name}: {}", keys.join(", ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn signature(rows: &[Value]) -> Value {
    let hash = serde_json::to_vec(rows)
        .unwrap()
        .into_iter()
        .fold(2_166_136_261u32, |hash, byte| {
            (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
        });
    json!([rows.len(), hash])
}

fn metadata(state: &EditorState, at: usize, side: Assoc) -> Value {
    let comments = state
        .language_data_at::<CommentTokens>("commentTokens", at, side)
        .into_iter()
        .map(|m| (m.line, m.block.unwrap().0, m.block.unwrap().1))
        .collect::<Vec<_>>();
    let brackets = state
        .language_data_at::<CloseBracketData>("closeBrackets", at, side)
        .into_iter()
        .map(|m| m.brackets)
        .collect::<Vec<_>>();
    let words = state.language_data_at::<&str>("wordChars", at, side);
    let handlers = state.language_data_at::<fn(&str) -> bool>("indentOnInput", at, side);
    assert_eq!(handlers.len(), 1);
    let prefixes = [
        "case",
        "\tdefault",
        "}",
        "\u{feff}}",
        "case ",
        "case:",
        "caseΩ",
        "}\n",
        "\u{85}}",
    ];
    json!([
        comments,
        brackets,
        words,
        prefixes.map(|prefix| handlers[0](prefix))
    ])
}

fn snapshot(state: &EditorState) -> Value {
    let doc = attached_language_snapshot(state).unwrap();
    assert_eq!(doc.text(), state.text());
    let syntax = doc.syntax();
    let NativeSyntax::Go(go) = syntax else {
        unreachable!()
    };
    let raw = state.text().as_str();
    let mut errors = 0;
    let nodes = go
        .nodes()
        .map(|node| {
            errors += usize::from(node.kind() == GoNodeKind::Error);
            let range = node.range();
            assert!(raw.get(range.clone()).is_some());
            let mut depth = 0;
            let mut current = node;
            while let Some(parent) = current.parent() {
                let bounds = parent.range();
                assert!(bounds.start <= range.start && range.end <= bounds.end);
                depth += 1;
                current = parent;
            }
            json!([depth, node.kind().name(), range.start, range.end])
        })
        .collect::<Vec<_>>();
    assert_eq!(go.is_recovered(), errors != 0);
    let highlights = go
        .highlights(0..raw.len())
        .unwrap()
        .map(|span| {
            let tags = span
                .tags
                .iter()
                .map(|tag| {
                    let mut name = tag.name().to_owned();
                    for modifier in tag.modifiers() {
                        name.push_str(match modifier {
                            TagModifier::Definition => ".definition",
                            TagModifier::Function => ".function",
                            _ => unreachable!(),
                        });
                    }
                    name
                })
                .collect::<Vec<_>>()
                .join(" ");
            json!([span.range.start, span.range.end, tags])
        })
        .collect::<Vec<_>>();
    let (mut indents, mut simulated, mut fixed, mut local) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for at in raw.char_indices().map(|(at, _)| at).chain([raw.len()]) {
        if at > 0 && raw.as_bytes().get(at - 1..at + 1) == Some(b"\r\n") {
            continue;
        }
        indents.push(json!([
            at,
            get_indentation(state, at, IndentOptions::default(), None).unwrap()
        ]));
        let simulate = |double| {
            get_indentation(
                state,
                at,
                IndentOptions {
                    simulate_break: Some(at),
                    simulate_double_break: double,
                    ..IndentOptions::default()
                },
                None,
            )
            .unwrap()
        };
        simulated.push(json!([at, simulate(false), simulate(true)]));
        let sources =
            state.language_data_at::<GoCompletionSource>("autocomplete", at, Assoc::Before);
        assert_eq!(sources.len(), 2);
        for explicit in [false, true] {
            let (mut f, mut l) = (None, None);
            for (slot, source) in sources.iter().enumerate() {
                let Some(result) = source(state, at, explicit) else {
                    continue;
                };
                assert!(result.from <= at && result.to.is_none());
                match slot {
                    0 => f = Some(result.from),
                    1 => {
                        l = Some((
                            result.from,
                            result
                                .options
                                .iter()
                                .map(|o| format!("{}:{}", o.kind.as_deref().unwrap(), o.label))
                                .collect::<Vec<_>>(),
                        ))
                    }
                    _ => unreachable!(),
                }
            }
            fixed.push(json!([at, explicit, f]));
            local.push(json!([at, explicit, l]));
        }
    }
    let folds = lines(raw)
        .map(|Range { start, end }| {
            json!([
                start,
                end,
                foldable_in_state(state, start, end).map(|r| [r.start, r.end])
            ])
        })
        .collect::<Vec<_>>();
    let data = [0, raw.len()]
        .into_iter()
        .flat_map(|at| {
            [Assoc::Before, Assoc::After].map(|side| {
                assert_eq!(doc.language_at(at, side), Some(NativeLanguage::Go));
                metadata(state, at, side)
            })
        })
        .collect::<Vec<_>>();
    json!({"text": signature(&[json!(raw)]), "errors": errors,
        "nodes": signature(&nodes), "highlights": signature(&highlights),
        "indents": signature(&indents), "simulated": signature(&simulated), "folds": signature(&folds),
        "fixed": signature(&fixed), "local": signature(&local), "data": signature(&data)})
}

fn lines(raw: &str) -> impl Iterator<Item = Range<usize>> + '_ {
    let mut start = 0;
    raw.match_indices(['\r', '\n'])
        .filter(move |&(at, ch)| ch != "\n" || at == 0 || raw.as_bytes()[at - 1] != b'\r')
        .map(|(at, _)| at)
        .chain([raw.len()])
        .map(move |end| {
            let range = start..end;
            start = end
                + if raw.as_bytes().get(end..end + 2) == Some(b"\r\n") {
                    2
                } else {
                    1
                };
            range
        })
}
