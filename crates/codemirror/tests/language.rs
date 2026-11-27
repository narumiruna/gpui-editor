//! StreamLanguage fixtures from @codemirror/language 6.13.1 with
//! @codemirror/state 6.7.6 and @lezer/highlight 1.2.5.
//!
//! Compare styled nodes via `ensureSyntaxTree` and `getStyleTags`, and
//! indentation via `getIndentation`. Here positions are converted from
//! normalized UTF-16 document offsets to original UTF-8 byte offsets.

use std::{borrow::Cow, cell::RefCell, num::NonZeroUsize, ops::Range, rc::Rc};

use gpui_editor_codemirror::{
    Assoc, Change, ChangeSet, EditError, EditorState, Text, Transaction,
    language::{
        HighlightTag, IndentOptions, LanguageData, StreamDocument, StreamLanguage, StreamSettings,
        StreamUpdateError, TagModifier, TokenTable,
    },
    stream_parser::{StreamParser, StringStream, TokenizeError},
};

const SOURCE: &str = "{\r\n\t/*é\r\n\r\n β*/ tail\r\n\t}\r\n";
const SETTINGS: StreamSettings = StreamSettings {
    tab_size: NonZeroUsize::new(8).unwrap(),
    indent_unit: 3,
};

struct Mode;

#[derive(Clone)]
struct ModeState {
    unit: usize,
    depth: usize,
    blank: usize,
    last_indent: usize,
    comment: bool,
}

impl StreamParser for Mode {
    type State = ModeState;

    fn name(&self) -> Option<&str> {
        Some("fixtures")
    }

    fn start_state(&self, unit: usize) -> ModeState {
        ModeState {
            unit,
            depth: 0,
            blank: 0,
            last_indent: 0,
            comment: false,
        }
    }

    fn blank_line(&self, state: &mut ModeState, unit: usize) {
        assert_eq!(state.unit, unit);
        state.blank += 1;
    }

    fn token(&self, stream: &mut StringStream<'_>, state: &mut ModeState) -> Option<Cow<'_, str>> {
        assert_eq!(state.unit, stream.indent_unit());
        if stream.sol() {
            state.last_indent = stream.indentation();
        }
        if state.comment || stream.match_str("/*", true, false) {
            state.comment = !(stream.skip_to("*") && stream.match_str("*/", true, false));
            if state.comment {
                stream.skip_to_end();
            }
            return Some("comment".into());
        }
        if stream.eat_space() {
            return None;
        }
        let style = if stream.eat('{').is_some() {
            state.depth += 1;
            "brace"
        } else if stream.eat('}').is_some() {
            state.depth = state.depth.saturating_sub(1);
            "brace"
        } else if stream.eat('~').is_some() {
            "notKnown"
        } else {
            if !stream.eat_while(|ch| !ch.is_whitespace() && !"{}/*~".contains(ch)) {
                stream.next();
            }
            "variable"
        };
        Some(style.into())
    }

    fn indent(&self, state: &ModeState, after: &str, unit: usize) -> Option<usize> {
        assert_eq!(state.unit, unit);
        let depth = state
            .depth
            .saturating_sub(usize::from(after.starts_with('}')));
        Some(depth * unit + 100 * state.blank + state.last_indent)
    }
}

type Styled<'a> = (Range<usize>, &'a str, Vec<HighlightTag>);

fn styled<'a, M: StreamParser>(
    doc: &'a StreamDocument<'_, M>,
    range: Range<usize>,
) -> Vec<Styled<'a>> {
    doc.tokens(range)
        .unwrap()
        .map(|span| {
            let text = doc.text().slice(span.range.clone()).unwrap();
            (span.range, text, span.tags.to_vec())
        })
        .collect()
}

fn span<'a>(range: Range<usize>, text: &'a str, tag: &str) -> Styled<'a> {
    (range, text, vec![HighlightTag::new(tag)])
}

