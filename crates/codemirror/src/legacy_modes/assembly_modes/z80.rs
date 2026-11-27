use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, word};

static Z80_OP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i-u:^(?:exx?|(?:ld|cp|in)(?:[di]r?)?|pop|push|ad[cd]|cpl|daa|dec|inc|neg|sbc|sub|and|bit|",
        r"[cs]cf|x?or|res|set|r[lr]c?a?|r[lr]d|s[lr]a|srl|djnz|nop|rst|[de]i|halt|im|ot[di]r|out[di]?)\b)",
    ))
    .unwrap()
});
static EZ80_OP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i-u:^(?:exx?|(?:ld|cp)(?:[di]r?)?|[lp]ea|pop|push|ad[cd]|cpl|daa|dec|inc|neg|sbc|sub|and|bit|",
        r"[cs]cf|x?or|res|set|r[lr]c?a?|r[lr]d|s[lr]a|srl|djnz|nop|[de]i|halt|im|",
        r"in(?:[di]mr?|ir?|irx|2r?)|ot(?:dmr?|[id]rx|imr?)|out(?:0?|[di]r?|[di]2r?)|tst(?:io)?|slp)",
        r"(?:\.(?:[sl]?i)?[sl])?\b)",
    ))
    .unwrap()
});
static Z80_BRANCH: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i-u:^(?:call|j[pr]|ret[in]?|b_?(?:call|jump))\b)").unwrap());
static EZ80_BRANCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i-u:^(?:(?:call|j[pr]|rst|ret[in]?)(?:\.(?:[sl]?i)?[sl])?|(?:rs|st)mix)\b)")
        .unwrap()
});
static REGISTERS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i-u:^(?:af?|bc?|c|de?|e|hl?|l|i[xy]?|r|sp)\b)").unwrap());
static CONDITIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i-u:^(?:n?[zc]|p[oe]?|m)\b)").unwrap());
static ERRORS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i-u:^(?:[hl][xy]|i[xy][hl]|slia|sll)\b)").unwrap());
static NUMBERS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i-u:^(?:[\da-f]+h|[0-7]+o|[01]+b|\d+d?)\b)").unwrap());

#[derive(Clone, Debug, Default)]
pub(super) struct State(Context);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Context {
    #[default]
    Neutral,
    Op,
    Branch,
    Operand,
}

impl State {
    pub(super) fn token(
        &mut self,
        stream: &mut StringStream<'_>,
        ez80: bool,
    ) -> Option<&'static str> {
        if stream.column() == 0 {
            self.0 = Context::Neutral;
        }
        if stream.eat_space() {
            return None;
        }
        if stream.eat_while(word) {
            if ez80 && stream.eat('.').is_some() {
                stream.eat_while(word);
            }
            return self.style_word(stream, ez80);
        }
        if stream.eat(';').is_some() {
            stream.skip_to_end();
            return Some("comment");
        }
        if stream.eat('"').is_some() {
            while let Some(ch) = stream.next() {
                if ch == '"' {
                    break;
                }
                if ch == '\\' {
                    stream.next();
                }
            }
            return Some("string");
        }
        if stream.eat('\'').is_some() {
            return char_literal(stream).then_some("number");
        }
        if stream.eat('.').is_some() || stream.sol() && stream.eat('#').is_some() {
            self.0 = Context::Neutral;
            return stream.eat_while(word).then_some("def");
        }
        if stream.eat('$').is_some() {
            return stream
                .eat_while(|ch| ch.is_ascii_hexdigit())
                .then_some("number");
        }
        if stream.eat('%').is_some() {
            return stream
                .eat_while(|ch| matches!(ch, '0' | '1'))
                .then_some("number");
        }
        stream.next();
        None
    }

    fn style_word(&mut self, stream: &mut StringStream<'_>, ez80: bool) -> Option<&'static str> {
        if stream.indentation() == 0 {
            // The upstream mode matches at the cursor, after the word.
            return stream.match_regex(&NUMBERS, true).map(|_| "number");
        }
        let text = stream.current();
        if matches!(self.0, Context::Op | Context::Operand) && REGISTERS.is_match(text) {
            self.0 = Context::Operand;
            return Some("variable");
        }
        if self.0 == Context::Branch && CONDITIONS.is_match(text) {
            self.0 = Context::Operand;
            return Some("variableName.special");
        }
        let (ops, branches) = if ez80 {
            (&EZ80_OP, &EZ80_BRANCH)
        } else {
            (&Z80_OP, &Z80_BRANCH)
        };
        if ops.is_match(text) {
            self.0 = Context::Op;
            Some("keyword")
        } else if branches.is_match(text) {
            self.0 = Context::Branch;
            Some("keyword")
        } else if self.0 == Context::Operand && NUMBERS.is_match(text) {
            Some("number")
        } else {
            ERRORS.is_match(text).then_some("error")
        }
    }
}

fn char_literal(stream: &mut StringStream<'_>) -> bool {
    let rest = &stream.line()[stream.pos()..];
    // The optional escape may backtrack. The literal covers one UTF-16 code unit.
    for text in rest
        .strip_prefix('\\')
        .into_iter()
        .chain(std::iter::once(rest))
    {
        let mut chars = text.chars();
        if chars.next().is_some_and(|ch| {
            ch.len_utf16() == 1 && !matches!(ch, '\r' | '\n' | '\u{2028}' | '\u{2029}')
        }) && chars.next() == Some('\'')
        {
            stream
                .set_pos(stream.line().len() - chars.as_str().len())
                .unwrap();
            return true;
        }
    }
    false
}
