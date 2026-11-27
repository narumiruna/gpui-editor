use super::{StringStream, closes_comment, digit, until_unescaped, word};

mod parser;

#[derive(Clone, Debug)]
pub(super) struct State {
    scan: Scan,
    regexp_allowed: bool,
    keyword_allowed: bool,
    parser: parser::State,
}

#[derive(Clone, Copy, Debug)]
enum Scan {
    Base,
    String(char),
    Comment,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Punct(char),
    Comment,
    Number,
    String,
    Regexp,
    Operator,
    Meta,
    Type,
    Variable,
    Atom,
    Attribute,
    Conditional,
    Form,
    Prefix,
    Var,
    Function,
    For,
    Switch,
    Case,
    Default,
    Catch,
    Import,
    TypeDef,
    OtherKeyword,
}

impl Kind {
    fn atomic(self) -> bool {
        matches!(
            self,
            Self::Atom | Self::Number | Self::Variable | Self::String | Self::Regexp
        )
    }
}

impl State {
    pub(super) fn new(indent_unit: usize) -> Self {
        Self {
            scan: Scan::Base,
            regexp_allowed: true,
            keyword_allowed: true,
            parser: parser::State::new(indent_unit),
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.parser.start_line(stream.indentation());
        }
        if stream.eat_space() || stream.eol() {
            return None;
        }
        let (kind, style) = match self.scan {
            Scan::Base => self.base(stream),
            Scan::String(quote) => self.string(stream, quote),
            Scan::Comment => self.comment(stream),
        };
        if kind == Kind::Comment {
            return style;
        }
        self.regexp_allowed = matches!(
            kind,
            Kind::Operator | Kind::Prefix | Kind::Punct('[' | '{' | '}' | '(' | ',' | ';' | ':')
        );
        self.keyword_allowed = kind != Kind::Punct('.');
        self.parser
            .parse(kind, stream.current(), stream.column())
            .or(style)
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> usize {
        if matches!(self.scan, Scan::Base) {
            self.parser.indent(text_after, unit)
        } else {
            0
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        let ch = stream.next().expect("a nonempty stream");
        match ch {
            '"' | '\'' => self.string(stream, ch),
            ch if "[]{}(),;:.".contains(ch) => (Kind::Punct(ch), None),
            '0' if stream.eat_if(|ch| ch.eq_ignore_ascii_case(&'x')).is_some() => {
                stream.eat_while(|ch| ch.is_ascii_hexdigit());
                (Kind::Number, Some("number"))
            }
            ch if digit(ch) || ch == '-' && stream.eat_if(digit).is_some() => {
                number_tail(stream);
                (Kind::Number, Some("number"))
            }
            '~' if self.regexp_allowed && stream.eat('/').is_some() => {
                until_unescaped(stream, '/');
                stream.eat_while(|ch| "gimsu".contains(ch));
                (Kind::Regexp, Some("string.special"))
            }
            '/' if stream.eat('*').is_some() => self.comment(stream),
            '/' if stream.eat('/').is_some() => {
                stream.skip_to_end();
                (Kind::Comment, Some("comment"))
            }
            '#' => {
                stream.skip_to_end();
                (Kind::Meta, Some("meta"))
            }
            '@' => {
                stream.eat(':');
                stream.eat_while(word);
                (Kind::Meta, Some("meta"))
            }
            ch if ch == '/' || operator(ch) => {
                stream.eat_while(operator);
                (Kind::Operator, None)
            }
            ch if ch.is_ascii_uppercase() => {
                stream.eat_while(|ch| word(ch) || matches!(ch, '<' | '>'));
                (Kind::Type, Some("type"))
            }
            _ => {
                stream.eat_while(word);
                if self.keyword_allowed {
                    keyword(stream.current())
                } else {
                    (Kind::Variable, Some("variable"))
                }
            }
        }
    }

    fn string(
        &mut self,
        stream: &mut StringStream<'_>,
        quote: char,
    ) -> (Kind, Option<&'static str>) {
        self.scan = if until_unescaped(stream, quote) {
            Scan::Base
        } else {
            Scan::String(quote)
        };
        (Kind::String, Some("string"))
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        self.scan = if closes_comment(stream) {
            Scan::Base
        } else {
            Scan::Comment
        };
        (Kind::Comment, Some("comment"))
    }
}

fn operator(ch: char) -> bool {
    "+-*&%=<>!?|".contains(ch)
}

fn number_tail(stream: &mut StringStream<'_>) {
    stream.eat_while(digit);
    if stream.eat('.').is_some() {
        stream.eat_while(digit);
        if stream.peek() == Some('.') {
            // The fractional part must not end immediately before a dot.
            stream
                .back_up(1)
                .expect("the last digit or dot is one byte");
        }
    }
    let exponent = stream.pos();
    if stream.eat_if(|ch| matches!(ch, 'e' | 'E')).is_some() {
        stream.eat_if(|ch| matches!(ch, '+' | '-'));
        if !stream.eat_while(digit) {
            stream
                .set_pos(exponent)
                .expect("the exponent starts on a scalar boundary");
        }
    }
}

fn keyword(text: &str) -> (Kind, Option<&'static str>) {
    let kind = match text {
        "if" | "while" => Kind::Conditional,
        "else" | "do" | "try" => Kind::Form,
        "return" | "break" | "continue" | "new" | "throw" => Kind::Prefix,
        "var" => Kind::Var,
        "inline" | "static" | "public" | "private" => return (Kind::Attribute, Some("attribute")),
        "using" | "import" => Kind::Import,
        "cast" | "macro" | "untyped" | "callback" | "never" | "trace" => Kind::OtherKeyword,
        "function" => Kind::Function,
        "catch" => Kind::Catch,
        "for" => Kind::For,
        "switch" => Kind::Switch,
        "case" => Kind::Case,
        "default" => Kind::Default,
        "in" => Kind::Operator,
        "class" | "abstract" | "enum" | "interface" | "typedef" | "extends" | "implements"
        | "dynamic" => Kind::TypeDef,
        "true" | "false" | "null" => return (Kind::Atom, Some("atom")),
        _ => return (Kind::Variable, Some("variable")),
    };
    (kind, Some("keyword"))
}
