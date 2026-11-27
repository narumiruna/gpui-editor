use std::sync::LazyLock;

use regex::Regex;

use super::{QuoteEnd, StringStream, block_comment_ended, js_space, leading_word, quoted, word};

const MIDDLE: [&str; 17] = [
    "in", "then", "else", "of", "elseof", "elsecase", "elseif", "catch", "finally", "with",
    "require", "prepare", "import", "export", "define", "do", "end",
];
const COMMON: &str = "andthen at attr declare feat from lex mod div mode orelse parser prod prop scanner self syn token";
const OPENING: &str = "local proc fun case class if cond or dis choice not thread try raise lock for suchthat meth functor";
static HEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[xX][0-9a-fA-F]+").unwrap());
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]*(?:\.[0-9]+)?(?:[eE][~+]?[0-9]+)?").unwrap());
static QUOTED_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("^`[^\n\r\u{2028}\u{2029}]+`").unwrap());

#[derive(Clone, Copy, Debug, Default)]
enum Scan {
    #[default]
    Base,
    Class,
    Method,
    Procedure,
    ProcedureName,
    String(char),
    Comment,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    scan: Scan,
    indent: isize,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self.scan {
            Scan::Base => self.base(stream),
            Scan::String(quote) => Some(self.string(stream, quote)),
            Scan::Comment => Some(self.comment(stream)),
            _ if stream.eat_space() => None,
            Scan::Procedure => {
                if stream.eat('{').is_some() {
                    self.scan = Scan::ProcedureName;
                    Some("bracket")
                } else {
                    self.scan = Scan::Base;
                    None
                }
            }
            Scan::Class | Scan::Method | Scan::ProcedureName => {
                let scan = self.scan;
                self.scan = Scan::Base;
                let method = matches!(scan, Scan::Method);
                if stream
                    .eat_if(|ch| ch.is_ascii_uppercase() || (method && ch.is_ascii_lowercase()))
                    .is_some()
                {
                    stream.eat_while(word);
                } else if stream.match_regex(&QUOTED_NAME, true).is_none()
                    && matches!(scan, Scan::ProcedureName)
                {
                    stream.eat('$');
                }
                Some(if matches!(scan, Scan::Class) {
                    "type"
                } else {
                    "def"
                })
            }
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        if stream.eat_if(|ch| matches!(ch, '{' | '}')).is_some() {
            return Some("bracket");
        }
        if stream.match_str("[]", true, false) {
            return Some("keyword");
        }
        if [
            ":::", "...", "=<:", ">=:", "<-", ":=", "=<", ">=", "<=", "<:", ">:", "=:", "\\=",
            "!!", "==", "::",
        ]
        .iter()
        .any(|op| stream.match_str(op, true, false))
        {
            return Some("operator");
        }
        let keyword = leading_word(&stream.line()[stream.pos()..]);
        let style = match keyword {
            "true" | "false" | "nil" | "unit" => Some("atom"),
            key if OPENING.split_ascii_whitespace().any(|word| word == key) => {
                self.indent += 1;
                self.scan = match key {
                    "proc" | "fun" => Scan::Procedure,
                    "class" => Scan::Class,
                    "meth" => Scan::Method,
                    _ => Scan::Base,
                };
                Some("keyword")
            }
            "end" => {
                self.indent -= 1;
                Some("keyword")
            }
            key if MIDDLE.contains(&key)
                || COMMON.split_ascii_whitespace().any(|word| word == key) =>
            {
                Some("keyword")
            }
            _ => None,
        };
        if style.is_some() {
            stream.set_pos(stream.pos() + keyword.len()).unwrap();
            return style;
        }
        match stream.next()? {
            quote @ ('"' | '\'') => Some(self.string(stream, quote)),
            ch @ ('~' | '0'..='9') => {
                let first_digit = if ch == '~' {
                    stream.eat_if(|ch| ch.is_ascii_digit())?
                } else {
                    ch
                };
                if first_digit != '0' || stream.match_regex(&HEX, true).is_none() {
                    stream.match_regex(&NUMBER, true);
                }
                Some("number")
            }
            '%' => {
                stream.skip_to_end();
                Some("comment")
            }
            '/' if stream.eat('*').is_some() => Some(self.comment(stream)),
            ch if "^@!|<>#.*-+\\/,=".contains(ch) => Some("operator"),
            _ => {
                stream.eat_while(word);
                Some("variable")
            }
        }
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        self.scan = if quoted(stream, quote, '\\') == QuoteEnd::Escaped {
            Scan::String(quote)
        } else {
            Scan::Base
        };
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        self.scan = if block_comment_ended(stream, false) {
            Scan::Base
        } else {
            Scan::Comment
        };
        "comment"
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        let text = text_after.trim_matches(js_space);
        let closing = MIDDLE.contains(&leading_word(text)) || text.contains("[]");
        (self.indent - isize::from(closing))
            .max(0)
            .unsigned_abs()
            .saturating_mul(unit)
    }
}

pub(super) fn electric_input(input: &str) -> bool {
    input.contains(['[', ']']) || MIDDLE.iter().any(|word| input.ends_with(word))
}
