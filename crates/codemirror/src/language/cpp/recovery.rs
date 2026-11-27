use std::ops::Range;

use super::{super::lezer::SyntaxTree, parser::parse_tree};

pub(super) fn recover(
    source: &str,
    offset: usize,
) -> Option<(SyntaxTree, usize, Option<Range<usize>>)> {
    if let Some(ch) = source.get(offset..)?.chars().next()
        && !ch.is_whitespace()
    {
        // Keep byte offsets while trying a single deletion. The document still
        // contains the original character, which gets an error node.
        let end = offset + ch.len_utf8();
        let mut masked = source.to_owned();
        masked.replace_range(offset..end, &"    "[..ch.len_utf8()]);
        if let Ok(tree) = parse_tree(&masked) {
            return Some((tree, source.len(), Some(offset..end)));
        }
    }
    if source.get(offset..)?.trim().is_empty()
        && let Some(tree) = finish(source)
    {
        return Some((tree, source.len(), None));
    }
    // Save complete earlier lines and mark the unparsed suffix. Do not search
    // successively shorter prefixes or retry after a parser budget failure.
    let line_start = source[..offset].rfind('\n')? + 1;
    if line_start == source.len() {
        return None;
    }
    let prefix = &source[..line_start];
    parse_tree(prefix)
        .ok()
        .or_else(|| finish(prefix))
        .map(|tree| (tree, line_start, None))
}

fn finish(source: &str) -> Option<SyntaxTree> {
    let (before_close, brackets) = closing_suffix(source);
    for suffix in [
        format!("{before_close}{brackets}"),
        format!("{before_close}{brackets};"),
        format!("{before_close};{brackets}"),
        format!("{before_close};{brackets};"),
    ] {
        let suffix = if suffix.is_empty() { "\n" } else { &suffix };
        if let Ok(tree) = parse_tree(&format!("{source}{suffix}")) {
            return Some(tree);
        }
    }
    None
}

// Only used to propose EOF repairs. The published grammar must accept the
// repaired text before it contributes any syntax to the document.
fn closing_suffix(mut source: &str) -> (String, String) {
    let mut prefix = String::new();
    let mut brackets = Vec::new();
    while let Some(at) = source.find(['/', '"', '\'', '(', ')', '[', ']', '{', '}']) {
        let delimiter = source.as_bytes()[at];
        let rest = &source[at + 1..];
        source = match delimiter {
            b'/' if rest.starts_with('/') => {
                let Some((_, rest)) = rest.split_once('\n') else {
                    prefix.push('\n');
                    break;
                };
                rest
            }
            b'/' if rest.starts_with('*') => {
                let Some((_, rest)) = rest[1..].split_once("*/") else {
                    prefix.push_str("*/");
                    break;
                };
                rest
            }
            b'"' | b'\'' => {
                let marker = (delimiter == b'"' && source[..at].ends_with('R'))
                    .then(|| rest.split_once('('))
                    .flatten()
                    .filter(|(marker, _)| {
                        !marker.contains(|ch: char| ch == ')' || ch == ' ' || ch <= '\u{d}')
                    });
                if let Some((marker, body)) = marker {
                    let close = format!("){marker}\"");
                    if let Some((_, rest)) = body.split_once(&close) {
                        rest
                    } else {
                        prefix.push_str(&close);
                        break;
                    }
                } else if delimiter == b'\''
                    && source[..at].ends_with(|ch: char| ch.is_ascii_digit())
                    && rest.starts_with(|ch: char| ch.is_ascii_digit())
                {
                    // Apostrophes can separate digits in the grammar's number token.
                    rest
                } else if let Some(rest) = quoted_end(rest, delimiter) {
                    rest
                } else {
                    prefix.push(char::from(delimiter));
                    break;
                }
            }
            b'(' | b'[' | b'{' => {
                brackets.push(match delimiter {
                    b'(' => ')',
                    b'[' => ']',
                    _ => '}',
                });
                rest
            }
            b')' | b']' | b'}' if brackets.last().copied() == Some(char::from(delimiter)) => {
                brackets.pop();
                rest
            }
            _ => rest,
        };
    }
    (prefix, brackets.into_iter().rev().collect())
}

fn quoted_end(source: &str, delimiter: u8) -> Option<&str> {
    let mut escaped = false;
    for (i, byte) in source.bytes().enumerate() {
        if !escaped && byte == delimiter {
            return Some(&source[i + 1..]);
        }
        if !escaped && byte == b'\n' {
            return Some(&source[i + 1..]);
        }
        escaped = !escaped && byte == b'\\';
    }
    None
}
