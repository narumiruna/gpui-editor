use std::sync::LazyLock;

use regex::Regex;

use super::{StringStream, ascii_word, eat_str, remaining, word_end};

pub(super) const KEYWORDS: &[&str] = &[
    "alias",
    "and",
    "BEGIN",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "defined?",
    "do",
    "else",
    "elsif",
    "END",
    "end",
    "ensure",
    "false",
    "for",
    "if",
    "in",
    "module",
    "next",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "self",
    "super",
    "then",
    "true",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
    "nil",
    "raise",
    "throw",
    "catch",
    "fail",
    "loop",
    "callcc",
    "caller",
    "lambda",
    "proc",
    "public",
    "protected",
    "private",
    "require",
    "load",
    "require_relative",
    "extend",
    "autoload",
    "__END__",
    "__FILE__",
    "__LINE__",
    "__dir__",
];
const INDENT_WORDS: &[&str] = &[
    "def", "class", "case", "for", "while", "until", "module", "catch", "loop", "proc", "begin",
];

static HERE_DOC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^<[-~][`"']?([a-zA-Z_?][a-zA-Z0-9_]*)[`"']?(?:;|$)"#).unwrap());
static NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9_]*(?:\.[0-9_]+)?(?:[eE][+-]?[0-9_]+)?").unwrap());

#[derive(Clone, Debug)]
pub(super) struct State {
    frames: Vec<Frame>,
    contexts: Vec<Context>,
    root_unit: usize,
    indented: usize,
    continued: bool,
    last: Option<&'static str>,
    variable_list: bool,
}

#[derive(Clone, Debug)]
enum Frame {
    Quoted(Quote),
    Brace(usize),
    Once(bool),
    HereDoc(String),
    BlockComment,
}

#[derive(Clone, Copy, Debug)]
struct Quote {
    close: char,
    style: &'static str,
    embed: bool,
    unescaped: bool,
}

#[derive(Clone, Copy, Debug)]
enum Context {
    Bracket(char, usize),
    Keyword(usize),
    Pause,
}

type Lex = (Option<&'static str>, Option<&'static str>);

fn styled(style: &'static str) -> Lex {
    (Some(style), None)
}

