use std::sync::LazyLock;

use regex::Regex;

use super::{super::js_space, ascii_word, remaining};
use crate::stream_parser::StringStream;

mod stack;

use stack::Stack;

const OPEN: &[&str] = &["<<", "(", "[", "{"];
const CLOSE: &[&str] = &[">>", ")", "]", "}"];
const BLOCK_START: &[&str] = &["begin", "case", "fun", "if", "receive", "try"];
const CLAUSE_BLOCK: &[&str] = &["try", "if", "case", "receive"];

const GUARDS: &str = "is_atom is_binary is_bitstring is_boolean is_float is_function is_integer
                      is_list is_number is_pid is_port is_record is_reference is_tuple atom binary
                      bitstring boolean function integer list number pid port record reference tuple";
const BUILTINS: &str = "abs adler32 adler32_combine alive apply atom_to_binary atom_to_list
                        binary_to_atom binary_to_existing_atom binary_to_list binary_to_term bit_size
                        bitstring_to_list byte_size check_process_code contact_binary crc32
                        crc32_combine date decode_packet delete_module disconnect_node element erase
                        exit float float_to_list garbage_collect get get_keys group_leader halt hd
                        integer_to_list internal_bif iolist_size iolist_to_binary is_alive is_atom
                        is_binary is_bitstring is_boolean is_float is_function is_integer is_list
                        is_number is_pid is_port is_process_alive is_record is_reference is_tuple
                        length link list_to_atom list_to_binary list_to_bitstring list_to_existing_atom
                        list_to_float list_to_integer list_to_pid list_to_tuple load_module make_ref
                        module_loaded monitor_node node node_link node_unlink nodes notalive now
                        open_port pid_to_list port_close port_command port_connect port_control
                        pre_loaded process_flag process_info processes purge_module put register
                        registered round self setelement size spawn spawn_link spawn_monitor spawn_opt
                        split_binary statistics term_to_binary time throw tl trunc tuple_size
                        tuple_to_list unlink unregister whereis";

static ESCAPE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^(?:[0-7]{1,3}|[bdefnrstv\\"']|\^[a-zA-Z]|x[0-9a-zA-Z]{2}|x\{[0-9a-zA-Z]+\})"#)
        .expect("valid Erlang escape pattern")
});

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Atom,
    Attribute,
    Builtin,
    Close,
    Colon,
    Comment,
    Dot,
    Error,
    Fun,
    Function,
    Group,
    Guard,
    Keyword,
    Macro,
    Number,
    Open,
    Operator,
    Other,
    Record,
    Separator,
    Space,
    String,
    Type,
    Variable,
}