#[test]
fn parses_multiline_state_and_queries_bounded_byte_ranges() {
    let language = StreamLanguage::new(Mode).with_language_data(
        LanguageData::default()
            .with("wordChars", "-")
            .with("commentTokens", ("/*", "*/")),
    );
    let doc = language.parse(Text::new(SOURCE), SETTINGS).unwrap();
    assert_eq!(language.name(), "fixtures");
    assert_eq!(
        styled(&doc, 0..SOURCE.len()),
        [
            span(0..1, "{", "brace"),
            span(4..8, "/*é", "comment"),
            span(12..17, " β*/", "comment"),
            span(18..22, "tail", "variableName"),
            span(25..26, "}", "brace"),
        ]
    );
    assert_eq!(
        styled(&doc, 8..19),
        [
            span(12..17, " β*/", "comment"),
            span(18..19, "t", "variableName")
        ]
    );
    assert_eq!(styled(&doc, 6..8), [span(6..8, "é", "comment")]);
    assert!(styled(&doc, 16..16).is_empty());
    assert!(styled(&doc, 8..12).is_empty());
    assert!(doc.tokens(7..8).is_none());
    assert!(doc.tokens(0..SOURCE.len() + 1).is_none());
    assert!(doc.tokens(Range { start: 4, end: 3 }).is_none());

    for position in [0, 8, 9, 10, 13, SOURCE.len()] {
        let data = doc.language_data_at(position).unwrap();
        assert_eq!(data.get::<&str>("wordChars"), Some(&"-"));
        assert_eq!(
            data.get::<(&str, &str)>("commentTokens"),
            Some(&("/*", "*/"))
        );
        assert_eq!(data.get::<bool>("unconfigured"), None);
        assert_eq!(data.get::<bool>("wordChars"), None);
    }
    assert!(doc.language_data_at(7).is_none());
}

#[test]
fn indentation_uses_the_mode_state_and_replays_overrides() {
    let language = StreamLanguage::new(Mode);
    let doc = language.parse(Text::new(SOURCE), SETTINGS).unwrap();
    for (position, expected) in [
        (0, 0),
        (3, 3),
        (4, 11),
        (8, 11),
        (10, 11),
        (12, 111),
        (13, 104),
        (22, 104),
        (24, 101),
        (28, 108),
    ] {
        assert_eq!(
            doc.indentation(position, IndentOptions::default()),
            Ok(Some(expected)),
            "{position}"
        );
    }
    assert_eq!(doc.indentation(7, IndentOptions::default()), Ok(None));
    let options = IndentOptions {
        override_indentation: Some(&|start| match start {
            3 => Some(0),
            12 => Some(5),
            _ => None,
        }),
        ..IndentOptions::default()
    };
    for (position, expected) in [(4, 3), (8, 3), (12, 103), (13, 108), (22, 108)] {
        assert_eq!(
            doc.indentation(position, options),
            Ok(Some(expected)),
            "{position}"
        );
    }
    assert_eq!(doc.indentation(22, IndentOptions::default()), Ok(Some(104)));
}

#[test]
fn simulated_break_passes_the_correct_line_and_context() {
    let language = StreamLanguage::new(Mode).with_indent(|mode, state, after, cx| {
        let position = cx.position();
        if cx.simulated_break() == Some(position) {
            let left = cx.line_at(position, Assoc::Before).unwrap();
            let right = cx.line_at(position, Assoc::After).unwrap();
            if right.text.is_empty() {
                assert_eq!((left.text, after), ("", ""));
            } else {
                assert_eq!(
                    (left.text, right.text, after),
                    ("{", "  }   next", "}   next")
                );
                assert_eq!(
                    cx.text_after_pos(position, Assoc::After),
                    Some("  }   next")
                );
                assert_eq!(cx.line_indent(position, Assoc::After), Some(2));
                assert_eq!(cx.column(position, Assoc::Before), Some(1));
                assert_eq!(cx.column(position, Assoc::After), Some(0));
            }
        } else {
            assert_eq!(after, "{  }   next");
        }
        mode.indent(state, after, cx.unit())
    });
    let doc = language
        .parse(Text::new("{  }   next\n"), StreamSettings::default())
        .unwrap();
    assert_eq!(doc.indentation(1, IndentOptions::default()), Ok(Some(2)));
    let options = IndentOptions {
        simulate_break: Some(1),
        ..IndentOptions::default()
    };
    assert_eq!(doc.indentation(1, options), Ok(Some(0)));
    assert_eq!(
        doc.indentation(
            1,
            IndentOptions {
                simulate_double_break: true,
                ..options
            }
        ),
        Ok(Some(2))
    );
}

struct StyledChars(bool);

impl StreamParser for StyledChars {
    type State = ();

    fn start_state(&self, _: usize) {}

