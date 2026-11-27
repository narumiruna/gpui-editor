use std::{borrow::Cow, num::NonZeroUsize};

use gpui_editor_codemirror::stream_parser::{
    InvalidStreamPosition, StreamParser, StringStream, TokenizeError, Tokenizer,
};
use regex::Regex;

const TAB: NonZeroUsize = NonZeroUsize::new(4).unwrap();

#[test]
fn reads_scalars_and_checks_byte_boundaries() {
    let mut stream = StringStream::new("é🦀αβ.[] foo", TAB, 2);
    assert!(stream.sol());
    assert!(!stream.eol());
    assert_eq!(stream.peek(), Some('é'));
    assert_eq!(stream.pos(), 0);
    assert_eq!(stream.eat('e'), None);
    assert_eq!(stream.next(), Some('é'));
    assert_eq!(stream.pos(), 2);
    assert_eq!(stream.set_pos(3), Err(InvalidStreamPosition));
    assert_eq!(stream.set_pos(usize::MAX), Err(InvalidStreamPosition));
    assert_eq!(stream.back_up(3), Err(InvalidStreamPosition));
    assert_eq!(stream.pos(), 2);

    stream.start_token();
    assert_eq!(stream.eat_if(|ch| ch == '🦀'), Some('🦀'));
    assert_eq!(stream.pos(), 6);
    assert_eq!(stream.current(), "🦀");
    assert_eq!(stream.back_up(1), Err(InvalidStreamPosition));
    stream.back_up(6).unwrap();
    assert!(stream.sol());
    assert_eq!(stream.current(), "");
    stream.set_pos(6).unwrap();

    let greek = Regex::new(r"^\p{Greek}$").unwrap();
    assert_eq!(stream.eat_regex(&greek), Some('α'));
    assert!(stream.eat_while_regex(&greek));
    assert_eq!(stream.current(), "🦀αβ");
    assert!(!stream.eat_while_regex(&greek));
    assert!(!stream.skip_to("missing"));
    assert!(stream.skip_to("[]"));
    assert_eq!(stream.current(), "🦀αβ.");
    assert_eq!(stream.peek(), Some('['));
    assert!(stream.skip_to("["));
    stream.skip_to_end();
    assert!(stream.eol());
    assert_eq!(stream.peek(), None);
    assert_eq!(stream.next(), None);
    assert_eq!(
        stream.eat_if(|_| panic!("no scalar at the end of a line")),
        None
    );
}

#[test]
fn columns_count_scalars_from_the_token_start_and_indentation_uses_ecmascript_space() {
    let prefix = " \t\u{a0}\u{feff}";
    let line = " \t\u{a0}\u{feff}π\u{301}🦀\t!";
    let tab = NonZeroUsize::new(8).unwrap();
    let mut stream = StringStream::new(line, tab, 3);
    assert_eq!(stream.indent_unit(), 3);
    assert_eq!(stream.indentation(), 10);
    assert!(stream.eat_space());
    assert_eq!(stream.pos(), prefix.len());
    assert_eq!(stream.column(), 0);
    stream.start_token();
    assert_eq!(stream.column(), 10);
    assert!(!stream.eat_space());
    assert!(stream.skip_to("\t!"));
    stream.start_token();
    assert_eq!(stream.column(), 13);
    assert_eq!(stream.next(), Some('\t'));
    stream.start_token();
    assert_eq!(stream.column(), 16);
    stream.set_pos(1).unwrap();
    stream.start_token();
    assert_eq!(stream.column(), 1);
    assert_eq!(stream.indentation(), 10);

    let mut stream = StringStream::new("\t\t\u{2028}\u{2029}\r\u{85}a", TAB, 2);
    assert_eq!(stream.indentation(), 11);
    assert!(stream.eat_space());
    assert_eq!(stream.peek(), Some('\u{85}'));
    let mut stream = stream.with_indentation_override(0);
    stream.start_token();
    assert_eq!(stream.indentation(), 0);
    assert_eq!(stream.column(), 11);
    assert_eq!(StringStream::new(" \t", TAB, 2).indentation(), 4);
}

