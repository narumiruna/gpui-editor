use super::{JS_WS, State, ascii_word, js_space};
use crate::stream_parser::StringStream;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Sof,
    Unknown,
    Punct(char),
    Arrow,
    Number,
    String,
    JsonLdKeyword,
    Regexp,
    Quasi,
    Operator,
    IncDec,
    Spread,
    Meta,
    Comment,
    Variable,
    Atom,
    If,
    KeywordA,
    KeywordB,
    KeywordC,
    KeywordD,
    New,
    Debugger,
    Var,
    Function,
    Catch,
    For,
    Switch,
    Case,
    Default,
    This,
    Class,
    Export,
    Import,
    Async,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Scan {
    Base,
    String(char),
    Comment,
    Quasi,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Token<'line> {
    pub kind: Kind,
    pub style: Option<&'static str>,
    pub value: &'line str,
}

impl State {
    pub(super) fn scan<'line>(&mut self, stream: &mut StringStream<'line>) -> Token<'line> {
        use Kind::*;
        let (kind, style) = match self.scan {
            Scan::Base => self.base(stream),
            Scan::String(quote) => self.string(stream, quote),
            Scan::Comment => self.comment(stream),
            Scan::Quasi => self.quasi(stream),
        };
        let value = match kind {
            Variable if style == Some("property") => "",
            Variable | Operator | Quasi | Atom | If | KeywordA | KeywordB | KeywordC | KeywordD
            | New | Debugger | Var | Function | Catch | For | Switch | Case | Default | This
            | Class | Export | Import | Async => stream.current(),
            _ => "",
        };
        Token { kind, style, value }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        use Kind::*;
        let Some(ch) = stream.next() else {
            return (Unknown, None);
        };
        match ch {
            '"' | '\'' => {
                self.scan = Scan::String(ch);
                self.string(stream, ch)
            }
            '.' if stream
                .match_regex(re!(r"^[0-9][0-9_]*(?:[eE][+\-]?[0-9_]+)?"), true)
                .is_some() =>
            {
                (Number, Some("number"))
            }
            '.' if stream.match_str("..", true, false) => (Spread, Some("meta")),
            '[' | ']' | '{' | '}' | '(' | ')' | ',' | ';' | ':' | '.' => (Punct(ch), None),
            '=' if stream.eat('>').is_some() => (Arrow, Some("operator")),
            '0' if stream
                .match_regex(re!(r"^(?:x[0-9A-Fa-f_]+|o[0-7_]+|b[01_]+)n?"), true)
                .is_some() =>
            {
                (Number, Some("number"))
            }
            '0'..='9' => {
                stream.match_regex(
                    re!(r"^[0-9_]*(?:n|(?:\.[0-9_]*)?(?:[eE][+\-]?[0-9_]+)?)?"),
                    true,
                );
                (Number, Some("number"))
            }
            '/' if stream.eat('*').is_some() => {
                self.scan = Scan::Comment;
                self.comment(stream)
            }
            '/' if stream.eat('/').is_some() => {
                stream.skip_to_end();
                (Comment, Some("comment"))
            }
            '/' if self.expression_allowed(stream) => {
                regexp(stream);
                (Regexp, Some("string.special"))
            }
            '/' => {
                stream.eat('=');
                (Operator, Some("operator"))
            }
            '`' => {
                self.scan = Scan::Quasi;
                self.quasi(stream)
            }
            '#' if stream.peek() == Some('!') => {
                stream.skip_to_end();
                (Meta, Some("meta"))
            }
            '#' if stream.eat_while(word) => (Variable, Some("property")),
            '<' if stream.match_str("!--", true, false) => {
                stream.skip_to_end();
                (Comment, Some("comment"))
            }
            '-' if stream.match_str("->", true, false)
                && stream.line()[..stream.start()].chars().all(js_space) =>
            {
                stream.skip_to_end();
                (Comment, Some("comment"))
            }
            ch if operator(ch) => {
                if ch != '>' || self.lexical().kind != super::Lex::Bracket('>') {
                    if stream.eat('=').is_some() {
                        if matches!(ch, '!' | '=') {
                            stream.eat('=');
                        }
                    } else if matches!(ch, '<' | '>' | '*' | '+' | '-' | '|' | '&' | '?') {
                        stream.eat(ch);
                        if ch == '>' {
                            stream.eat(ch);
                        }
                    }
                }
                if ch == '?' && stream.eat('.').is_some() {
                    (Punct('.'), None)
                } else {
                    (Operator, Some("operator"))
                }
            }
            ch if word(ch) => {
                stream.eat_while(word);
                if self.last != Punct('.') {
                    let value = stream.current();
                    if let Some((kind, style)) = keyword(value) {
                        return (kind, Some(style));
                    }
                    if value == "async" && async_prefix(stream) {
                        return (Async, Some("keyword"));
                    }
                }
                (Variable, Some("variable"))
            }
            // Do not reuse mutable token scratch data from another stream.
            _ => (Unknown, None),
        }
    }

    fn string(
        &mut self,
        stream: &mut StringStream<'_>,
        quote: char,
    ) -> (Kind, Option<&'static str>) {
        if self.json_ld() && stream.match_regex(re!(r#"^@(context|id|value|language|type|container|list|set|reverse|index|base|vocab|graph)""#), true).is_some() {
            self.scan = Scan::Base;
            return (Kind::JsonLdKeyword, Some("meta"));
        }
        let mut escaped = false;
        for ch in stream {
            if ch == quote && !escaped {
                break;
            }
            escaped = !escaped && ch == '\\';
        }
        if !escaped {
            self.scan = Scan::Base;
        }
        (Kind::String, Some("string"))
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        if stream.skip_to("*/") {
            stream.match_str("*/", true, false);
            self.scan = Scan::Base;
        } else {
            stream.skip_to_end();
        }
        (Kind::Comment, Some("comment"))
    }

    fn quasi(&mut self, stream: &mut StringStream<'_>) -> (Kind, Option<&'static str>) {
        let mut escaped = false;
        while let Some(ch) = stream.next() {
            if !escaped && (ch == '`' || ch == '$' && stream.eat('{').is_some()) {
                self.scan = Scan::Base;
                break;
            }
            escaped = !escaped && ch == '\\';
        }
        (Kind::Quasi, Some("string.special"))
    }

    fn expression_allowed(&self, stream: &StringStream<'_>) -> bool {
        use Kind::*;
        matches!(
            self.last,
            Operator
                | Sof
                | KeywordB
                | KeywordC
                | KeywordD
                | Case
                | New
                | Export
                | Default
                | Spread
                | Punct('[' | '{' | '}' | '(' | ',' | ';' | ':')
                | Arrow
        ) || self.last == Quasi
            && stream.line()[..stream.pos() - 1]
                .trim_end_matches(js_space)
                .ends_with('{')
    }

    pub(super) fn find_arrow(&mut self, stream: &StringStream<'_>) {
        if self.fat_arrow != Some(0) {
            self.fat_arrow = None;
        }
        let start = stream.start();
        let line = stream.line();
        let Some(found) = line[start..].find("=>") else {
            return;
        };
        let mut arrow = start + found;
        if self.ts() {
            let prefix = &line[start..arrow];
            let suffix = prefix.trim_end_matches(js_space);
            if let Some(matched) = re!(format!(
                r":{JS_WS}*(?:[A-Za-z0-9_]+(?:<[^>]*>|\[\])?|\{{[^}}]*\}})$"
            ))
            .find(suffix)
            {
                // Upstream uses the match index relative to start in this case.
                arrow = matched.start();
            }
        }
        let Some(before) = line.get(..arrow) else {
            return;
        };
        let mut chars = before.char_indices().rev();
        let mut depth = 0;
        let mut seen = false;
        let mut offset = None;
        while let Some((index, ch)) = chars.next() {
            if matches!(ch, '(' | '[' | '{') {
                if depth == 0 {
                    offset = Some(index + 1);
                    break;
                }
                depth -= 1;
                if depth == 0 {
                    seen |= ch == '(';
                    offset = Some(index);
                    break;
                }
            } else if matches!(ch, ')' | ']' | '}') {
                depth += 1;
            } else if word(ch) {
                seen = true;
            } else if matches!(ch, '"' | '\'' | '/' | '`') {
                loop {
                    let Some((at, inner)) = chars.next() else {
                        return;
                    };
                    if inner == ch && !line[..at].ends_with('\\') {
                        break;
                    }
                }
            } else if seen && depth == 0 {
                offset = Some(index + ch.len_utf8());
                break;
            }
        }
        if seen && depth == 0 {
            self.fat_arrow = offset;
        }
    }
}

