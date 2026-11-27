use std::sync::{Arc, Mutex};

use gpui_editor_codemirror::{
    Assoc, EditError, EditorSelection, EditorState, Extension, READ_ONLY, SelectionRange, Text,
    Transaction,
    commands::{
        CommentMode, CommentStyle, CommentTokens, comment, comment_with_language_data,
        comment_with_language_data_in_lines,
    },
    language::{LanguageData, language_data_provider},
    setup::basic_setup,
    view::keymap::KeyPlatform,
};
use serde_json::{Value, json};

fn marked(spec: &str) -> (String, EditorSelection) {
    let mut doc = String::new();
    let mut offsets = Vec::new();
    for ch in spec.chars() {
        if ch == '|' {
            offsets.push(doc.len());
        } else {
            doc.push(ch);
        }
    }
    let selection = match offsets.as_slice() {
        [] => EditorSelection::single(0, 0),
        [at] => EditorSelection::single(*at, *at),
        _ => EditorSelection::new(
            offsets
                .chunks_exact(2)
                .map(|group| SelectionRange::new(group[0], group[1]))
                .collect(),
            0,
        )
        .unwrap(),
    };
    (doc, selection)
}

fn state(spec: &str) -> EditorState {
    let (doc, selection) = marked(spec);
    let mut state = EditorState::new(doc);
    state
        .update(Transaction::default().with_selection(selection))
        .unwrap();
    state
}

fn check(from: &str, to: &str, style: CommentStyle, mode: CommentMode, tokens: CommentTokens<'_>) {
    let mut state = state(from);
    let transaction = comment(&state, mode, style, |_| tokens).unwrap();
    if from == to {
        assert!(transaction.is_none());
    } else {
        state.update(transaction.unwrap()).unwrap();
    }
    let (expected, selection) = marked(to);
    assert_eq!(state.text().as_str(), expected, "{from:?}");
    assert_eq!(state.selection(), &selection, "{from:?}");
}

#[test]
fn pinned_line_comments_align_indentation_and_preserve_selection() {
    let tokens = CommentTokens {
        line: Some("//"),
        block: None,
    };
    for (from, to) in [
        ("line 1\n  |line 2\nline 3", "line 1\n  // |line 2\nline 3"),
        (
            "  //lin|e 1\n  //  line 2\n  // line |3",
            "  lin|e 1\n   line 2\n  line |3",
        ),
        (
            "  //lin|e 1\n  //  line 2\n   line 3\n  // li|ne 4",
            "  // //lin|e 1\n  // //  line 2\n  //  line 3\n  // // li|ne 4",
        ),
        (
            " |字\r\n\r\n\t\t次|\r\nelse",
            " // |字\r\n\r\n\t// \t次|\r\nelse",
        ),
        ("line| 1\nline 2\n|line 3", "// line| 1\n// line 2\n|line 3"),
        ("|line| |with| |ranges|", "// |line| |with| |ranges|"),
        ("|\nline 2", "// |\nline 2"),
        ("|\n    | \n", "|\n    | \n"),
    ] {
        check(from, to, CommentStyle::Line, CommentMode::Toggle, tokens);
    }
}

#[test]
fn pinned_block_comment_toggles_exact_ranges_and_whole_lines() {
    let tokens = CommentTokens {
        block: Some(("/*", "*/")),
        line: None,
    };
    for (from, to, style) in [
        (
            "  lin|e 1\n  line 2\n  line |3",
            "  lin/* |e 1\n  line 2\n  line | */3",
            CommentStyle::Block,
        ),
        (
            "|/* one\ntwo */| three",
            "|one\ntwo| three",
            CommentStyle::Block,
        ),
        (
            "  /* |one\r\ntwo| */ three",
            "  |one\r\ntwo| three",
            CommentStyle::Block,
        ),
        (
            "on|e\nt|w|o\nth|ree",
            "/* on|e\nt|w|o\nth|ree */",
            CommentStyle::BlockByLine,
        ),
        ("|one\n|two", "/* |one */\n|two", CommentStyle::BlockByLine),
        ("|one\ntwo", "|/* one */\ntwo", CommentStyle::BlockByLine),
    ] {
        check(from, to, style, CommentMode::Toggle, tokens);
    }
}

#[test]
fn comment_policy_can_choose_language_per_range_and_respects_read_only() {
    let mut state = state("fi|rst|\nsec|ond|");
    let tx = comment(
        &state,
        CommentMode::Comment,
        CommentStyle::PreferLine,
        |at| CommentTokens {
            line: Some(if at == 0 { "#" } else { "//" }),
            block: Some(("<!--", "-->")),
        },
    )
    .unwrap()
    .unwrap();
    state.update(tx).unwrap();
    assert_eq!(state.text().as_str(), "# first\n// second");
    let absent = comment(&state, CommentMode::Toggle, CommentStyle::Block, |_| {
        CommentTokens::default()
    });
    assert!(absent.unwrap().is_none());
    let locked = EditorState::with_extensions("x", READ_ONLY.of(true)).unwrap();
    assert!(
        comment(&locked, CommentMode::Toggle, CommentStyle::Line, |_| {
            CommentTokens {
                line: Some("#"),
                block: None,
            }
        })
        .unwrap()
        .is_none()
    );
}

