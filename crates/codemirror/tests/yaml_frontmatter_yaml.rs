//! Replay the pinned YAML wrapper with stock front matter and a configured body.

use std::{ptr, sync::Arc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, Compartment, ConfigurationError, EditError, EditorSelection,
    EditorState, Extension, Precedence, Text, Transaction,
    commands::CommentTokens,
    language::{
        IndentOptions, LanguageData,
        fold::foldable_in_state,
        get_indentation,
        lezer::{ParseError, SyntaxNode},
        yaml::{
            YamlNode,
            frontmatter_language::{YamlFrontmatterDocument, YamlFrontmatterLanguage},
        },
    },
    language_data::{
        NativeSyntax, SyntaxUpdateError, YamlLanguage, attached_language_snapshot, json_language,
        yaml_language,
    },
};
use serde_json::{Value, json};

fn byte(value: &Value) -> usize {
    usize::try_from(value.as_u64().unwrap()).unwrap()
}

fn yaml_nodes(node: &YamlNode, origin: usize, depth: usize, rows: &mut Vec<Value>) {
    let range = node.range();
    rows.push(json!([
        depth,
        node.kind().name(),
        origin + range.start,
        origin + range.end
    ]));
    for child in node.children() {
        yaml_nodes(child, origin, depth + 1, rows);
    }
}

fn outer_errors(node: SyntaxNode<'_>, errors: &mut Vec<Value>) {
    if node.is_error() {
        let range = node.range();
        errors.push(json!([range.start, range.end]));
    }
    for child in node.children() {
        outer_errors(child, errors);
    }
}

fn check_document(doc: &YamlFrontmatterDocument, row: &Value) {
    let label = &row["label"];
    let body = doc.body_range();
    assert_eq!(doc.text().as_str(), row["source"], "{label}");
    assert_eq!(json!([body.start, body.end]), row["body"], "{label}");
    assert_eq!(
        doc.body().text().as_str(),
        doc.text().slice(body.clone()).unwrap()
    );
    let mut front_rows = Vec::new();
    if let Some(front) = doc.frontmatter() {
        yaml_nodes(front.yaml().root(), 0, 0, &mut front_rows);
        assert_eq!(json!(front_rows), row["frontNodes"], "{label}");
    } else {
        assert!(row["frontNodes"].is_null(), "{label}");
    }
    let mut body_rows = Vec::new();
    if let NativeSyntax::Yaml(syntax) = doc.body() {
        yaml_nodes(syntax.root(), body.start, 0, &mut body_rows);
        assert_eq!(json!(body_rows), row["bodyNodes"], "{label}");
    } else {
        assert!(matches!(doc.body(), NativeSyntax::Json(_)));
        assert!(row["bodyNodes"].is_null(), "{label}");
    }
    let mount_errors = |nodes: Vec<Value>| {
        nodes
            .into_iter()
            .filter(|node| node[1] == "⚠")
            .map(|node| json!([node[2], node[3]]))
    };
    let mut errors = mount_errors(front_rows).collect();
    outer_errors(doc.wrapper_tree().unwrap().root(), &mut errors);
    errors.extend(mount_errors(body_rows));
    assert_eq!(json!(errors), row["errors"], "{label}");
}

fn check_state(state: &EditorState, strict: &YamlLanguage, row: &Value) {
    let label = &row["label"];
    assert_eq!(state.text().as_str(), row["source"], "{label}");
    assert!(attached_language_snapshot(state).is_none(), "{label}");
    assert!(YamlLanguage::active_name(state).is_none(), "{label}");
    for handle in [yaml_language(), strict] {
        let regions: Vec<_> = handle
            .find_regions(state)
            .into_iter()
            .map(|range| [range.start, range.end])
            .collect();
        assert_eq!(json!(regions), row["regions"], "{label}");
    }
    for group in row["data"].as_array().unwrap() {
        for at in group[1].as_array().unwrap().iter().map(byte) {
            for (index, side) in [Assoc::Before, Assoc::After].into_iter().enumerate() {
                let active = yaml_language().is_active_at(state, at, side);
                assert_eq!(active, strict.is_active_at(state, at, side), "{label}");
                let comments: Vec<_> = state
                    .language_data_at::<CommentTokens>("commentTokens", at, side)
                    .into_iter()
                    .map(|tokens| tokens.line)
                    .collect();
                let rules: Vec<_> = state
                    .language_data_at::<fn(&str) -> bool>("indentOnInput", at, side)
                    .into_iter()
                    .map(|rule| ["  ]", "}", "] junk"].map(rule))
                    .collect();
                let probe = state.language_data_at::<&str>("probe", at, side);
                assert_eq!(
                    json!([active, probe, comments, rules]),
                    group[0][index],
                    "{label}: {at} {side:?}"
                );
            }
        }
    }
    let source = state.text().as_str();
    for at in (0..source.len()).filter(|&at| {
        !source.is_char_boundary(at) || (at > 0 && source.as_bytes()[at - 1..=at] == *b"\r\n")
    }) {
        for side in [Assoc::Before, Assoc::After] {
            assert!(!strict.is_active_at(state, at, side), "{label}: {at}");
            assert!(
                state
                    .language_data_at::<CommentTokens>("commentTokens", at, side)
                    .is_empty(),
                "{label}: {at}"
            );
        }
    }
    for line in row["lines"].as_array().unwrap() {
        let (from, to) = (byte(&line[0]), byte(&line[1]));
        let indent = get_indentation(state, from, IndentOptions::default(), None).unwrap();
        assert_eq!(json!(indent), line[2], "{label}: {from}");
        let fold = foldable_in_state(state, from, to).map(|range| [range.start, range.end]);
        assert_eq!(json!(fold), line[3], "{label}: {from}");
    }
}