    fn token(&self, stream: &mut StringStream<'_>, _: &mut ()) -> Option<Cow<'_, str>> {
        Some(
            match stream.next().unwrap() {
                '0' | 'b' | 'c' => "variable",
                '1' => "variableName",
                '2' => "custom.constant.special.constant",
                '3' => "definition.keyword",
                '4' => "keyword.string",
                '5' => "nope",
                '6' => "",
                '7' => "keyword unknown",
                '8' => "unknown.keyword",
                '9' => "keyword  string",
                'a' => "variable.special",
                _ => return None,
            }
            .into(),
        )
    }

    fn merge_tokens(&self) -> bool {
        self.0
    }
}

#[test]
fn resolves_custom_tags_and_modifiers_and_merges_only_equal_node_types() {
    use TagModifier::{Constant, Special};
    let table = TokenTable::from([
        ("variable".into(), vec![HighlightTag::new("bool")]),
        ("variableName".into(), vec![HighlightTag::new("number")]),
        (
            "custom".into(),
            vec![HighlightTag::new("string"), HighlightTag::new("number")],
        ),
    ]);
    let language = StreamLanguage::new(StyledChars(true)).with_token_table(table);
    let doc = language
        .parse(Text::new("0123456789abc"), StreamSettings::default())
        .unwrap();
    assert_eq!(
        styled(&doc, 0..13),
        [
            span(0..1, "0", "variableName"),
            span(1..2, "1", "number"),
            (
                2..3,
                "2",
                ["string", "number"]
                    .map(|name| HighlightTag::new(name)
                        .with_modifier(Constant)
                        .with_modifier(Special))
                    .to_vec()
            ),
            span(3..4, "3", "keyword"),
            span(4..5, "4", "keyword"),
            span(7..8, "7", "keyword"),
            span(8..9, "8", "keyword"),
            (
                9..10,
                "9",
                vec![HighlightTag::new("keyword"), HighlightTag::new("string")]
            ),
            (
                10..11,
                "a",
                vec![HighlightTag::new("bool").with_modifier(Special)]
            ),
            span(11..13, "bc", "variableName"),
        ]
    );

    let language = StreamLanguage::new(StyledChars(true));
    let doc = language
        .parse(Text::new("01bc"), StreamSettings::default())
        .unwrap();
    assert_eq!(styled(&doc, 0..4), [span(0..4, "01bc", "variableName")]);
    let language = StreamLanguage::new(StyledChars(false));
    let doc = language
        .parse(Text::new("01bc"), StreamSettings::default())
        .unwrap();
    assert_eq!(
        styled(&doc, 0..4),
        [
            span(0..1, "0", "variableName"),
            span(1..2, "1", "variableName"),
            span(2..3, "b", "variableName"),
            span(3..4, "c", "variableName")
        ]
    );
}

#[test]
fn display_wrap_ranges_do_not_change_logical_lines() {
    let language = StreamLanguage::new(Mode);
    let source = format!("/*{}\r\nend*/", "界".repeat(768));
    let doc = language.parse(Text::new(&source), SETTINGS).unwrap();
    let wrap_start = 2 + 509 * 3;
    let first_end = 2 + 768 * 3;
    assert_eq!(
        styled(&doc, wrap_start..first_end + 4),
        [
            span(
                wrap_start..first_end,
                &source[wrap_start..first_end],
                "comment"
            ),
            span(first_end + 2..first_end + 4, "en", "comment")
        ]
    );
    assert!(doc.tokens(wrap_start + 1..first_end).is_none());
}

#[test]
fn blank_line_hook_runs_for_physical_blank_lines_but_not_at_eof() {
    let language = StreamLanguage::new(Mode);
    for (source, expected) in [
        ("", 0),
        ("\n", 100),
        ("\r\n\n", 200),
        ("x\n", 0),
        (" \n", 1),
    ] {
        let doc = language.parse(Text::new(source), SETTINGS).unwrap();
        assert_eq!(
            doc.indentation(source.len(), IndentOptions::default()),
            Ok(Some(expected)),
            "{source:?}"
        );
    }
}

#[test]
fn edits_match_a_fresh_parse_and_the_editor_document() {
    let language = StreamLanguage::new(Mode);
    let mut doc = language.parse(Text::new(SOURCE), SETTINGS).unwrap();
    let mut editor = EditorState::new(SOURCE);
    for (changes, expected) in [
        (
            vec![Change::new(4..6, ""), Change::new(18..22, "λ")],
            vec![
                span(0..1, "{", "brace"),
                span(4..6, "é", "variableName"),
                span(11..15, "β*/", "variableName"),
                span(16..18, "λ", "variableName"),
                span(21..22, "}", "brace"),
            ],
        ),
        // Insert a comment opener and change CRLF to LF on the same line.
        (
            vec![Change::new(4..4, "/*"), Change::new(6..7, "")],
            vec![
                span(0..1, "{", "brace"),
                span(4..8, "/*é", "comment"),
                span(11..16, " β*/", "comment"),
                span(17..19, "λ", "variableName"),
                span(22..23, "}", "brace"),
            ],
        ),
        // Delete the separators to join three logical lines.
        (
            vec![Change::new(8..11, "")],
            vec![
                span(0..1, "{", "brace"),
                span(4..13, "/*é β*/", "comment"),
                span(14..16, "λ", "variableName"),
                span(19..20, "}", "brace"),
            ],
        ),
        (
            vec![Change::new(0..0, "\r\n")],
            vec![
                span(2..3, "{", "brace"),
                span(6..15, "/*é β*/", "comment"),
                span(16..18, "λ", "variableName"),
                span(21..22, "}", "brace"),
            ],
        ),
    ] {
        let transaction = Transaction::new(ChangeSet::new(changes).unwrap());
        doc.apply_changes(transaction.changes()).unwrap();
        editor.update(transaction).unwrap();
        assert_eq!(doc.text(), editor.text());
        let source = editor.text().as_str();
        assert_eq!(styled(&doc, 0..source.len()), expected);
        let fresh = language.parse(Text::new(source), SETTINGS).unwrap();
        assert_eq!(
            styled(&doc, 0..source.len()),
            styled(&fresh, 0..source.len())
        );
        for position in 0..=source.len() {
            assert_eq!(
                doc.indentation(position, IndentOptions::default()),
                fresh.indentation(position, IndentOptions::default()),
                "{source:?} at {position}"
            );
        }
    }
    // Deleting down to an empty document and then appending at EOF is safe.
    let changes = ChangeSet::single(Change::new(0..doc.text().len(), "")).unwrap();
    doc.apply_changes(&changes).unwrap();
    doc.apply_changes(&ChangeSet::single(Change::new(0..0, "x\r\n")).unwrap())
        .unwrap();
    assert_eq!(styled(&doc, 0..3), [span(0..1, "x", "variableName")]);
}

struct Fallible;

impl StreamParser for Fallible {
    type State = Rc<RefCell<usize>>;

