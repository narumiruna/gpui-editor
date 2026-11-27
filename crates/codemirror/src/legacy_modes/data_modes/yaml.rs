use super::{super::js_space, word};
use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    pair: bool,
    pair_start: bool,
    key_column: usize,
    inline_pairs: isize,
    inline_list: isize,
    literal: bool,
    escaped: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let escaped = std::mem::take(&mut self.escaped);
        let start = stream.pos();
        let ch = stream.peek()?;
        let rest = &stream.line()[start..];
        if ch == '#' && (start == 0 || stream.line()[..start].ends_with(js_space)) {
            stream.skip_to_end();
            return Some("comment");
        }
        if matches!(ch, '\'' | '"') {
            stream.next();
            // The pinned regex takes the first matching quote, even after a backslash.
            stream.eat_while(|c| c != ch);
            stream.eat(ch);
            return Some("string");
        }
        if self.literal && stream.indentation() > self.key_column {
            stream.skip_to_end();
            return Some("string");
        }
        self.literal = false;

        if stream.sol() {
            self.reset_pair();
            if stream.match_str("---", true, false) || stream.match_str("...", true, false) {
                return Some("def");
            }
            if let Some(after_dash) = rest.trim_start_matches(js_space).strip_prefix('-')
                && after_dash.starts_with(js_space)
            {
                let end = rest.len() - after_dash.trim_start_matches(js_space).len();
                stream.set_pos(end).unwrap();
                return Some("meta");
            }
        }
        match ch {
            '{' => self.inline_pairs += 1,
            '}' => self.inline_pairs -= 1,
            '[' => self.inline_list += 1,
            ']' => self.inline_list -= 1,
            ',' if !escaped && self.inline_list > 0 => {}
            ',' if !escaped && self.inline_pairs > 0 => self.reset_pair(),
            _ => {
                if self.pair_start
                    && let Some(style) = self.value(stream, rest)
                {
                    return Some(style);
                }
                if !self.pair
                    && let Some(end) = key_end(rest)
                {
                    stream.set_pos(start + end).unwrap();
                    self.pair = true;
                    self.key_column = stream.indentation();
                    return Some("atom");
                }
                if self.pair && stream.eat(':').is_some() {
                    stream.eat_space();
                    self.pair_start = true;
                    return Some("meta");
                }
                self.pair_start = false;
                self.escaped = ch == '\\';
                stream.next();
                return None;
            }
        }
        stream.next();
        Some("meta")
    }

    fn reset_pair(&mut self) {
        self.key_column = 0;
        self.pair = false;
        self.pair_start = false;
    }

    fn value(&mut self, stream: &mut StringStream<'_>, rest: &str) -> Option<&'static str> {
        let text = rest.trim_start_matches(js_space);
        let start = stream.pos() + rest.len() - text.len();
        let (end, style) = if let Some(tail) = text.strip_prefix(['|', '>']) {
            self.literal = true;
            (text.len() - tail.trim_start_matches(js_space).len(), "meta")
        } else if let Some(end) = reference_end(text) {
            (end, "variable")
        } else if let Some(end) = number_end(text, self.inline_pairs) {
            (end, "number")
        } else if ["true", "false", "on", "off", "yes", "no"]
            .iter()
            .any(|keyword| rest.eq_ignore_ascii_case(keyword))
        {
            (text.len(), "keyword")
        } else {
            return None;
        };
        stream.set_pos(start + end).unwrap();
        Some(style)
    }
}

fn reference_end(text: &str) -> Option<usize> {
    let tail = text.strip_prefix(['&', '*'])?;
    let end = tail
        .find(|c| !word(c) && !matches!(c, '.' | '-'))
        .unwrap_or(tail.len());
    // The ASCII word boundary can leave trailing dots or dashes unconsumed.
    let word_end = tail[..end].rfind(word)?;
    Some(word_end + 2)
}

fn number_end(text: &str, inline_pairs: isize) -> Option<usize> {
    let sign = usize::from(text.starts_with('-'));
    let tail = &text[sign..];
    let digits = tail
        .find(|c: char| !c.is_ascii_digit() && !matches!(c, '.' | ','))
        .unwrap_or(tail.len());
    if digits == 0 {
        return None;
    }
    let end = sign + digits;
    let rest = &text[end..];
    let space = rest
        .chars()
        .next()
        .filter(|&c| js_space(c))
        .map_or(0, char::len_utf8);
    if inline_pairs == 0 && rest.len() == space {
        Some(end + space)
    } else if inline_pairs > 0 {
        if rest[space..].starts_with([',', '}']) {
            Some(end + space)
        } else {
            // The greedy numeric run includes commas. Backtrack to a separator.
            tail[..digits]
                .rfind(',')
                .filter(|&comma| comma > 0)
                .map(|comma| sign + comma)
        }
    } else {
        None
    }
}

fn key_end(text: &str) -> Option<usize> {
    let key = text.trim_start_matches(js_space);
    let spaces = text.len() - key.len();
    if let Some(prefix) = key_prefix(key)
        && let Some(end) = key_colon(&key[prefix..])
    {
        return Some(spaces + prefix + end);
    }
    // The leading whitespace can backtrack so that one space acts as the key prefix.
    if spaces != 0 {
        return key_colon(key).map(|end| spaces + end);
    }
    None
}

fn key_prefix(key: &str) -> Option<usize> {
    let mut chars = key.chars();
    let first = chars.next()?;
    if ",[]{}&*!|>'\"%@`".contains(first) {
        let second = chars.next()?;
        (!js_space(second) && !matches!(second, '\'' | '"' | ':'))
            .then_some(first.len_utf8() + second.len_utf8())
    } else {
        (first != '#').then_some(first.len_utf8())
    }
}

fn key_colon(text: &str) -> Option<usize> {
    let mut chars = text.char_indices().peekable();
    let mut whitespace = None;
    while let Some((at, ch)) = chars.next() {
        if ch == '#' {
            return None;
        }
        if ch == ':' && chars.peek().is_none_or(|&(_, next)| js_space(next)) {
            return Some(whitespace.unwrap_or(at));
        }
        if js_space(ch) {
            whitespace.get_or_insert(at);
        } else {
            whitespace = None;
        }
    }
    None
}