fn regexp(stream: &mut StringStream<'_>) {
    let mut escaped = false;
    let mut in_set = false;
    for ch in &mut *stream {
        if !escaped {
            match ch {
                '/' if !in_set => break,
                '[' => in_set = true,
                ']' => in_set = false,
                _ => {}
            }
        }
        escaped = !escaped && ch == '\\';
    }
    let rest = &stream.line()[stream.pos()..];
    let size = rest.bytes().take_while(|ch| b"gimyus".contains(ch)).count();
    let flags = &rest[..size];
    if size > 0
        && !rest[size..].starts_with(ascii_word)
        && !stream.line()[..stream.pos()].ends_with(ascii_word)
        && !flags
            .bytes()
            .enumerate()
            .any(|(i, ch)| flags.as_bytes()[i + 1..].contains(&ch))
    {
        stream.set_pos(stream.pos() + size).unwrap();
    }
}

fn async_prefix(stream: &StringStream<'_>) -> bool {
    let mut rest = &stream.line()[stream.pos()..];
    loop {
        rest = rest.trim_start_matches(js_space);
        if let Some(tail) = rest
            .strip_prefix("/*")
            .and_then(|tail| tail.split_once("*/"))
        {
            rest = tail.1;
        } else {
            return rest.starts_with(|ch| matches!(ch, '[' | '(') || ascii_word(ch));
        }
    }
}

fn keyword(value: &str) -> Option<(Kind, &'static str)> {
    use Kind::*;
    Some((
        match value {
            "if" => If,
            "while" | "with" => KeywordA,
            "else" | "do" | "try" | "finally" => KeywordB,
            "return" | "break" | "continue" => KeywordD,
            "delete" | "void" | "throw" | "yield" | "extends" | "await" => KeywordC,
            "new" => New,
            "debugger" => Debugger,
            "var" | "const" | "let" => Var,
            "function" => Function,
            "catch" => Catch,
            "for" => For,
            "switch" => Switch,
            "case" => Case,
            "default" => Default,
            "in" | "typeof" | "instanceof" => Operator,
            "true" | "false" | "null" | "undefined" | "NaN" | "Infinity" => {
                return Some((Atom, "atom"));
            }
            "this" => This,
            "class" => Class,
            "super" => Atom,
            "export" => Export,
            "import" => Import,
            _ => return None,
        },
        "keyword",
    ))
}

pub(super) fn operator(ch: char) -> bool {
    "+-*&%=<>!?|~^@".contains(ch)
}

pub(super) fn word(ch: char) -> bool {
    ascii_word(ch) || ch == '$' || ch >= '\u{a1}'
}
