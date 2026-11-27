use super::LezerNodeKind::{self as Kind, *};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum TokenKind {
    Word,
    Node(Kind),
    Hidden(char),
    Invalid,
    Eof,
}

#[derive(Clone, Copy)]
pub(super) struct Lexeme {
    pub kind: TokenKind,
    pub from: usize,
    pub to: usize,
}

pub(super) fn symbol(ch: char) -> TokenKind {
    TokenKind::Node(match ch {
        '{' => OpenBrace,
        '}' => CloseBrace,
        '[' => OpenBracket,
        ']' => CloseBracket,
        '(' => OpenParen,
        ')' => CloseParen,
        '=' => Equal,
        '.' => Dot,
        '|' => Bar,
        '!' => Bang,
        '~' => Tilde,
        '*' => Star,
        '+' => Plus,
        '?' => Question,
        '<' | '>' | ',' => return TokenKind::Hidden(ch),
        _ => return TokenKind::Invalid,
    })
}

fn directive(name: &str) -> Kind {
    match name {
        "@asciiLetter" | "@asciiUpperCase" | "@asciiLowerCase" | "@digit" | "@whitespace"
        | "@eof" => CharClass,
        "@specialize" => AtSpecialize,
        "@extend" => AtExtend,
        "@top" => AtTop,
        "@precedence" => AtPrecedence,
        "@left" => AtLeft,
        "@right" => AtRight,
        "@cut" => AtCut,
        "@tokens" => AtTokens,
        "@conflict" => AtConflict,
        "@local" => AtLocal,
        "@else" => AtElse,
        "@external" => AtExternal,
        "@context" => AtContext,
        "@dialects" => AtDialects,
        "@skip" => AtSkip,
        "@detectDelim" => AtDetectDelim,
        _ => AtName,
    }
}

fn word_len(input: &str) -> usize {
    input
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '\u{a1}'..)))
        .unwrap_or(input.len())
}

// These are the Unicode White_Space characters, as defined by the generator.
// A name can contain some of them, but whitespace wins at a token start.
fn space_len(input: &str) -> usize {
    input
        .find(|ch: char| !ch.is_whitespace())
        .unwrap_or(input.len())
}

fn quoted(input: &str, closing: char, literal: bool) -> usize {
    let mut offset = 0;
    while let Some(ch) = input[offset..].chars().next() {
        if ch == closing {
            return offset + ch.len_utf8();
        }
        if literal && (ch == '\n' || input[offset..].starts_with("\r\n")) {
            break;
        }
        if ch == '\\' {
            let escaped = &input[offset + 1..];
            let Some(next) = escaped.chars().next() else {
                break;
            };
            offset += if escaped.starts_with("\r\n") {
                2
            } else {
                next.len_utf8()
            };
        }
        offset += ch.len_utf8();
    }
    offset
}

pub(super) fn scan(source: &str) -> (Vec<Lexeme>, Vec<Lexeme>) {
    let mut tokens = Vec::new();
    let mut comments = Vec::new();
    let mut offset = 0;
    // Avoid rescanning a suffix for every opener if it has no closing token.
    let last_block = source.rfind("*/");
    let last_set = source.rmatch_indices(']').find_map(|(at, _)| {
        source.as_bytes()[..at]
            .iter()
            .rev()
            .take_while(|&&b| b == b'\\')
            .count()
            .is_multiple_of(2)
            .then_some(at)
    });
    while offset < source.len() {
        let input = &source[offset..];
        let ch = input.chars().next().unwrap();
        let (kind, len) = if ch.is_whitespace() {
            offset += space_len(input);
            continue;
        } else if input.starts_with("//") {
            let end = input.find('\n').map_or(input.len(), |lf| {
                lf - usize::from(input.as_bytes()[lf - 1] == b'\r')
            });
            (TokenKind::Node(LineComment), end)
        } else if input.starts_with("/*") && last_block.is_some_and(|at| at >= offset + 2) {
            let end = input[2..].find("*/").unwrap() + 4;
            (TokenKind::Node(BlockComment), end)
        } else if matches!(ch, '\'' | '"') {
            (TokenKind::Node(Literal), 1 + quoted(&input[1..], ch, true))
        } else if (input.starts_with("$[") || input.starts_with("!["))
            && last_set.is_some_and(|at| at >= offset + 2)
        {
            let kind = if ch == '$' { CharSet } else { InvertedCharSet };
            (TokenKind::Node(kind), 2 + quoted(&input[2..], ']', false))
        } else if ch == '@' {
            let len = 1 + word_len(&input[1..]);
            let kind = if len == 1 {
                TokenKind::Invalid
            } else {
                TokenKind::Node(directive(&input[..len]))
            };
            (kind, len)
        } else if ch == '_' {
            (TokenKind::Node(AnyChar), 1)
        } else if ch.is_ascii_alphanumeric() || ch == '-' || ch >= '\u{a1}' {
            (TokenKind::Word, word_len(input))
        } else {
            (symbol(ch), ch.len_utf8())
        };
        let token = Lexeme {
            kind,
            from: offset,
            to: offset + len,
        };
        if matches!(kind, TokenKind::Node(LineComment | BlockComment)) {
            comments.push(token);
        } else {
            tokens.push(token);
        }
        offset += len;
    }
    tokens.push(Lexeme {
        kind: TokenKind::Eof,
        from: offset,
        to: offset,
    });
    (tokens, comments)
}
