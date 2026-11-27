use std::ops::Range;

use super::{JavascriptNodeKind, JavascriptOptions, Node, parser, specialize, trim_js_space};

pub(super) fn parse(
    source: &str,
    options: JavascriptOptions,
    expression: bool,
    recover_grammar: bool,
) -> Vec<Node> {
    let mut nodes = Vec::new();
    let root = if expression {
        "SingleExpression"
    } else {
        "Script"
    };
    push(&mut nodes, None, root, 0..source.len());
    let mut groups: Vec<(usize, char)> = vec![(0, '\0')];
    let mut offset = 0;
    let mut previous = "";
    let mut trailing = "";
    while offset < source.len() {
        let rest = &source[offset..];
        let trimmed = trim_js_space(rest);
        offset += rest.len() - trimmed.len();
        if trimmed.is_empty() {
            break;
        }
        let token = lex(trimmed, previous, options);
        let end = offset + token.len;
        let (mut parent, close) = *groups.last().expect("root group");
        if let Some((kind, closer)) = match token.kind {
            "{" => Some((
                if matches!(previous, "Equals" | "(" | "[" | "," | "return") {
                    "ObjectExpression"
                } else {
                    "Block"
                },
                '}',
            )),
            "[" => Some(("ArrayExpression", ']')),
            "(" => Some(("ParenthesizedExpression", ')')),
            _ => None,
        } {
            parent = push(&mut nodes, Some(parent), kind, offset..source.len());
            groups.push((parent, closer));
        }
        let closing = matches!(token.kind, "}" | "]" | ")");
        let kind = if closing && !token.kind.starts_with(close) {
            "⚠"
        } else {
            token.kind
        };
        let id = push(&mut nodes, Some(parent), kind, offset..end);
        if closing && kind != "⚠" {
            nodes[parent].range.end = end;
            groups.pop();
        }
        if !token.missing.is_empty() {
            push(&mut nodes, Some(id), "⚠", end..end);
        }
        previous = if matches!(kind, "LineComment" | "BlockComment" | "Hashbang") {
            previous
        } else {
            kind
        };
        trailing = if kind == "LineComment" || kind == "Hashbang" {
            "\n"
        } else {
            token.missing
        };
        offset = end;
    }
    if expression && matches!(previous, "." | "?.") && trailing.is_empty() {
        trailing = "_";
    }
    let open_groups = &groups[1..];
    let parser = parser::configured(options, expression);
    if (trailing != "\n" && !trailing.is_empty() || !open_groups.is_empty())
        && open_groups.len() <= 32
    {
        let mut repaired = source.to_owned();
        repaired.push_str(trailing);
        repaired.extend(open_groups.iter().rev().map(|(_, closer)| closer));
        let tree = parser::parse_fresh(parser, &repaired);
        if let Ok(tree) = tree {
            return parser::collect(&tree, source.len());
        }
    }
    // If suffix repair cannot finish, try to keep the grammar's scopes and
    // groups. Only retry a token error, not a strict parser limit or hook error.
    if recover_grammar && let Ok(tree) = parser.parse_recovering(source) {
        return parser::collect(&tree, source.len());
    }
    for (id, _) in open_groups.iter().rev() {
        push(&mut nodes, Some(*id), "⚠", source.len()..source.len());
    }
    if !nodes.iter().any(|node| node.kind.is_error()) {
        push(&mut nodes, Some(0), "⚠", source.len()..source.len());
    }
    nodes
}

fn push(
    nodes: &mut Vec<Node>,
    parent: Option<usize>,
    name: &'static str,
    range: Range<usize>,
) -> usize {
    let id = nodes.len();
    if let Some(parent) = parent {
        nodes[parent].children.push(id);
    }
    nodes.push(Node {
        kind: JavascriptNodeKind(name),
        term: None,
        range,
        parent,
        children: Vec::new(),
    });
    id
}

struct Token {
    kind: &'static str,
    len: usize,
    missing: &'static str,
}

impl Token {
    fn new(kind: &'static str, len: usize) -> Self {
        Self {
            kind,
            len,
            missing: "",
        }
    }
}