impl State {
    pub(super) fn new(unit: usize) -> Self {
        Self {
            frames: Vec::new(),
            contexts: Vec::new(),
            root_unit: unit,
            indented: 0,
            continued: false,
            last: None,
            variable_list: false,
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Lex {
        if stream.sol() && eat_str(stream, "=begin") && stream.eol() {
            self.frames.push(Frame::BlockComment);
            return styled("comment");
        }
        if stream.eat_space() {
            return (None, None);
        }
        let Some(ch) = stream.next() else {
            return (None, None);
        };
        match ch {
            '`' | '\'' | '"' => self.open_quote(stream, ch, "string", ch != '\'', false),
            '/' if regexp_ahead(remaining(stream)) => {
                self.open_quote(stream, ch, "string.special", true, false)
            }
            '/' => styled("operator"),
            '%' => {
                let modifier = stream.eat_if(|ch| "sWQrwxq".contains(ch));
                let style = match modifier {
                    Some('s') => "atom",
                    Some('r') => "string.special",
                    _ => "string",
                };
                let embed = !matches!(modifier, Some('w' | 'x' | 'q'));
                let Some(delimiter) =
                    stream.eat_if(|ch| !ascii_word(ch) && !super::js_space(ch) && ch != '=')
                else {
                    return styled("operator");
                };
                self.open_quote(stream, close(delimiter), style, embed, true)
            }
            '#' => {
                stream.skip_to_end();
                styled("comment")
            }
            '<' if let Some(matched) = stream.match_regex(&HERE_DOC, true) => {
                let phrase = matched.get(1).unwrap().as_str();
                if !here_doc_end(stream, phrase) {
                    self.frames.push(Frame::HereDoc(phrase.to_owned()));
                }
                styled("string")
            }
            '0' => {
                if stream.eat('x').is_some() {
                    stream.eat_while(|ch| ch.is_ascii_hexdigit());
                } else if stream.eat('b').is_some() {
                    stream.eat_while(|ch| matches!(ch, '0' | '1'));
                } else if stream.eat('.').is_some() {
                    // The upstream single-character test accepts an empty regex match.
                    stream.next();
                } else {
                    stream.eat_while(|ch| ('0'..='7').contains(&ch));
                }
                styled("number")
            }
            ch if ch.is_ascii_digit() => {
                stream.match_regex(&NUMBER, true);
                styled("number")
            }
            '?' => {
                while eat_str(stream, "\\C-") || eat_str(stream, "\\M-") {}
                if stream.eat('\\').is_some() {
                    stream.eat_while(ascii_word);
                } else {
                    stream.next();
                }
                styled("string")
            }
            ':' => {
                if let Some(quote) = stream.eat_if(|ch| matches!(ch, '\'' | '"')) {
                    return self.open_quote(stream, quote, "atom", quote == '"', false);
                }
                if stream.eat_if(|ch| matches!(ch, '<' | '>')).is_some() {
                    stream.eat_if(|ch| matches!(ch, '<' | '>'));
                    return styled("atom");
                }
                if stream.eat_if(|ch| "+-*/&|:!".contains(ch)).is_some() {
                    return styled("atom");
                }
                if stream
                    .eat_if(|ch| name_start(ch) || matches!(ch, '$' | '@'))
                    .is_some()
                {
                    stream.eat_while(|ch| name_char(ch) || ch == '$');
                    stream.eat_if(|ch| "?!=".contains(ch));
                    return styled("atom");
                }
                styled("operator")
            }
            '@' if remaining(stream)
                .strip_prefix('@')
                .unwrap_or(remaining(stream))
                .starts_with(name_start) =>
            {
                // The prefix check also consumes the first letter upstream.
                stream.eat('@');
                stream.eat_while(name_char);
                styled("propertyName")
            }
            '$' => {
                if stream
                    .eat_if(|ch| ch.is_ascii_alphabetic() || ch == '_')
                    .is_some()
                {
                    stream.eat_while(ascii_word);
                } else if stream.eat_if(|ch| ch.is_ascii_digit()).is_some() {
                    stream.eat_if(|ch| ch.is_ascii_digit());
                } else {
                    stream.next();
                }
                styled("variableName.special")
            }
            ch if name_start(ch) => {
                stream.eat_while(name_char);
                stream.eat_if(|ch| matches!(ch, '?' | '!'));
                if stream.eat(':').is_some() {
                    styled("atom")
                } else {
                    styled("variable")
                }
            }
            '|' if self.variable_list || matches!(self.last, Some("{" | "do")) => (None, Some("|")),
            '(' => (None, Some("(")),
            ')' => (None, Some(")")),
            '[' => (None, Some("[")),
            ']' => (None, Some("]")),
            '{' => (None, Some("{")),
            '}' => (None, Some("}")),
            '\\' => (None, Some("\\")),
            ';' => (None, Some(";")),
            '-' if stream.eat('>').is_some() => styled("operator"),
            ch if operator(ch) => {
                let more = stream.eat_while(operator);
                (Some("operator"), (ch == '.' && !more).then_some("."))
            }
            _ => (None, None),
        }
    }

    fn open_quote(
        &mut self,
        stream: &mut StringStream<'_>,
        close: char,
        style: &'static str,
        embed: bool,
        unescaped: bool,
    ) -> Lex {
        let quote = Quote {
            close,
            style,
            embed,
            unescaped,
        };
        self.frames.push(Frame::Quoted(quote));
        self.quoted(stream, quote)
    }

    fn quoted(&mut self, stream: &mut StringStream<'_>, quote: Quote) -> Lex {
        if matches!(self.contexts.last(), Some(Context::Pause)) {
            self.contexts.pop();
            stream.eat('}');
        }
        let mut escaped = false;
        while let Some(ch) = stream.next() {
            if ch == quote.close && (quote.unescaped || !escaped) {
                self.frames.pop();
                break;
            }
            if quote.embed && ch == '#' && !escaped {
                if stream.eat('{').is_some() {
                    if quote.close == '}' {
                        self.contexts.push(Context::Pause);
                    }
                    self.frames.push(Frame::Brace(1));
                    break;
                }
                if matches!(stream.peek(), Some('@' | '$')) {
                    self.frames.push(Frame::Once(false));
                    break;
                }
            }
            escaped = !escaped && ch == '\\';
        }
        styled(quote.style)
    }

    fn lex(&mut self, stream: &mut StringStream<'_>) -> Lex {
        loop {
            match self.frames.last_mut() {
                None => return self.base(stream),
                Some(Frame::Quoted(quote)) => {
                    let quote = *quote;
                    return self.quoted(stream, quote);
                }
                Some(Frame::Brace(depth)) => match stream.peek() {
                    Some('}') if *depth == 1 => {
                        self.frames.pop();
                    }
                    next => {
                        match next {
                            Some('{') => *depth += 1,
                            Some('}') => *depth -= 1,
                            _ => (),
                        }
                        return self.base(stream);
                    }
                },
                Some(Frame::Once(called)) => {
                    if *called {
                        self.frames.pop();
                    } else {
                        *called = true;
                        return self.base(stream);
                    }
                }
                Some(Frame::HereDoc(phrase)) => {
                    if here_doc_end(stream, phrase) {
                        self.frames.pop();
                    }
                    return styled("string");
                }
                Some(Frame::BlockComment) => {
                    if stream.sol() && eat_str(stream, "=end") && stream.eol() {
                        self.frames.pop();
                    }
                    stream.skip_to_end();
                    return styled("comment");
                }
            }
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.indented = stream.indentation();
        }
        let (mut style, punc) = self.lex(stream);
        let mut this_token = punc;
        let mut indent = false;
        let mut dedent = false;
        if style == Some("variable") {
            let word = stream.current();
            style = Some(if self.last == Some(".") {
                "property"
            } else if let Some(kw) = KEYWORDS.iter().copied().find(|&kw| kw == word) {
                this_token = Some(kw);
                indent = INDENT_WORDS.contains(&kw)
                    || (matches!(kw, "if" | "unless") && stream.column() == stream.indentation())
                    || (kw == "do"
                        && match self.contexts.last() {
                            Some(Context::Bracket(_, col) | Context::Keyword(col)) => {
                                *col < self.indented
                            }
                            Some(Context::Pause) => false,
                            None => self.root_unit > 0 || self.indented > 0,
                        });
                dedent = matches!(kw, "end" | "until");
                "keyword"
            } else if word.starts_with(|ch: char| ch.is_ascii_uppercase()) {
                "tag"
            } else if self.variable_list || matches!(self.last, Some("def" | "class")) {
                "def"
            } else {
                "variable"
            });
        }
        if punc.is_some() || style.is_some_and(|s| s != "comment") {
            self.last = this_token;
        }
        if punc == Some("|") {
            self.variable_list = !self.variable_list;
        }
        if indent {
            self.contexts.push(Context::Keyword(self.indented));
        } else if let Some("(" | "[" | "{") = punc {
            self.contexts.push(Context::Bracket(
                close(punc.unwrap().chars().next().unwrap()),
                self.indented,
            ));
        } else if dedent || matches!(punc, Some(")" | "]" | "}")) {
            self.contexts.pop();
        }
        if stream.eol() {
            self.continued = punc == Some("\\") || style == Some("operator");
        }
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        if !self.frames.is_empty() {
            return None;
        }
        let continued = if self.continued { unit } else { 0 };
        let (indented, closed) = match self.contexts.last() {
            Some(Context::Bracket(close, indented)) => (*indented, text_after.starts_with(*close)),
            Some(Context::Keyword(indented)) => (
                *indented,
                ["end", "until", "else", "elsif", "when", "rescue"]
                    .iter()
                    .any(|word| text_after.strip_prefix(word).is_some_and(word_end)),
            ),
            Some(Context::Pause) => return None,
            None => {
                return Some(
                    unit.saturating_add(continued)
                        .saturating_sub(self.root_unit),
                );
            }
        };
        Some(
            indented
                .saturating_add(if closed { 0 } else { unit })
                .saturating_add(continued),
        )
    }
}

fn here_doc_end(stream: &mut StringStream<'_>, phrase: &str) -> bool {
    stream.eat_space();
    if eat_str(stream, phrase) {
        true
    } else {
        stream.skip_to_end();
        false
    }
}

fn close(ch: char) -> char {
    match ch {
        '[' => ']',
        '{' => '}',
        '(' => ')',
        _ => ch,
    }
}

fn operator(ch: char) -> bool {
    "=+-/*:.^%<>~|".contains(ch)
}

fn name_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || ch >= '\u{00A1}'
}

fn name_char(ch: char) -> bool {
    name_start(ch) || ch.is_ascii_digit()
}

fn regexp_ahead(text: &str) -> bool {
    let mut depth = 0usize;
    let mut escaped = false;
    for ch in text.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' if depth == 0 => return false,
            ')' | ']' | '}' => depth -= 1,
            '/' if depth == 0 => return true,
            '\\' => escaped = true,
            _ => (),
        }
    }
    false
}
