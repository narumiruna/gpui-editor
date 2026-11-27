use super::{LegacyMode, js_space};
use crate::stream_parser::StringStream;

// ECMAScript whitespace and line terminators, for patterns that use \s.
const JS_WS: &str =
    r"[\t-\r \x{a0}\x{1680}\x{2000}-\x{200a}\x{2028}\x{2029}\x{202f}\x{205f}\x{3000}\x{feff}]";

macro_rules! re {
    ($pattern:expr) => {{
        static REGEX: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(&$pattern).unwrap());
        &*REGEX
    }};
}
mod combinators;
mod scanner;

use combinators::Rule;
use scanner::{Kind, Scan};

#[derive(Clone, Debug)]
pub(super) struct State {
    pub(super) mode: LegacyMode,
    scan: Scan,
    last: Kind,
    rules: Vec<Rule>,
    lexical: Vec<Lexical>,
    locals: Vec<String>,
    scopes: Vec<Scope>,
    indented: usize,
    fat_arrow: Option<usize>,
    start_unit: usize,
}

#[derive(Clone, Debug)]
struct Scope {
    vars: Vec<String>,
    block: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Lex {
    Block,
    Statement,
    Form,
    VarDef,
    Bracket(char),
}

#[derive(Clone, Copy, Debug)]
struct Lexical {
    kind: Lex,
    indented: usize,
    column: usize,
    align: Option<bool>,
    info: &'static str,
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self {
            mode,
            scan: Scan::Base,
            last: Kind::Sof,
            rules: Vec::new(),
            lexical: vec![Lexical {
                kind: Lex::Block,
                indented: 0,
                column: 0,
                align: Some(false),
                info: "",
            }],
            locals: Vec::new(),
            scopes: Vec::new(),
            indented: 0,
            fat_arrow: None,
            start_unit: indent_unit,
        }
    }

    fn json(&self) -> bool {
        self.mode == LegacyMode::Json || self.json_ld()
    }

    fn json_ld(&self) -> bool {
        self.mode == LegacyMode::JsonLd
    }

    fn ts(&self) -> bool {
        self.mode == LegacyMode::TypeScript
    }

    fn lexical(&self) -> &Lexical {
        self.lexical.last().unwrap()
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.lexical.last_mut().unwrap().align.get_or_insert(false);
            self.indented = stream.indentation();
            self.find_arrow(stream);
        }
        if self.scan != Scan::Comment && stream.eat_space() {
            return None;
        }
        let token = self.scan(stream);
        if token.kind == Kind::Comment {
            return token.style;
        }
        self.last = if token.kind == Kind::Operator && matches!(token.value, "++" | "--") {
            Kind::IncDec
        } else {
            token.kind
        };
        self.parse(stream, token)
    }

    pub(super) fn indent(&self, text: &str, unit: usize) -> Option<usize> {
        match self.scan {
            Scan::Comment | Scan::Quasi => return None,
            Scan::String(_) => return Some(0),
            Scan::Base => {}
        }
        let mut index = self.lexical.len() - 1;
        if !word_prefix(text.trim_start_matches(js_space), "else") {
            for rule in self.rules.iter().rev() {
                match rule {
                    Rule::PopLex => index = index.saturating_sub(1),
                    Rule::MaybeElse | Rule::PopScope => {}
                    _ => break,
                }
            }
        }
        let first = text.chars().next();
        let operator_pending = matches!(self.rules.last(), Some(Rule::MaybeOperator(_)));
        while index > 0
            && matches!(self.lexical[index].kind, Lex::Statement | Lex::Form)
            && (first == Some('}')
                || operator_pending && !text.starts_with(|ch| ",.=+-*:?[(".contains(ch)))
        {
            index -= 1;
        }
        let lex = self.lexical[index];
        let closing = matches!(lex.kind, Lex::Bracket(ch) if first == Some(ch));
        Some(match lex.kind {
            Lex::Block => unit.saturating_sub(self.start_unit),
            Lex::VarDef => {
                lex.indented
                    + if matches!(self.last, Kind::Operator | Kind::Punct(',')) {
                        lex.info.len() + 1
                    } else {
                        0
                    }
            }
            Lex::Form => lex.indented + if first == Some('{') { 0 } else { unit },
            Lex::Statement => {
                lex.indented
                    + if matches!(self.last, Kind::Operator | Kind::Punct(','))
                        || first.is_some_and(|ch| scanner::operator(ch) || matches!(ch, ',' | '.'))
                    {
                        unit
                    } else {
                        0
                    }
            }
            _ if lex.info == "switch" && !closing => {
                lex.indented
                    + if word_prefix(text, "case") || word_prefix(text, "default") {
                        unit
                    } else {
                        2 * unit
                    }
            }
            _ if lex.align == Some(true) => lex.column + usize::from(!closing),
            _ => lex.indented + if closing { 0 } else { unit },
        })
    }
}

fn ascii_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn word_prefix(text: &str, prefix: &str) -> bool {
    text.strip_prefix(prefix)
        .is_some_and(|tail| !tail.starts_with(ascii_word))
}