fn lex(text: &str, previous: &str, options: JavascriptOptions) -> Token {
    if text.starts_with("//") || text.starts_with("#!") {
        let kind = if text.starts_with("//") {
            "LineComment"
        } else {
            "Hashbang"
        };
        return Token::new(kind, text.find('\n').unwrap_or(text.len()));
    }
    if let Some(inner) = text.strip_prefix("/*") {
        return match inner.find("*/") {
            Some(end) => Token::new("BlockComment", end + 4),
            None => Token {
                kind: "BlockComment",
                len: text.len(),
                missing: "*/",
            },
        };
    }
    let first = text.chars().next().expect("non-empty input");
    if matches!(first, '\'' | '"' | '`') {
        return quote(text, first);
    }
    if first == '/'
        && matches!(
            previous,
            "" | "Equals"
                | "Arrow"
                | "return"
                | "throw"
                | "("
                | "["
                | "{"
                | ","
                | ";"
                | ":"
                | "LogicOp"
        )
        && let Some(len) = regexp(text)
    {
        return Token::new("RegExp", len);
    }
    if first.is_ascii_digit()
        || first == '.' && text.as_bytes().get(1).is_some_and(u8::is_ascii_digit)
    {
        let mut last = first;
        let mut len = 1;
        for ch in text[1..].chars() {
            if !(ch.is_ascii_alphanumeric()
                || matches!(ch, '_' | '.')
                || matches!(last, 'e' | 'E') && matches!(ch, '+' | '-'))
            {
                break;
            }
            last = ch;
            len += 1;
        }
        return Token::new("Number", len);
    }
    let identifier = |ch: char| {
        ch.is_ascii_alphabetic()
            || matches!(ch, '$' | '_')
            || ch >= '\u{a1}' && !trim_js_space(ch.encode_utf8(&mut [0; 4])).is_empty()
    };
    if identifier(first) {
        let end = text
            .char_indices()
            .skip(1)
            .find(|&(_, ch)| !identifier(ch) && !ch.is_ascii_digit())
            .map_or(text.len(), |(offset, _)| offset);
        let entry = specialize::identifier(&text[..end]);
        let term = entry >> 1;
        let kind = match previous {
            "." | "?." => "PropertyName",
            "let" | "var" | "const" | "function" | "class" => "VariableDefinition",
            _ if entry != 0 && (options.typescript || !super::tables::TS_TERMS.contains(&term)) => {
                parser::name(term)
            }
            _ => "VariableName",
        };
        return Token::new(kind, end);
    }
    for (text_token, kind) in [
        ("...", "Spread"),
        ("=>", "Arrow"),
        ("?.", "?."),
        ("++", "ArithOp"),
        ("--", "ArithOp"),
        ("===", "CompareOp"),
        ("!==", "CompareOp"),
        ("==", "CompareOp"),
        ("!=", "CompareOp"),
        ("<=", "CompareOp"),
        (">=", "CompareOp"),
        ("&&", "LogicOp"),
        ("||", "LogicOp"),
        ("??", "LogicOp"),
    ] {
        if text.starts_with(text_token) {
            return Token::new(kind, text_token.len());
        }
    }
    let kind = match first {
        '(' => "(",
        ')' => ")",
        '[' => "[",
        ']' => "]",
        '{' => "{",
        '}' => "}",
        ',' => ",",
        ';' => ";",
        ':' => ":",
        '.' => ".",
        '@' => "@",
        '=' => "Equals",
        '+' | '-' | '*' | '/' | '%' => "ArithOp",
        '!' | '?' => "LogicOp",
        '<' | '>' => "CompareOp",
        '&' | '|' | '^' | '~' => "BitOp",
        _ => "⚠",
    };
    Token::new(kind, first.len_utf8())
}

fn quote(text: &str, delimiter: char) -> Token {
    let kind = if delimiter == '`' {
        "TemplateString"
    } else {
        "String"
    };
    let mut escaped = false;
    for (offset, ch) in text.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == delimiter || ch == '\n' && delimiter != '`' {
            return Token::new(kind, offset + 1);
        }
    }
    let missing = match (delimiter, escaped) {
        ('\'', false) => "'",
        ('\'', true) => "\\'",
        ('"', false) => "\"",
        ('"', true) => "\\\"",
        (_, false) => "`",
        (_, true) => "\\`",
    };
    Token {
        kind,
        len: text.len(),
        missing,
    }
}

fn regexp(text: &str) -> Option<usize> {
    let mut escaped = false;
    let mut brackets = false;
    for (offset, ch) in text.char_indices().skip(1) {
        if ch == '\n' {
            return (offset > 1).then_some(offset);
        }
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '[' => brackets = true,
            ']' => brackets = false,
            '/' if !brackets => {
                let flags = text[offset + 1..]
                    .bytes()
                    .take_while(|ch| b"dgimsuvy".contains(ch))
                    .count();
                return Some(offset + 1 + flags);
            }
            _ => {}
        }
    }
    // If a closing slash is missing, consume the regexp prefix once.
    (text.len() > 1).then_some(text.len())
}