impl Kind {
    fn style(self) -> Option<&'static str> {
        Some(match self {
            Self::Atom => "atom",
            Self::Attribute => "attribute",
            Self::Builtin => "builtin",
            Self::Comment => "comment",
            Self::Error => "error",
            Self::Fun => "meta",
            Self::Function => "tag",
            Self::Guard => "property",
            Self::Keyword => "keyword",
            Self::Macro => "macroName",
            Self::Number => "number",
            Self::Operator => "operator",
            Self::Record => "bracket",
            Self::String => "string",
            Self::Type => "def",
            Self::Variable => "variable",
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    stack: Stack,
    quoted: Option<char>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eol() {
            return None;
        }
        let kind = self.scan(stream);
        self.stack.push(kind, stream);
        kind.style()
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        if self.quoted.is_some() {
            return None;
        }
        self.stack.indent(text_after, unit)
    }

    fn scan(&mut self, stream: &mut StringStream<'_>) -> Kind {
        if let Some(quote) = self.quoted {
            return self.quote(stream, quote);
        }
        if stream.eat_space() {
            return Kind::Space;
        }
        if self.stack.peek(1).is_none() && attribute(stream) {
            return match stream.current() {
                "-type" | "-spec" | "-export_type" | "-opaque" => Kind::Type,
                _ => Kind::Attribute,
            };
        }
        match stream.next().expect("stream is not empty") {
            '%' => {
                stream.skip_to_end();
                Kind::Comment
            }
            ':' => Kind::Colon,
            ch @ ('?' | '#') => {
                stream.eat_space();
                stream.eat_while(is_word);
                if ch == '?' { Kind::Macro } else { Kind::Record }
            }
            '$' => {
                if stream.next() == Some('\\') && stream.match_regex(&ESCAPE, true).is_none() {
                    Kind::Error
                } else {
                    Kind::Number
                }
            }
            '.' => Kind::Dot,
            '\'' => {
                self.quote(stream, '\'');
                if self.quoted.is_some() {
                    Kind::Atom
                } else if arity(stream) {
                    Kind::Fun
                } else if after(stream).starts_with(['(', ':']) {
                    Kind::Function
                } else {
                    Kind::Atom
                }
            }
            '"' => self.quote(stream, '"'),
            ch if is_upper(ch) || ch == '_' => {
                stream.eat_while(is_word);
                Kind::Variable
            }
            ch if is_lower(ch) => self.atom(stream),
            ch if ch.is_ascii_digit() => {
                number(stream);
                Kind::Number
            }
            _ if consume_symbol(stream, OPEN) => Kind::Open,
            _ if consume_symbol(stream, CLOSE) => Kind::Close,
            _ if consume_symbol(stream, &["->", ";", ","]) => Kind::Separator,
            _ if consume_symbol(
                stream,
                &[
                    "=:=", "=/=", ">=", "=<", "==", "/=", "||", "<-", "=", "+", "-", "*", "/", ">",
                    "<", "!",
                ],
            ) =>
            {
                Kind::Operator
            }
            _ => Kind::Other,
        }
    }

    fn quote(&mut self, stream: &mut StringStream<'_>, quote: char) -> Kind {
        self.quoted = Some(quote);
        while let Some(ch) = stream.next() {
            if ch == quote {
                self.quoted = None;
                break;
            }
            if ch == '\\' {
                stream.next();
            }
        }
        if quote == '"' {
            Kind::String
        } else {
            Kind::Atom
        }
    }

    fn atom(&self, stream: &mut StringStream<'_>) -> Kind {
        stream.eat_while(is_word);
        if arity(stream) {
            return Kind::Fun;
        }
        let word = stream.current();
        match word {
            "after" | "begin" | "catch" | "case" | "cond" | "end" | "fun" | "if" | "let" | "of"
            | "query" | "receive" | "try" | "when" => Kind::Keyword,
            "and" | "andalso" | "band" | "bnot" | "bor" | "bsl" | "bsr" | "bxor" | "div"
            | "not" | "or" | "orelse" | "rem" | "xor" => Kind::Operator,
            _ if after(stream).starts_with('(') => {
                if BUILTINS.split_ascii_whitespace().any(|w| w == word)
                    && (self.stack.peek(1) != Some(":") || self.stack.peek(2) == Some("erlang"))
                {
                    Kind::Builtin
                } else if GUARDS.split_ascii_whitespace().any(|w| w == word) {
                    Kind::Guard
                } else {
                    Kind::Function
                }
            }
            _ if after(stream).starts_with(':') => {
                if word == "erlang" {
                    Kind::Builtin
                } else {
                    Kind::Function
                }
            }
            _ => Kind::Atom,
        }
    }
}

fn is_upper(ch: char) -> bool {
    matches!(ch, 'A'..='Z' | 'Ø'..='Þ' | 'À'..='Ö')
}

fn is_lower(ch: char) -> bool {
    matches!(ch, 'a'..='z' | 'ß'..='ö' | 'ø'..='ÿ')
}

fn is_word(ch: char) -> bool {
    ch == '@' || ascii_word(ch) || is_upper(ch) || is_lower(ch)
}

fn trim_space(text: &str) -> &str {
    text.trim_start_matches(js_space)
}

fn after<'line>(stream: &StringStream<'line>) -> &'line str {
    trim_space(remaining(stream))
}

fn attribute(stream: &mut StringStream<'_>) -> bool {
    let text = remaining(stream);
    let Some(name) = text
        .strip_prefix('-')
        .map(trim_space)
        .filter(|s| s.starts_with(is_lower))
    else {
        return false;
    };
    let tail = name.trim_start_matches(|ch| is_word(ch) && ch != '@');
    consume(stream, text.len() - tail.len());
    true
}

fn arity(stream: &mut StringStream<'_>) -> bool {
    let Some(digits) = after(stream).strip_prefix('/').map(trim_space) else {
        return false;
    };
    if !digits.starts_with(|ch: char| ch.is_ascii_digit()) {
        return false;
    }
    consume(stream, remaining(stream).len() - digits.len() + 1);
    true
}

fn number(stream: &mut StringStream<'_>) {
    let digits = |ch: char| ch.is_ascii_digit();
    stream.eat_while(digits);
    if stream.eat('#').is_some() {
        if !stream.eat_while(|ch| ch.is_ascii_alphanumeric()) {
            stream.back_up(1).expect("radix marker is ASCII");
        }
    } else if stream.eat('.').is_some() {
        if !stream.eat_while(digits) {
            stream.back_up(1).expect("fraction marker is ASCII");
        } else {
            let end = stream.pos();
            if stream.eat_if(|ch| matches!(ch, 'e' | 'E')).is_some() {
                stream.eat_if(|ch| matches!(ch, '-' | '+'));
                if !stream.eat_while(digits) {
                    stream.set_pos(end).expect("known stream position");
                }
            }
        }
    }
}

// Candidates are ordered longest first, and include the scalar already read.
fn consume_symbol(stream: &mut StringStream<'_>, candidates: &[&str]) -> bool {
    let text = &stream.line()[stream.start()..];
    if let Some(candidate) = candidates
        .iter()
        .find(|&&candidate| text.starts_with(candidate))
    {
        consume(stream, candidate.len() - stream.current().len());
        true
    } else {
        false
    }
}

fn consume(stream: &mut StringStream<'_>, bytes: usize) {
    stream
        .set_pos(stream.pos() + bytes)
        .expect("matched text ends at a scalar boundary");
}