    fn start_state(&self, _: usize) -> Self::State {
        Rc::default()
    }

    fn copy_state(&self, state: &Self::State) -> Self::State {
        Rc::new(RefCell::new(*state.borrow()))
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        state: &mut Self::State,
    ) -> Option<Cow<'_, str>> {
        *state.borrow_mut() += 1;
        if stream.peek() == Some('!') {
            return None;
        }
        stream.skip_to_end();
        Some("keyword".into())
    }

    fn indent(&self, state: &Self::State, _: &str, _: usize) -> Option<usize> {
        Some(*state.borrow())
    }
}

#[test]
fn failed_edits_leave_text_tokens_and_mutable_checkpoints_unchanged() {
    let language = StreamLanguage::new(Fallible);
    let mut doc = language.parse(Text::new("é\nx\n"), SETTINGS).unwrap();
    let invalid = ChangeSet::single(Change::new(1..1, "!")).unwrap();
    assert_eq!(
        doc.apply_changes(&invalid),
        Err(StreamUpdateError::Edit(EditError::InvalidRange))
    );
    let stalls = ChangeSet::single(Change::new(3..4, "!")).unwrap();
    assert_eq!(
        doc.apply_changes(&stalls),
        Err(StreamUpdateError::Tokenize(TokenizeError::NoProgress {
            position: 3
        }))
    );
    assert_eq!(doc.text().as_str(), "é\nx\n");
    assert_eq!(
        styled(&doc, 0..5),
        [span(0..2, "é", "keyword"), span(3..4, "x", "keyword")]
    );
    for position in [3, 3, 5] {
        assert_eq!(
            doc.indentation(position, IndentOptions::default()),
            Ok(Some(if position == 3 { 1 } else { 2 }))
        );
    }
}
