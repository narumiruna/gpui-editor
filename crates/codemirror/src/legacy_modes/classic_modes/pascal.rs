use std::{collections::HashMap, sync::LazyLock};

use super::{identifier, number_tail, string_continues, word_styles};
use crate::stream_parser::StringStream;

const KEYWORDS: &str = "\
    absolute and array asm begin case const constructor destructor div do downto else end file \
    for function goto if implementation in inherited inline interface label mod nil not object \
    of operator or packed procedure program record reintroduce repeat self set shl shr string \
    then to type unit until uses var while with xor as class dispinterface except exports \
    finalization finally initialization is library on out property raise resourcestring \
    threadvar try abstract alias assembler bitpacked break cdecl continue cppdecl cvar default \
    deprecated dynamic enumerator experimental export external far far16 forward generic \
    helper implements index interrupt iocheck local message name near nodefault noreturn \
    nostackframe oldfpccall otherwise overload override pascal platform private protected \
    public published read register result safecall saveregisters softfloat specialize static \
    stdcall stored strict unaligned unimplemented varargs virtual write";

static STYLE: LazyLock<HashMap<&str, &str>> =
    LazyLock::new(|| word_styles(&[("atom", "null"), ("keyword", KEYWORDS)]));

const OPERATORS: &str = "+-*&%=<>!?|/";

#[derive(Clone, Debug, Default)]
enum Scan {
    #[default]
    Base,
    String(char),
    CommentParen,
    CommentBrace,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State(Scan);

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match self.0 {
            Scan::String(quote) => return Some(self.string(stream, quote)),
            Scan::CommentParen => return Some(self.comment_paren(stream)),
            Scan::CommentBrace => return Some(self.comment_brace(stream)),
            Scan::Base => {}
        }
        // The upstream start-of-line flag is never set, so `#` is not meta.
        let ch = stream.next()?;
        Some(match ch {
            '"' | '\'' => self.string(stream, ch),
            '(' if stream.eat('*').is_some() => self.comment_paren(stream),
            '{' => self.comment_brace(stream),
            '[' | ']' | '(' | ')' | ',' | ';' | ':' | '.' => return None,
            '0'..='9' => {
                stream.eat_while(number_tail);
                "number"
            }
            '/' if stream.eat('/').is_some() => {
                stream.skip_to_end();
                "comment"
            }
            ch if OPERATORS.contains(ch) => {
                stream.eat_while(|next| OPERATORS.contains(next));
                "operator"
            }
            _ => {
                stream.eat_while(identifier);
                let word = stream.current().to_lowercase();
                STYLE.get(word.as_str()).copied().unwrap_or("variable")
            }
        })
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> &'static str {
        self.0 = if string_continues(stream, quote) {
            Scan::String(quote)
        } else {
            Scan::Base
        };
        "string"
    }

    fn comment_paren(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        let mut star = false;
        for ch in stream {
            if ch == ')' && star {
                self.0 = Scan::Base;
                return "comment";
            }
            star = ch == '*';
        }
        self.0 = Scan::CommentParen;
        "comment"
    }

    fn comment_brace(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        stream.eat_while(|ch| ch != '}');
        self.0 = if stream.eat('}').is_some() {
            Scan::Base
        } else {
            Scan::CommentBrace
        };
        "comment"
    }
}
