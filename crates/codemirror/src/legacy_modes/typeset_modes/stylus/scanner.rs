use crate::stream_parser::StringStream;

use super::{State, is_tag, match_pattern, rest, vendor_pattern};
use crate::legacy_modes::js_space;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Scanner {
    Base,
    Comment,
    String(char),
    Parenthesized,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Kind {
    Other,
    Comment,
    String,
    Def,
    Hash,
    Unit,
    Qualifier,
    Word,
    Mixin,
    BlockMixin,
    Reference,
    Operator,
    VariableName,
    VendorPrefix,
    Mark(char),
}

pub(super) type Style = Option<&'static str>;
pub(super) struct Token(pub(super) Kind, pub(super) Style);

impl State {
    pub(super) fn scan(&mut self, stream: &mut StringStream<'_>) -> Token {
        match self.scanner {
            Scanner::Base => self.scan_base(stream),
            Scanner::Comment => self.scan_comment(stream),
            Scanner::String(quote) => self.scan_string(stream, quote),
            Scanner::Parenthesized => {
                stream.next();
                self.scanner = if rest(stream)
                    .trim_start_matches(js_space)
                    .starts_with(['"', '\'', ')'])
                {
                    Scanner::Base
                } else {
                    Scanner::String(')')
                };
                Token(Kind::Mark('('), None)
            }
        }
    }

    fn scan_base(&mut self, stream: &mut StringStream<'_>) -> Token {
        let pattern = pattern!(
            r"(?:^[\w-]+\s*=\s*$)|(?:^\s*[\w-]+\s*=\s*[\w-])|(?:^\s*(?:[.#@$&\[0-9+{>~/]|::?)?\s*[\w-]*(?:[a-z0-9*-]|/\*)(?:\(|,)?)"
        );
        let first_word = pattern.find(stream.line()).map_or("", |matched| {
            stream.line()[matched.range()].trim_start_matches(js_space)
        });
        let context = self.contexts.last_mut().unwrap();
        if context.first_word != first_word {
            first_word.clone_into(&mut context.first_word);
        }
        context.line_indent = stream.indentation();

        let ch = stream.peek();
        if stream.match_str("//", true, false) {
            stream.skip_to_end();
            return Token(Kind::Comment, Some("comment"));
        }
        if stream.match_str("/*", true, false) {
            self.scanner = Scanner::Comment;
            return self.scan_comment(stream);
        }
        if let Some(quote @ ('"' | '\'')) = ch {
            stream.next();
            self.scanner = Scanner::String(quote);
            return self.scan_string(stream, quote);
        }
        if stream.eat('@').is_some() {
            stream.eat_while(|ch| super::is_word(ch) || matches!(ch, '\\' | '-'));
            return Token(Kind::Def, Some("def"));
        }
        if stream.eat('#').is_some() {
            if match_pattern(
                stream,
                pattern!(r"^[0-9a-f]{3}(?:[0-9a-f](?:[0-9a-f]{2}){0,2})?\b(?!-)", "i"),
            ) {
                return Token(Kind::Other, Some("atom"));
            }
            if match_pattern(stream, pattern!(r"^[a-z][\w-]*", "i")) {
                return Token(Kind::Hash, Some("builtin"));
            }
        }
        if match_pattern(stream, vendor_pattern()) {
            return Token(Kind::VendorPrefix, Some("meta"));
        }
        if match_pattern(stream, pattern!(r"^-?[0-9]?\.?[0-9]")) {
            stream.eat_while(|ch| ch.is_ascii_alphabetic() || ch == '%');
            return Token(Kind::Unit, Some("number"));
        }
        if ch == Some('!') {
            stream.next();
            let keyword = match_pattern(stream, pattern!(r"^(?:important|optional)", "i"));
            return Token(
                Kind::Other,
                Some(if keyword { "keyword" } else { "operator" }),
            );
        }
        if ch == Some('.') && match_pattern(stream, pattern!(r"^\.[a-z][\w-]*", "i")) {
            return Token(Kind::Qualifier, Some("qualifier"));
        }
        if match_pattern(stream, pattern!(r"^(?:domain|regexp|url-prefix|url)\b")) {
            if stream.peek() == Some('(') {
                self.scanner = Scanner::Parenthesized;
            }
            return Token(Kind::Word, Some("property"));
        }
        if match_pattern(stream, pattern!(r"^[a-z][\w-]*\(", "i")) {
            stream.back_up(1).unwrap();
            return Token(Kind::Mixin, Some("keyword"));
        }
        if match_pattern(stream, pattern!(r"^[+-][a-z][\w-]*\(", "i")) {
            stream.back_up(1).unwrap();
            return Token(Kind::BlockMixin, Some("keyword"));
        }
        if stream.line().trim_start_matches(js_space).starts_with('&')
            && match_pattern(stream, pattern!(r"^[-_]+[a-z][\w-]*"))
        {
            return Token(Kind::Qualifier, Some("qualifier"));
        }
        if match_pattern(stream, pattern!(r"^[/&](?=[-_:.#a-z])"))
            || match_pattern(stream, pattern!(r"^&\s*$"))
        {
            return Token(Kind::Reference, Some("variableName.special"));
        }
        if match_pattern(
            stream,
            pattern!(r"^(?:in|and|or|not|is not|is a|is|isnt|defined|if unless)\b"),
        ) {
            return Token(Kind::Operator, Some("operator"));
        }
        if match_pattern(stream, pattern!(r"^\$?[-_]*[a-z0-9]+[\w-]*", "i")) {
            if pattern!(r#"^[.\[][\w'"\]-]+"#, "i")
                .find(rest(stream))
                .is_some()
                && !is_tag(stream.current())
            {
                stream.eat('.');
                return Token(Kind::VariableName, Some("variable"));
            }
            return Token(Kind::Word, Some("variable"));
        }
        if match_pattern(
            stream,
            pattern!(r"^\s*(?:\.{2,3}|&&|\|\||\*\*|[?!=:]?=|[-+*/%<>]=?|\?:|~)"),
        ) {
            let kind = match stream.current() {
                "=" => Kind::Mark('='),
                "*" => Kind::Mark('*'),
                _ => Kind::Other,
            };
            return Token(kind, Some("operator"));
        }
        if let Some(delimiter) = ch.filter(|ch| ":;,{}[]()".contains(*ch)) {
            stream.eat(delimiter);
            return Token(Kind::Mark(delimiter), None);
        }
        stream.next();
        Token(Kind::Other, None)
    }

    fn scan_comment(&mut self, stream: &mut StringStream<'_>) -> Token {
        if stream.skip_to("*/") {
            stream.match_str("*/", true, false);
            self.scanner = Scanner::Base;
        } else {
            stream.skip_to_end();
        }
        Token(Kind::Comment, Some("comment"))
    }

    fn scan_string(&mut self, stream: &mut StringStream<'_>, quote: char) -> Token {
        let mut escaped = false;
        while let Some(ch) = stream.next() {
            if ch == quote && !escaped {
                if quote == ')' {
                    stream.back_up(1).unwrap();
                }
                self.scanner = Scanner::Base;
                return Token(Kind::String, Some("string"));
            }
            escaped = !escaped && ch == '\\';
        }
        if !escaped && quote != ')' {
            self.scanner = Scanner::Base;
        }
        Token(Kind::String, Some("string"))
    }
}