#[test]
fn pinned_comments_use_logical_lines_without_rewriting_original_breaks() {
    for row in include_str!("fixtures/projected_comments.jsonl").lines() {
        let row: Value = serde_json::from_str(row).unwrap();
        let name = row["name"].as_str().unwrap();
        let source = row["source"].as_str().unwrap();
        let line = if row["line"].is_null() {
            None
        } else {
            Some("//")
        };
        let tokens = CommentTokens {
            line,
            block: Some(("/*", "*/")),
        };
        let input = serde_json::from_value::<Vec<[usize; 2]>>(row["input"].clone()).unwrap();
        let selection = EditorSelection::new(
            input
                .into_iter()
                .map(|[a, b]| SelectionRange::new(a, b))
                .collect(),
            0,
        )
        .unwrap();
        let mode = match row["command"].as_str().unwrap() {
            "toggleComment" => (CommentMode::Toggle, CommentStyle::PreferLine),
            "lineUncomment" => (CommentMode::Uncomment, CommentStyle::Line),
            "toggleBlockCommentByLine" => (CommentMode::Toggle, CommentStyle::BlockByLine),
            other => panic!("unknown comment command: {other}"),
        };
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut state = EditorState::with_extensions(
            source,
            Extension::group([
                READ_ONLY.of(row["readOnly"].as_bool().unwrap()),
                language_data_provider({
                    let calls = calls.clone();
                    move |_, pos, side| {
                        assert_eq!(side, Assoc::After);
                        calls.lock().unwrap().push([pos, 1]);
                        vec![Arc::new(
                            LanguageData::default().with("commentTokens", tokens),
                        )]
                    }
                }),
            ]),
        )
        .unwrap();
        state
            .update(Transaction::default().with_selection(selection.clone()))
            .unwrap();
        let mut setup = basic_setup::<()>(state, KeyPlatform::Linux).unwrap();
        let view = setup.view_mut();
        let change = comment_with_language_data(view.state(), mode.0, mode.1).unwrap();
        assert_eq!(json!(*calls.lock().unwrap()), row["calls"], "{name}");
        if let Some(edit) = change {
            view.update(edit, (), |_| (None, ())).unwrap();
            assert_eq!(
                view.state().text().as_str(),
                row["output"].as_str().unwrap(),
                "{name}"
            );
            let ranges: Vec<_> = view
                .state()
                .selection()
                .ranges()
                .iter()
                .map(|r| [r.anchor(), r.head()])
                .collect();
            assert_eq!(json!(ranges), row["select"], "{name}");
            assert_eq!(view.undo_depth(), 1, "{name}");
            view.undo().unwrap();
        } else {
            assert!(row["output"].is_null(), "{name}");
        }
        assert_eq!(view.state().text().as_str(), source, "{name}");
        assert_eq!(view.state().selection(), &selection, "{name}");
    }
}

#[test]
fn projected_comments_validate_the_line_view_and_preserve_breaks() {
    let mut state = EditorState::with_extensions(
        "p\r// \nq",
        language_data_provider(|_, _, _| {
            vec![Arc::new(LanguageData::default().with(
                "commentTokens",
                CommentTokens {
                    line: Some("//"),
                    block: None,
                },
            ))]
        }),
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(EditorSelection::single(2, 2)))
        .unwrap();
    let uncomment = |lines: &Text| {
        comment_with_language_data_in_lines(
            &state,
            CommentMode::Uncomment,
            CommentStyle::Line,
            lines,
        )
    };
    assert!(uncomment(&Text::new("p\n// \nq")).unwrap().is_none());
    assert!(matches!(
        uncomment(&Text::new("p\n#  \nq")),
        Err(EditError::InvalidRange)
    ));
    assert_eq!(state.text().as_str(), "p\r// \nq");

    let mut split = EditorState::with_extensions(
        "p\rx\r\ny",
        language_data_provider(|_, _, _| {
            vec![Arc::new(LanguageData::default().with(
                "commentTokens",
                CommentTokens {
                    line: None,
                    block: Some(("/*", "*/")),
                },
            ))]
        }),
    )
    .unwrap();
    split
        .update(Transaction::default().with_selection(EditorSelection::single(4, 4)))
        .unwrap();
    assert!(
        comment_with_language_data_in_lines(
            &split,
            CommentMode::Comment,
            CommentStyle::Block,
            &Text::new("p\nx\r\ny"),
        )
        .unwrap()
        .is_none()
    );
}

#[test]
fn explicit_comment_and_uncomment_do_not_toggle_the_other_way() {
    let tokens = CommentTokens {
        line: Some("#"),
        block: None,
    };
    check(
        "# |title",
        "# |title",
        CommentStyle::Line,
        CommentMode::Comment,
        tokens,
    );
    check(
        "# |title",
        "|title",
        CommentStyle::Line,
        CommentMode::Uncomment,
        tokens,
    );
    check(
        "|title",
        "|title",
        CommentStyle::Line,
        CommentMode::Uncomment,
        tokens,
    );
    let both = CommentTokens {
        line: Some("//"),
        block: Some(("/*", "*/")),
    };
    check(
        "|x",
        "// |x",
        CommentStyle::PreferLine,
        CommentMode::Toggle,
        both,
    );
    let only_block = CommentTokens {
        block: Some(("<!--", "-->")),
        line: None,
    };
    check(
        "x|",
        "<!-- x| -->",
        CommentStyle::PreferLine,
        CommentMode::Toggle,
        only_block,
    );
}
