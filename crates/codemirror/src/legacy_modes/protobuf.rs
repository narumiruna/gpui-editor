use std::sync::LazyLock;

use regex::Regex;

use crate::stream_parser::StringStream;

#[rustfmt::skip]
pub(super) const KEYWORDS: &[&str] = &[
    "package", "message", "import", "syntax",
    "required", "optional", "repeated", "reserved", "default", "extensions", "packed",
    "bool", "bytes", "double", "enum", "float", "string",
    "int32", "int64", "uint32", "uint64", "sint32", "sint64", "fixed32", "fixed64", "sfixed32", "sfixed64",
    "option", "service", "rpc", "returns",
];

static KEYWORD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(&format!(r"^(?i-u:{})(?-u:\b)", KEYWORDS.join("|"))).unwrap());
static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[+-]?(?:0x[0-9a-fA-F]+|[0-9]*\.[0-9]+(?:[EeDd][+-]?[0-9]+)?|[0-9]+(?:[EeDd][+-]?[0-9]+)?)").unwrap()
});
static STRING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(?:"(?:[^"]|"")*"|'(?:[^']|'')*')"#).unwrap());

pub(super) fn token(stream: &mut StringStream<'_>) -> Option<&'static str> {
    if stream.eat_space() {
        return None;
    }
    if stream.match_str("//", true, false) {
        stream.skip_to_end();
        return Some("comment");
    }
    if stream.match_regex(&NUMBER, true).is_some() {
        return Some("number");
    }
    if stream.match_regex(&STRING, true).is_some() {
        return Some("string");
    }
    if stream.match_regex(&KEYWORD, true).is_some() {
        return Some("keyword");
    }
    if stream.eat_if(identifier_start).is_some() {
        stream.eat_while(|ch| identifier_start(ch) || ch.is_ascii_digit());
        return Some("variable");
    }
    stream.next();
    None
}

fn identifier_start(ch: char) -> bool {
    // Upstream's character class includes both UTF-16 surrogate ranges.
    ch.is_ascii_alphabetic() || ch == '_' || ch >= '\u{a1}'
}
