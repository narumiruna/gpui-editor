use std::collections::BTreeMap;

use super::{LegacyCssHookToken, LegacyCssTokenHook, LegacyCssTokenKind, js_space};
use crate::stream_parser::StringStream;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Kind {
    #[default]
    Other,
    Comment,
    AtRule(AtRule),
    Mark(char),
    Hash,
    Word,
    Qualifier,
    Meta,
    Variable,
    VariableDefinition,
    Interpolation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum AtRule {
    Document,
    Block,
    Restricted(Restriction),
    Keyframes,
    Component,
    Other,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Restriction {
    #[default]
    None,
    FontFace,
    CounterStyle,
}

pub(super) type Style = Option<&'static str>;

#[derive(Clone, Copy, Debug)]
pub(super) struct Token(pub Kind, pub Style);

#[derive(Clone, Copy, Debug)]
enum Continuation {
    Comment,
    String(char),
    Parenthesized,
    Hook(fn(&mut StringStream<'_>) -> LegacyCssHookToken),
}

#[derive(Clone, Copy)]
pub(super) enum Hooks<'a> {
    Css,
    Scss,
    Less,
    Custom(&'a BTreeMap<char, LegacyCssTokenHook>),
}

#[derive(Clone, Debug, Default)]
pub(super) struct Scanner {
    continuation: Option<Continuation>,
    // Keep the upstream hyphen fallback local to this stream state.
    previous: Kind,
}

impl Scanner {
    // Whitespace outside a continuation does not reach the parser.
    pub(super) fn scan(
        &mut self,
        hooks: Hooks<'_>,
        stream: &mut StringStream<'_>,
    ) -> Option<Token> {
        let token = match self.continuation {
            Some(Continuation::Comment) => self.comment(stream),
            Some(Continuation::String(quote)) => self.string(stream, quote),
            Some(Continuation::Parenthesized) => {
                stream.next(); // The opening parenthesis.
                self.continuation = match after_space(stream).chars().next() {
                    Some('"' | '\'' | ')') => None,
                    _ => Some(Continuation::String(')')),
                };
                Token(Kind::Mark('('), None)
            }
            Some(Continuation::Hook(next)) => {
                let token = next(stream);
                self.hook_token(token, stream)
            }
            None if stream.eat_space() => return None,
            None => self.base(hooks, stream),
        };
        self.previous = token.0;
        Some(token)
    }

    fn base(&mut self, hooks: Hooks<'_>, stream: &mut StringStream<'_>) -> Token {
        let Some(ch) = stream.next() else {
            return Token(Kind::Other, None);
        };
        if let Some(token) = self.hook(hooks, ch, stream) {
            return token;
        }
        match ch {
            '@' => {
                stream.eat_while(identifier);
                Token(at_rule_kind(stream.current()), Some("def"))
            }
            '=' => Token(Kind::Other, None),
            '~' | '|' if stream.eat('=').is_some() => Token(Kind::Other, None),
            '"' | '\'' => {
                self.continuation = Some(Continuation::String(ch));
                self.string(stream, ch)
            }
            '#' => {
                stream.eat_while(identifier);
                Token(Kind::Hash, Some("atom"))
            }
            '!' => {
                stream.eat_space();
                stream.eat_while(word);
                Token(Kind::Other, Some("keyword"))
            }
            '0'..='9' => number(stream),
            '.' if stream.eat_if(|ch| ch.is_ascii_digit()).is_some() => number(stream),
            '-' if stream
                .peek()
                .is_some_and(|ch| ch.is_ascii_digit() || ch == '.') =>
            {
                number(stream)
            }
            '-' if stream.eat('-').is_some() => {
                stream.eat_while(identifier);
                variable(stream, "variableName")
            }
            '-' => {
                let tail = suffix(stream);
                let prefix = prefix_len(tail, word);
                if prefix > 0 && tail[prefix..].starts_with('-') {
                    stream.set_pos(stream.pos() + prefix + 1).unwrap();
                    Token(Kind::Meta, Some("meta"))
                } else {
                    // The upstream fallback for a lone hyphen does not set a token type.
                    Token(self.previous, None)
                }
            }
            ',' | '+' | '>' | '*' | '/' => Token(Kind::Other, None),
            '.' if qualifier(stream) => Token(Kind::Qualifier, Some("qualifier")),
            ':' | ';' | '{' | '}' | '[' | ']' | '(' | ')' => Token(Kind::Mark(ch), None),
            _ if function(stream) => {
                if ["url", "url-prefix", "domain", "regexp"]
                    .iter()
                    .any(|name| stream.current().eq_ignore_ascii_case(name))
                {
                    self.continuation = Some(Continuation::Parenthesized);
                }
                Token(Kind::Variable, Some("variableName.function"))
            }
            _ if identifier(ch) => {
                stream.eat_while(identifier);
                Token(Kind::Word, Some("property"))
            }
            _ => Token(Kind::Other, None),
        }
    }

    fn hook(&mut self, hooks: Hooks<'_>, ch: char, stream: &mut StringStream<'_>) -> Option<Token> {
        match (hooks, ch) {
            (Hooks::Custom(hooks), ch) => {
                let hook = hooks.get(&ch)?;
                let start = stream.pos();
                match hook(stream) {
                    Some(token) if stream.pos() >= start => Some(self.hook_token(token, stream)),
                    _ => {
                        stream.set_pos(start).unwrap();
                        None
                    }
                }
            }
            (Hooks::Scss | Hooks::Less, '/') if stream.eat('/').is_some() => {
                stream.skip_to_end();
                Some(Token(Kind::Comment, Some("comment")))
            }
            (_, '/') if stream.eat('*').is_some() => {
                self.continuation = Some(Continuation::Comment);
                Some(self.comment(stream))
            }
            (Hooks::Scss | Hooks::Less, '/') => Some(Token(Kind::Other, Some("operator"))),
            (Hooks::Scss, ':') if after_space(stream).starts_with('{') => {
                Some(Token(Kind::Other, None))
            }
            (Hooks::Scss, '$') => {
                stream.eat_while(|ch| word(ch) || ch == '-');
                Some(variable(stream, "variableName.special"))
            }
            (Hooks::Scss, '#') | (Hooks::Less, '@') if stream.eat('{').is_some() => {
                Some(Token(Kind::Interpolation, None))
            }
            (Hooks::Less, '@') if !less_at_rule(suffix(stream)) => {
                stream.eat_while(identifier);
                Some(variable(stream, "variableName"))
            }
            (Hooks::Less, '&') => Some(Token(Kind::Other, Some("atom"))),
            _ => None,
        }
    }

    fn hook_token(&mut self, token: LegacyCssHookToken, stream: &StringStream<'_>) -> Token {
        self.continuation = token.continuation().map(Continuation::Hook);
        let kind = match token.kind {
            LegacyCssTokenKind::Other => Kind::Other,
            LegacyCssTokenKind::Comment => Kind::Comment,
            LegacyCssTokenKind::AtRule => at_rule_kind(stream.current()),
            LegacyCssTokenKind::Mark(mark) => Kind::Mark(mark),
            LegacyCssTokenKind::Hash => Kind::Hash,
            LegacyCssTokenKind::Word => Kind::Word,
            LegacyCssTokenKind::Qualifier => Kind::Qualifier,
            LegacyCssTokenKind::Meta => Kind::Meta,
            LegacyCssTokenKind::Variable => Kind::Variable,
            LegacyCssTokenKind::VariableDefinition => Kind::VariableDefinition,
            LegacyCssTokenKind::Interpolation => Kind::Interpolation,
        };
        Token(kind, token.style)
    }

    fn comment(&mut self, stream: &mut StringStream<'_>) -> Token {
        let mut star = false;
        for ch in stream {
            if star && ch == '/' {
                self.continuation = None;
                break;
            }
            star = ch == '*';
        }
        Token(Kind::Comment, Some("comment"))
    }

    fn string(&mut self, stream: &mut StringStream<'_>, quote: char) -> Token {
        let mut escaped = false;
        for ch in &mut *stream {
            if ch == quote && !escaped {
                if quote == ')' {
                    stream.back_up(1).unwrap();
                }
                self.continuation = None;
                break;
            }
            escaped = !escaped && ch == '\\';
        }
        if !escaped && quote != ')' {
            self.continuation = None;
        }
        Token(Kind::Other, Some("string"))
    }
}

pub(super) fn after_space<'line>(stream: &StringStream<'line>) -> &'line str {
    suffix(stream).trim_start_matches(js_space)
}

fn suffix<'line>(stream: &StringStream<'line>) -> &'line str {
    &stream.line()[stream.pos()..]
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn identifier(ch: char) -> bool {
    word(ch) || matches!(ch, '\\' | '-')
}

fn prefix_len(text: &str, include: impl Fn(char) -> bool) -> usize {
    text.find(|ch| !include(ch)).unwrap_or(text.len())
}

fn variable(stream: &StringStream<'_>, style: &'static str) -> Token {
    if after_space(stream).starts_with(':') {
        Token(Kind::VariableDefinition, Some("def"))
    } else {
        Token(Kind::Variable, Some(style))
    }
}

fn number(stream: &mut StringStream<'_>) -> Token {
    stream.eat_while(|ch| word(ch) || matches!(ch, '.' | '%'));
    Token(Kind::Other, Some("number"))
}

fn qualifier(stream: &mut StringStream<'_>) -> bool {
    let tail = suffix(stream);
    let selector = tail.strip_prefix('-').unwrap_or(tail);
    if !selector
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
    {
        return false;
    }
    let bytes = tail.len() - selector.len() + prefix_len(selector, |ch| word(ch) || ch == '-');
    stream.set_pos(stream.pos() + bytes).unwrap();
    true
}

fn function(stream: &mut StringStream<'_>) -> bool {
    let tail = suffix(stream);
    let bytes = prefix_len(tail, |ch| word(ch) || matches!(ch, '-' | '.'));
    if bytes == 0 || !tail[bytes..].starts_with('(') {
        return false;
    }
    stream.set_pos(stream.pos() + bytes).unwrap();
    true
}

const KEYFRAMES: &[&str] = &[
    "keyframes",
    "-moz-keyframes",
    "-ms-keyframes",
    "-o-keyframes",
    "-webkit-keyframes",
];

fn at_rule_kind(text: &str) -> Kind {
    let Some(name) = text.strip_prefix('@') else {
        return Kind::Other;
    };
    let name = name.to_ascii_lowercase();
    Kind::AtRule(match name.as_str() {
        "document" | "-moz-document" => AtRule::Document,
        "media" | "supports" | "import" => AtRule::Block,
        _ if name.starts_with("font-face") || name.starts_with("counter-style") => {
            AtRule::Restricted(match text {
                "@font-face" => Restriction::FontFace,
                "@counter-style" => Restriction::CounterStyle,
                _ => Restriction::None,
            })
        }
        _ if KEYFRAMES.contains(&name.as_str()) => AtRule::Keyframes,
        _ if name.starts_with("component") => AtRule::Component,
        _ => AtRule::Other,
    })
}

fn less_at_rule(text: &str) -> bool {
    [
        "charset",
        "document",
        "font-face",
        "import",
        "media",
        "namespace",
        "page",
        "supports",
    ]
    .into_iter()
    .chain(KEYFRAMES.iter().copied())
    .any(|name| {
        text.get(..name.len()).is_some_and(|prefix| {
            prefix.eq_ignore_ascii_case(name)
                && !text[name.len()..].chars().next().is_some_and(word)
        })
    })
}
