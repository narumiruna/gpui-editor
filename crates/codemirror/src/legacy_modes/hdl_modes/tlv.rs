use super::word;
use crate::{legacy_modes::js_line_terminator, stream_parser::StringStream};

const INDENT_UNIT: usize = 3;

#[derive(Clone, Debug)]
pub(super) struct State {
    active: bool,
    block_comment: bool,
    scope_levels: usize,
    next_indent: Option<usize>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            active: true,
            block_comment: false,
            scope_levels: 0,
            next_indent: None,
        }
    }
}

impl State {
    pub(super) fn indent(&self) -> Option<usize> {
        self.next_indent.filter(|_| self.active)
    }

    pub(super) fn token(
        &mut self,
        stream: &mut StringStream<'_>,
        indented: &mut usize,
    ) -> Option<&'static str> {
        if stream.sol() && !self.block_comment {
            let line = stream.line();
            let region = line.starts_with('\\');
            if region {
                stream.skip_to_end();
                if line.contains("\\SV") {
                    self.active = false;
                } else if line.contains("\\TLV") {
                    self.active = true;
                }
            }
            if self.active
                && stream.sol()
                && *indented == 0
                && let Some(prefix) = line.strip_prefix(['!', ' '])
            {
                *indented = 1 + prefix.bytes().take_while(|&ch| ch == b' ').count();
            }
            self.next_indent = Some(self.line_indent(line, *indented));
            if region {
                return Some("def");
            }
        }
        if !self.active {
            return m4(stream, false);
        }
        if self.block_comment {
            let remaining = &stream.line()[stream.pos()..];
            // ECMAScript `.` does not match line terminators.
            let body = remaining
                .split(js_line_terminator)
                .next()
                .unwrap_or_default();
            if let Some(end) = body.find("*/") {
                stream.set_pos(stream.pos() + end + 2).unwrap();
                self.block_comment = false;
            } else {
                stream.skip_to_end();
            }
            return Some("comment");
        }
        if stream.match_str("//", true, false) {
            stream.skip_to_end();
            return Some("comment");
        }
        if stream.match_str("/*", true, false) {
            self.block_comment = true;
            return Some("comment");
        }
        if let Some((prefix, mnemonic)) = identifier(&stream.line()[stream.pos()..]) {
            let end = stream.pos() + prefix.len() + mnemonic.len();
            if let Some(style) = identifier_style(prefix)
                && (!mnemonic.is_empty() || end == stream.line().len())
            {
                stream.set_pos(end).unwrap();
                return Some(style);
            }
            // The hook advances by one character but leaves styling to Verilog.
            stream.next();
            return None;
        }
        if stream.eat_while(|ch| ch == '\t') {
            return Some("invalid");
        }
        if stream.eat_while(|ch| "[]{}();:".contains(ch)) {
            return Some("meta");
        }
        if let Some(style) = m4(stream, true) {
            return Some(style);
        }
        if stream.eat_while(|ch| ch == ' ') {
            return stream.eol().then_some("error");
        }
        if stream.eat_while(word) {
            return Some("number");
        }
        stream.next();
        None
    }

    fn line_indent(&mut self, line: &str, indented: usize) -> usize {
        if indented > self.scope_levels.saturating_mul(INDENT_UNIT) {
            return indented;
        }
        let (len, body) = if line.is_ascii() {
            (line.len(), line.get(indented..))
        } else {
            // Upstream uses a column offset as a UTF-16 index, even with tabs.
            (line.encode_utf16().count(), suffix_at_utf16(line, indented))
        };
        let mut next_indent = indented;
        let mut scope_limit = indented;
        if let Some(body) = body
            && body.starts_with(['/', '>', '-', '|', '?', '@', '\\'])
            && identifier(body).is_some_and(|(prefix, _)| identifier_style(prefix).is_some())
        {
            next_indent = next_indent.saturating_add(INDENT_UNIT);
            if indented == 0 || !body.starts_with('\\') {
                scope_limit = scope_limit.saturating_add(INDENT_UNIT);
                // A fractional depth does not set an array index upstream.
                if indented.is_multiple_of(INDENT_UNIT) {
                    self.scope_levels = self.scope_levels.max(scope_limit / INDENT_UNIT);
                }
            }
        }
        if len != indented {
            self.scope_levels = self.scope_levels.min(scope_limit / INDENT_UNIT);
        }
        next_indent
    }
}

fn identifier(text: &str) -> Option<(&str, &str)> {
    // In the upstream regex `\*-\+` is a range from `*` to `+`.
    // As a result no identifier prefix contains a literal hyphen.
    let prefix = text.trim_start_matches(|ch| "~!@#$%^&*+=?/\\|'\"<>".contains(ch));
    let prefix_len = text.len() - prefix.len();
    if prefix_len == 0 {
        return None;
    }
    let mnemonic = prefix.trim_start_matches(word);
    Some((
        &text[..prefix_len],
        &prefix[..prefix.len() - mnemonic.len()],
    ))
}

fn identifier_style(prefix: &str) -> Option<&'static str> {
    Some(match prefix {
        "|" => "link",
        ">" | "/" => "property",
        "$" | "$$" | "*" | "**" => "variable",
        "?$" | "?*" => "qualifier",
        "@" | "@++" | "@+=" => "variableName.special",
        "%+" | "%" | ">>" | "<<" | "<>" | "#" => "tag",
        "^" | "^^" | "^!" => "attribute",
        "\\" => "keyword",
        "\"" => "comment",
        _ => return None,
    })
}

fn m4(stream: &mut StringStream<'_>, tlv: bool) -> Option<&'static str> {
    let bytes = &stream.line().as_bytes()[stream.pos()..];
    if !matches!(bytes, [b'm' | b'M', b'4', ..]) {
        return None;
    }
    stream.next();
    stream.next();
    let plus = tlv && stream.eat('+').is_some();
    stream.eat_while(word);
    Some(if plus { "keyword.special" } else { "keyword" })
}

fn suffix_at_utf16(line: &str, mut index: usize) -> Option<&str> {
    for (offset, ch) in line.char_indices() {
        if index == 0 {
            return Some(&line[offset..]);
        }
        index = index.checked_sub(ch.len_utf16())?;
    }
    (index == 0).then_some("")
}