#[test]
fn literal_and_regex_matches_use_the_cursor_and_can_leave_it_unchanged() {
    let mut stream = StringStream::new("KélAN 🦀123 a", TAB, 2);
    assert!(!stream.match_str("KÉLAN", true, false));
    assert!(!stream.match_str("KÉLAN too long", true, true));
    assert!(stream.match_str("KÉLAN", false, true));
    assert_eq!(stream.pos(), 0);
    assert!(stream.match_str("KÉLAN", true, true));
    assert_eq!(stream.current(), "KélAN");
    assert!(stream.match_str(" ", true, false));
    assert_eq!(stream.pos(), "KélAN ".len());
    stream.start_token();

    let pattern = Regex::new(r"^(?P<glyph>🦀)([0-9]+)").unwrap();
    let captures = stream.match_regex(&pattern, false).unwrap();
    assert_eq!(captures.name("glyph").unwrap().range(), 0..4);
    assert_eq!(captures.get(2).unwrap().range(), 4..7);
    assert_eq!(stream.current(), "");
    assert_eq!(
        stream
            .match_regex(&pattern, true)
            .unwrap()
            .get(0)
            .unwrap()
            .as_str(),
        "🦀123"
    );
    assert_eq!(stream.current(), "🦀123");
    assert!(
        stream
            .match_regex(&Regex::new("a$").unwrap(), true)
            .is_none()
    );
    assert_eq!(stream.peek(), Some(' '));
    assert_eq!(
        stream
            .match_regex(&Regex::new("^").unwrap(), true)
            .unwrap()
            .get(0)
            .unwrap()
            .range(),
        0..0
    );
    assert_eq!(stream.peek(), Some(' '));
    assert!(
        stream
            .match_regex(&Regex::new(" a$").unwrap(), true)
            .is_some()
    );
    assert!(stream.eol());
    assert!(stream.match_str("", true, false));
}

struct BlockMode {
    word: Regex,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Comment {
    Outside,
    Continue,
    Inside,
}

#[derive(Clone)]
struct BlockState {
    unit: usize,
    depth: usize,
    comment: Comment,
    blank_lines: usize,
    blank_comment_lines: usize,
    empty_tokens: usize,
}

impl StreamParser for BlockMode {
    type State = BlockState;

    fn name(&self) -> Option<&str> {
        Some("block")
    }

    fn start_state(&self, indent_unit: usize) -> Self::State {
        BlockState {
            unit: indent_unit,
            depth: 0,
            comment: Comment::Outside,
            blank_lines: 0,
            blank_comment_lines: 0,
            empty_tokens: 0,
        }
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        state: &mut Self::State,
    ) -> Option<Cow<'_, str>> {
        assert_eq!(stream.indent_unit(), state.unit);
        if state.comment == Comment::Continue {
            assert!(stream.sol());
            state.comment = Comment::Inside;
            state.empty_tokens += 1;
            return Some("discarded style".into());
        }
        if state.comment == Comment::Inside || stream.match_str("/*", true, false) {
            if stream.skip_to("*/") {
                assert!(stream.match_str("*/", true, false));
                state.comment = Comment::Outside;
            } else {
                stream.skip_to_end();
                state.comment = Comment::Continue;
            }
            return Some("comment".into());
        }
        if stream.eat_space() {
            return None;
        }
        if stream.match_regex(&self.word, true).is_some() {
            return match stream.current() {
                "let" => Some("keyword".into()),
                name => Some(format!("variableName.{name}").into()),
            };
        }
        match stream.next() {
            Some('{') => state.depth += 1,
            Some('}') => state.depth -= 1,
            _ => {}
        }
        Some("punctuation".into())
    }

    fn blank_line(&self, state: &mut Self::State, indent_unit: usize) {
        assert_eq!(state.unit, indent_unit);
        state.blank_lines += 1;
        if state.comment != Comment::Outside {
            state.blank_comment_lines += 1;
        }
    }

    fn indent(&self, state: &Self::State, text_after: &str, indent_unit: usize) -> Option<usize> {
        let depth = state
            .depth
            .saturating_sub(usize::from(text_after.starts_with('}')));
        Some(depth * indent_unit)
    }

    fn merge_tokens(&self) -> bool {
        false
    }
}

