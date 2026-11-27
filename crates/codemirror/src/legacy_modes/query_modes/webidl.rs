use std::sync::LazyLock;

use super::{super::js_line_terminator, js_space, word};
use crate::stream_parser::StringStream;

const BUILTINS: &str = "Clamp Constructor EnforceRange Exposed ImplicitThis Global PrimaryGlobal LegacyArrayClass LegacyUnenumerableNamedProperties LenientThis NamedConstructor NewObject NoInterfaceObject OverrideBuiltins PutForwards Replaceable SameObject TreatNonObjectAsNull TreatNullAs EmptyString Unforgeable Unscopeable";
const TYPES: &str = "unsigned short long unrestricted float double boolean byte octet Promise ArrayBuffer DataView Int8Array Int16Array Int32Array Uint8Array Uint16Array Uint32Array Uint8ClampedArray Float32Array Float64Array ByteString DOMString USVString sequence object RegExp Error DOMException FrozenArray any void";
const KEYWORDS: &str = "attribute callback const deleter dictionary enum getter implements inherit interface iterable legacycaller maplike partial required serializer setlike setter static stringifier typedef optional readonly or";
const ATOMS: &str = "true false Infinity NaN null";

pub(super) fn autocomplete() -> &'static [&'static str] {
    static WORDS: LazyLock<Vec<&str>> = LazyLock::new(|| {
        [BUILTINS, TYPES, KEYWORDS, ATOMS]
            .into_iter()
            .flat_map(str::split_ascii_whitespace)
            .collect()
    });
    WORDS.as_slice()
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    in_comment: bool,
    after_inheritance: bool,
    start_def: bool,
    end_def: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let style = self.read(stream)?;
        let text = stream.current();
        self.after_inheritance = matches!(text, ":" | "implements");
        self.start_def =
            style == "keyword" && matches!(text, "callback" | "dictionary" | "enum" | "interface");
        self.end_def |= style == "keyword" && text == "typedef";
        Some(style)
    }

    fn read(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        let rest = &stream.line()[stream.pos()..];
        if self.in_comment || rest.starts_with("/*") {
            let body = if self.in_comment { rest } else { &rest[2..] };
            let line = body.split(js_line_terminator).next().unwrap_or("");
            if let Some(end) = line.find("*/") {
                self.in_comment = false;
                advance(stream, rest.len() - body.len() + end + 2);
            } else if self.in_comment {
                stream.skip_to_end();
            } else {
                self.in_comment = true;
                advance(stream, 2 + line.len());
            }
            return Some("comment");
        }
        if rest.starts_with("//") {
            stream.skip_to_end();
            return Some("comment");
        }
        if let Some(end) = number_end(rest) {
            advance(stream, end);
            return Some("number");
        }
        if let Some(body) = rest.strip_prefix('"')
            && let Some(end) = body.find('"')
        {
            advance(stream, end + 2);
            return Some("string");
        }

        let identifier = identifier_end(rest);
        if let Some(end) = identifier {
            if self.start_def {
                advance(stream, end);
                return Some("def");
            }
            if self.end_def && rest[end..].trim_start_matches(js_space).starts_with(';') {
                self.end_def = false;
                advance(stream, end);
                return Some("def");
            }
        }
        let head = rest.split(|ch| !word(ch)).next().unwrap_or("");
        for (words, style) in [
            (KEYWORDS, "keyword"),
            (TYPES, "type"),
            (BUILTINS, "builtin"),
            (ATOMS, "atom"),
        ] {
            if words.split_ascii_whitespace().any(|word| word == head) {
                advance(stream, head.len());
                let style = if style == "type"
                    && (self.after_inheritance || used_as_name(&rest[head.len()..]))
                {
                    "builtin"
                } else {
                    style
                };
                return Some(style);
            }
        }
        if let Some(end) = identifier {
            advance(stream, end);
            return Some("variable");
        }
        stream
            .next()
            .filter(|&ch| ":<=>?".contains(ch))
            .map(|_| "operator")
    }
}

fn used_as_name(rest: &str) -> bool {
    let rest = rest.trim_start_matches(js_space);
    rest.strip_prefix("implements")
        .is_some_and(|after| !after.starts_with(word))
        || rest
            .strip_prefix('=')
            .is_some_and(|after| after.starts_with(word))
}

fn identifier_end(rest: &str) -> Option<usize> {
    let start = usize::from(rest.starts_with('_'));
    let tail = rest.get(start..)?;
    tail.starts_with(|ch: char| ch.is_ascii_alphabetic())
        .then(|| {
            start
                + tail
                    .bytes()
                    .take_while(|&ch| ch.is_ascii_alphanumeric() || matches!(ch, b'_' | b'-'))
                    .count()
        })
}

fn number_end(rest: &str) -> Option<usize> {
    // Integers have priority over floats. Only the floats that start with a
    // dot can reach the float rule; `1.2` is two tokens.
    let start = usize::from(rest.starts_with('-'));
    let tail = rest.get(start..)?;
    let end = match tail.as_bytes().first()? {
        b'1'..=b'9' => digits(tail),
        b'0' => {
            let hex = tail
                .get(2..)
                .filter(|_| matches!(tail.as_bytes().get(1), Some(b'x' | b'X')))
                .map(|rest| rest.bytes().take_while(u8::is_ascii_hexdigit).count())
                .unwrap_or(0);
            if hex > 0 {
                2 + hex
            } else {
                1 + tail[1..]
                    .bytes()
                    .take_while(|ch| matches!(ch, b'0'..=b'7'))
                    .count()
            }
        }
        b'.' => {
            let count = digits(&tail[1..]);
            if count == 0 {
                return None;
            }
            let end = 1 + count;
            let exp = &tail[end..];
            if exp.starts_with(['e', 'E']) {
                let sign = 1 + usize::from(exp[1..].starts_with(['+', '-']));
                let count = digits(&exp[sign..]);
                end + if count > 0 { sign + count } else { 0 }
            } else {
                end
            }
        }
        _ => return None,
    };
    Some(start + end)
}

fn digits(text: &str) -> usize {
    text.bytes().take_while(u8::is_ascii_digit).count()
}

fn advance(stream: &mut StringStream<'_>, bytes: usize) {
    stream
        .set_pos(stream.pos() + bytes)
        .expect("token ends at a scalar boundary");
}