#[test]
fn configured_yaml_wrapper_matches_pinned_mounts_and_checked_edits() {
    let stock = yaml_language();
    let strict = stock.configure(Some(true), Some("strict body"));
    let languages = [
        YamlFrontmatterLanguage::with_yaml(strict.clone()),
        YamlFrontmatterLanguage::with_yaml(stock.clone()),
        YamlFrontmatterLanguage::with_json(json_language().configure(Some(true), None)),
    ];
    let data = |probe| stock.data(LanguageData::default().with("probe", probe));
    let fields = [
        languages[0].state_field(data("shared")),
        languages[1].state_field(Extension::default()),
        languages[2].state_field(data("shared")),
    ];
    let slot = Compartment::new();
    let parent = Compartment::new();
    let snapshot =
        |state: &EditorState, mode: usize| state.context().field(&fields[mode]).unwrap().unwrap();
    let rows: Vec<Value> = include_str!("fixtures/yaml_frontmatter_yaml.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(rows.len(), 15);
    let mut mode = 0;
    let mut state = EditorState::new("");
    let mut direct = languages[0].parse(Text::default()).unwrap();
    let mut expected = &rows[0];
    for row in &rows {
        let label = row["label"].as_str().unwrap();
        let next_mode = match row["mode"].as_str() {
            Some("strict") => 0,
            Some("recover") => 1,
            Some("json") => 2,
            None => mode,
            Some(other) => panic!("unknown mode: {other}"),
        };
        if row["start"] == true {
            state = EditorState::with_extensions(
                row["source"].as_str().unwrap(),
                Extension::group([
                    slot.of(fields[next_mode].extension()),
                    parent.of(Extension::default()),
                ]),
            )
            .unwrap();
            direct = languages[next_mode].parse(state.text().clone()).unwrap();
        } else {
            let previous = snapshot(&state, mode);
            let old = state.clone();
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
            if next_mode != mode {
                transaction =
                    transaction.with_effects([slot.reconfigure(fields[next_mode].extension())]);
            } else if row["override"] == true {
                transaction = transaction.with_effects([
                    parent.reconfigure(data("parent").with_precedence(Precedence::High))
                ]);
            }
            if row["selection"].is_number() {
                let at = byte(&row["selection"]);
                transaction = transaction.with_selection(EditorSelection::single(at, at));
            }
            if let Some(attempted) = row["attempted"].as_str() {
                let Some(SyntaxUpdateError::StrictSyntax(
                    error @ ParseError::UnexpectedToken { offset, .. },
                )) = languages[next_mode].parse(Text::new(attempted)).err()
                else {
                    panic!("expected a strict YAML error: {label}");
                };
                assert_eq!(offset, byte(&row["stop"]), "{label}");
                assert_eq!(
                    EditorState::with_extensions(attempted, fields[next_mode].extension()).err(),
                    Some(ConfigurationError::StrictSyntax(error)),
                    "{label}"
                );
                assert_eq!(
                    state.update(transaction),
                    Err(EditError::Configuration(ConfigurationError::StrictSyntax(
                        error
                    ))),
                    "{label}"
                );
                assert!(Arc::ptr_eq(&previous, &snapshot(&state, mode)), "{label}");
                assert_eq!(state.selection(), old.selection(), "{label}");
                check_state(&state, &strict, expected);
                if next_mode == mode {
                    assert_eq!(
                        direct.apply_changes(&changes),
                        Err(SyntaxUpdateError::StrictSyntax(error)),
                        "{label}"
                    );
                    check_document(&direct, expected);
                }
                continue;
            }
            state
                .update(transaction)
                .unwrap_or_else(|error| panic!("{label}: {error:?}"));
            let current = snapshot(&state, next_mode);
            if changes.is_empty() && next_mode == mode {
                assert!(Arc::ptr_eq(&previous, &current), "{label}");
            }
            if label == "front edit shifts body" {
                let before = previous.as_ref().as_ref().unwrap();
                let after = current.as_ref().as_ref().unwrap();
                assert_ne!(before.body_range().start, after.body_range().start);
                assert!(ptr::eq(before.body(), after.body()));
            }
            if next_mode == mode {
                direct.apply_changes(&changes).unwrap();
            } else {
                assert!(
                    state.context().field(&fields[mode]).unwrap().is_none(),
                    "{label}"
                );
                direct = languages[next_mode].parse(state.text().clone()).unwrap();
            }
            check_state(&old, &strict, expected);
            check_document(previous.as_ref().as_ref().unwrap(), expected);
        }
        mode = next_mode;
        expected = row;
        check_document(snapshot(&state, mode).as_ref().as_ref().unwrap(), row);
        check_document(&direct, row);
        check_state(&state, &strict, row);
    }
    state
        .update(Transaction::default().with_effects([slot.reconfigure(Extension::default())]))
        .unwrap();
    assert!(stock.find_regions(&state).is_empty());
    assert!(state.context().field(&fields[mode]).unwrap().is_none());
}

#[test]
fn strict_yaml_body_keeps_native_editor_limits() {
    let strict = yaml_language().configure(Some(true), None);
    let language = YamlFrontmatterLanguage::with_yaml(strict);
    let front = "---\r\nname: 😀\r---\r\n";
    let source = Text::new(format!("{front}#{}", "x".repeat(1 << 20)));
    assert_eq!(
        language.parse(source).err(),
        Some(SyntaxUpdateError::StrictSyntax(ParseError::LimitExceeded {
            offset: front.len()
        })),
    );
}