#[test]
fn yields_tokens_lazily_with_state_across_mixed_line_endings() {
    let source = "let café = {\r\n\t/* start\r\n\r\nβ */\n \n\tλ\rtext\n}\n";
    let mode = BlockMode {
        word: Regex::new(r"^\p{L}+").unwrap(),
    };
    let mut tokens = Tokenizer::new(&mode, source, TAB, 2);
    let expected = [
        (0..3, "let", Some("keyword")),
        (3..4, " ", None),
        (4..9, "café", Some("variableName.café")),
        (9..10, " ", None),
        (10..11, "=", Some("punctuation")),
        (11..12, " ", None),
        (12..13, "{", Some("punctuation")),
        (15..16, "\t", None),
        (16..24, "/* start", Some("comment")),
        (28..33, "β */", Some("comment")),
        (34..35, " ", None),
        (36..37, "\t", None),
        (37..39, "λ", Some("variableName.λ")),
        (39..40, "\r", None),
        (40..44, "text", Some("variableName.text")),
        (45..46, "}", Some("punctuation")),
    ];
    assert_eq!(mode.name(), Some("block"));
    assert!(!mode.merge_tokens());
    let mut saved = None;
    for (range, text, style) in expected {
        let token = tokens.next().unwrap().unwrap();
        assert_eq!(token.range, range);
        assert_eq!(token.text, text);
        assert_eq!(token.text, &source[token.range]);
        assert_eq!(token.style.as_deref(), style);
        match text {
            "{" => {
                assert_eq!(mode.indent(tokens.state(), "x", 2), Some(2));
                assert_eq!(mode.indent(tokens.state(), "}", 2), Some(0));
                saved = Some(mode.copy_state(tokens.state()));
            }
            "/* start" => assert_eq!(tokens.state().blank_lines, 0),
            "β */" => {
                assert_eq!(tokens.state().blank_lines, 1);
                assert_eq!(tokens.state().blank_comment_lines, 1);
                assert_eq!(tokens.state().empty_tokens, 1);
            }
            _ => {}
        }
    }
    assert_eq!(saved.unwrap().depth, 1);
    assert_eq!(tokens.state().depth, 0);
    assert_eq!(tokens.state().blank_lines, 1);
    assert_eq!(tokens.next(), None);
    assert_eq!(tokens.state().blank_lines, 2);
    assert_eq!(tokens.next(), None);
    assert_eq!(tokens.state().blank_lines, 2);
}

#[test]
fn blank_hook_includes_the_empty_input_and_each_ending_without_content() {
    let mode = BlockMode {
        word: Regex::new("^x").unwrap(),
    };
    for (source, blank_lines) in [("", 1), ("\n\r\n", 3)] {
        let mut tokens = Tokenizer::new(&mode, source, TAB, 2);
        assert_eq!(tokens.state().blank_lines, 0);
        assert_eq!(tokens.next(), None);
        assert_eq!(tokens.state().blank_lines, blank_lines);
    }
}

struct RetryMode(Option<usize>);

impl StreamParser for RetryMode {
    type State = usize;

    fn start_state(&self, _: usize) -> Self::State {
        0
    }

    fn token(
        &self,
        stream: &mut StringStream<'_>,
        attempts: &mut Self::State,
    ) -> Option<Cow<'_, str>> {
        if stream.eat('é').is_some() {
            return None;
        }
        *attempts += 1;
        assert_eq!(stream.pos(), stream.start());
        assert!(stream.match_str("", true, false));
        if self.0 == Some(*attempts) {
            stream.next();
        }
        Some("retry".into())
    }
}

#[test]
fn retries_zero_length_tokens_only_up_to_the_tenth_attempt() {
    let mut tokens = Tokenizer::new(&RetryMode(Some(10)), "x", TAB, 2);
    let token = tokens.next().unwrap().unwrap();
    assert_eq!(token.range, 0..1);
    assert_eq!(token.style.as_deref(), Some("retry"));
    assert_eq!(*tokens.state(), 10);
    assert_eq!(tokens.next(), None);

    let mut tokens = Tokenizer::new(&RetryMode(None), "é\r\nx\ny", TAB, 2);
    assert_eq!(tokens.next().unwrap().unwrap().range, 0..2);
    assert_eq!(
        tokens.next(),
        Some(Err(TokenizeError::NoProgress { position: 4 }))
    );
    assert_eq!(*tokens.state(), 10);
    assert_eq!(tokens.next(), None);
}

#[test]
fn a_replaced_line_cannot_emit_tokens_outside_the_input() {
    struct ReplacesLine;

    impl StreamParser for ReplacesLine {
        type State = ();

        fn start_state(&self, _: usize) -> Self::State {}

        fn token(
            &self,
            stream: &mut StringStream<'_>,
            _: &mut Self::State,
        ) -> Option<Cow<'_, str>> {
            *stream = StringStream::new("other line", TAB, 2);
            stream.skip_to_end();
            None
        }
    }

    let mut tokens = Tokenizer::new(&ReplacesLine, "a", TAB, 2);
    assert_eq!(
        tokens.next(),
        Some(Err(TokenizeError::ReplacedLine { position: 0 }))
    );
    assert_eq!(tokens.next(), None);
}
