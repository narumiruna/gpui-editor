use std::{collections::HashMap, sync::LazyLock};

use super::{LegacyMode, ascii_word, remaining};
use crate::stream_parser::StringStream;

#[derive(Clone, Copy, Debug)]
pub(super) enum Dialect {
    OCaml,
    FSharp,
    Sml,
}

impl Dialect {
    pub(super) fn from_mode(mode: LegacyMode) -> Self {
        match mode {
            LegacyMode::OCaml => Self::OCaml,
            LegacyMode::FSharp => Self::FSharp,
            LegacyMode::Sml => Self::Sml,
            _ => unreachable!(),
        }
    }

    pub(super) fn hints(self) -> &'static [&'static str] {
        &VOCABULARIES[self as usize].hints
    }

    fn style(self, word: &str) -> &'static str {
        VOCABULARIES[self as usize]
            .styles
            .get(word)
            .copied()
            .unwrap_or("variable")
    }
}

#[derive(Clone, Debug)]
pub(super) struct State {
    dialect: Dialect,
    scan: Scan,
}

#[derive(Clone, Copy, Debug)]
enum Scan {
    Normal,
    Comment(usize),
    String,
    LongString,
}

impl State {
    pub(super) fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            scan: Scan::Normal,
        }
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match self.dialect {
            Dialect::OCaml => LegacyMode::OCaml,
            Dialect::FSharp => LegacyMode::FSharp,
            Dialect::Sml => LegacyMode::Sml,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match self.scan {
            Scan::Normal => self.normal(stream),
            Scan::Comment(depth) => Some(self.comment(stream, depth)),
            Scan::String => Some(self.string(stream)),
            Scan::LongString => Some(self.long_string(stream)),
        }
    }

    fn normal(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let style = match stream.next()? {
            '"' => self.string(stream),
            '{' if stream.eat('|').is_some() => self.long_string(stream),
            '(' if remaining(stream).starts_with('*') && !remaining(stream).starts_with("*)") => {
                stream.next();
                self.comment(stream, 1)
            }
            '~' | '?' => {
                stream.eat_while(ascii_word);
                "variableName.special"
            }
            '`' => {
                stream.eat_while(ascii_word);
                "quote"
            }
            '/' if !matches!(self.dialect, Dialect::OCaml) && stream.eat('/').is_some() => {
                stream.skip_to_end();
                "comment"
            }
            ch if ch.is_ascii_digit() => {
                number(stream, ch);
                "number"
            }
            ch if "+-*&%=<>!?|@.~:".contains(ch) => "operator",
            ch if is_identifier(ch) => {
                stream.eat_while(is_identifier);
                self.dialect.style(stream.current())
            }
            _ => return None,
        };
        Some(style)
    }

    fn string(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        let mut escaped = false;
        self.scan = Scan::String;
        for ch in stream {
            if ch == '"' && !escaped {
                self.scan = Scan::Normal;
                break;
            }
            escaped = !escaped && ch == '\\';
        }
        "string"
    }

    fn long_string(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        self.scan = Scan::LongString;
        let mut prev = None;
        for ch in stream {
            if prev == Some('|') && ch == '}' {
                self.scan = Scan::Normal;
                break;
            }
            prev = Some(ch);
        }
        "string"
    }

    fn comment(&mut self, stream: &mut StringStream<'_>, mut depth: usize) -> &'static str {
        let mut prev = None;
        // The delimiter pairs may overlap, including an inner `(*)`.
        for ch in stream {
            match (prev, ch) {
                (Some('('), '*') => depth += 1,
                (Some('*'), ')') => {
                    depth -= 1;
                    if depth == 0 {
                        self.scan = Scan::Normal;
                        return "comment";
                    }
                }
                _ => {}
            }
            prev = Some(ch);
        }
        self.scan = Scan::Comment(depth);
        "comment"
    }
}

fn is_identifier(ch: char) -> bool {
    ascii_word(ch) || ch >= '\u{a1}'
}

fn number(stream: &mut StringStream<'_>, first: char) {
    if first == '0' && stream.eat_if(|ch| matches!(ch, 'b' | 'B')).is_some() {
        stream.eat_while(|ch| matches!(ch, '0' | '1'));
    }
    if first == '0' && stream.eat_if(|ch| matches!(ch, 'x' | 'X')).is_some() {
        stream.eat_while(|ch| ch.is_ascii_hexdigit());
    }
    if first == '0' && stream.eat_if(|ch| matches!(ch, 'o' | 'O')).is_some() {
        stream.eat_while(|ch| matches!(ch, '0'..='7'));
    } else {
        stream.eat_while(|ch| ch.is_ascii_digit() || ch == '_');
        if stream.eat('.').is_some() {
            stream.eat_while(|ch| ch.is_ascii_digit());
        }
        if stream.eat_if(|ch| matches!(ch, 'e' | 'E')).is_some() {
            stream.eat_while(|ch| ch.is_ascii_digit() || matches!(ch, '-' | '+'));
        }
    }
}

struct Vocabulary {
    styles: HashMap<&'static str, &'static str>,
    hints: Vec<&'static str>,
}

impl Vocabulary {
    fn new(extra: &[(&'static str, &'static str)]) -> Self {
        let groups = [("keyword", COMMON)]
            .into_iter()
            .chain(extra.iter().copied());
        let words = groups.flat_map(|(style, words)| {
            words
                .split_ascii_whitespace()
                .map(move |word| (word, style))
        });
        let mut styles = HashMap::new();
        let mut hints = Vec::new();
        for (word, style) in words {
            if styles.insert(word, style).is_none() {
                hints.push(word);
            }
        }
        Self { styles, hints }
    }
}

static VOCABULARIES: LazyLock<[Vocabulary; 3]> =
    LazyLock::new(|| [OCAML, F_SHARP, SML].map(Vocabulary::new));

const COMMON: &str = "as do else end exception fun functor if in include let of open rec struct then type val while with";

const OCAML: &[(&str, &str)] = &[
    ("keyword", "and assert begin class constraint done downto external function initializer lazy
                  match method module mutable new nonrec object private sig to try value virtual when"),
    ("builtin", "raise failwith true false asr land lor lsl lsr lxor mod or raise_notrace trace exit
                  print_string print_endline"),
    ("type", "int float bool char string unit"),
    ("builtin", "List"),
];

const F_SHARP: &[(&str, &str)] = &[
    ("keyword", "abstract assert base begin class default delegate do! done downcast downto elif
                  extern finally for function global inherit inline interface internal lazy let! match
                  member module mutable namespace new null override private public return! return
                  select static to try upcast use! use void when yield! yield atomic break checked
                  component const constraint constructor continue eager event external fixed method
                  mixin object parallel process protected pure sealed tailcall trait virtual volatile"),
    ("builtin", "List Seq Map Set Option int string not true false raise failwith"),
];

const SML: &[(&str, &str)] = &[
    (
        "keyword",
        "abstype and andalso case datatype fn handle infix infixr local nonfix op orelse
                  raise withtype eqtype sharing sig signature structure where true false",
    ),
    ("builtin", "int real string char bool"),
];
