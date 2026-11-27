use std::{ops::Range, sync::LazyLock};

use regex::Regex;

use super::super::MarkdownNodeKind::*;
use super::{Node, inline_space, js_space, skip_space};

pub(super) fn label(source: &str, start: usize, mut require_text: bool) -> Option<Range<usize>> {
    let mut escaped = false;
    let mut units = 0;
    for (i, ch) in source[start + 1..].char_indices() {
        if units >= 1000 {
            break;
        }
        if escaped {
            escaped = false;
        } else if ch == ']' {
            return (!require_text).then_some(start..start + 2 + i);
        } else {
            require_text &= ch.is_ascii() && inline_space(ch as u8);
            match ch {
                '[' => return None,
                '\\' => escaped = true,
                _ => (),
            }
        }
        units += ch.len_utf16();
    }
    None
}

fn destination(source: &str, start: usize) -> Option<Range<usize>> {
    let bytes = source.as_bytes();
    if bytes.get(start) == Some(&b'<') {
        let mut escaped = false;
        for (i, &byte) in bytes.iter().enumerate().skip(start + 1) {
            match byte {
                b'>' if !escaped => return Some(start..i + 1),
                b'<' if !escaped => return None,
                b'\n' => return None,
                _ => escaped = byte == b'\\',
            }
        }
        return None;
    }
    let mut depth = 0;
    let mut escaped = false;
    let mut end = start;
    while let Some(&b) = bytes.get(end) {
        if inline_space(b) {
            break;
        }
        if escaped {
            escaped = false;
        } else {
            match b {
                b'(' => depth += 1,
                b')' if depth == 0 => break,
                b')' => depth -= 1,
                b'\\' => escaped = true,
                _ => (),
            }
        }
        end += 1;
    }
    (end > start).then_some(start..end)
}

fn quoted_title(source: &str, start: usize) -> Option<Range<usize>> {
    let bytes = source.as_bytes();
    let mark = match *bytes.get(start)? {
        mark @ (b'\'' | b'"') => mark,
        b'(' => b')',
        _ => return None,
    };
    let mut escaped = false;
    for (i, &byte) in bytes.iter().enumerate().skip(start + 1) {
        if !escaped && byte == mark {
            return Some(start..i + 1);
        }
        escaped = !escaped && byte == b'\\';
    }
    None
}

pub(super) fn finish(source: &str, after: usize, children: &mut Vec<Node>) -> usize {
    match source.as_bytes().get(after) {
        Some(b'(') => {
            let mut end = skip_space(source, after + 1);
            let dest = destination(source, end);
            let mut title = None;
            if let Some(dest) = &dest {
                end = skip_space(source, dest.end);
                if end != dest.end {
                    title = quoted_title(source, end);
                    if let Some(title) = &title {
                        end = skip_space(source, title.end);
                    }
                }
            }
            if source.as_bytes().get(end) == Some(&b')') {
                children.push(Node::leaf(LinkMark, after..after + 1));
                children.extend(dest.map(|range| Node::leaf(URL, range)));
                children.extend(title.map(|range| Node::leaf(LinkTitle, range)));
                children.push(Node::leaf(LinkMark, end..end + 1));
                return end + 1;
            }
        }
        Some(b'[') => {
            if let Some(label) = label(source, after, false) {
                let end = label.end;
                children.push(Node::leaf(LinkLabel, label));
                return end;
            }
        }
        _ => (),
    }
    after
}

pub(in super::super) fn definition(source: &str, range: Range<usize>) -> Option<Node> {
    let source = &source[..range.end];
    let label = label(source, range.start, true)?;
    if source.as_bytes().get(label.end) != Some(&b':') {
        return None;
    }
    let dest = destination(source, skip_space(source, label.end + 1))?;
    let start = skip_space(source, dest.end);
    let mut end = dest.end;
    let mark = label.end..label.end + 1;
    let mut children = vec![
        Node::leaf(LinkLabel, label),
        Node::leaf(LinkMark, mark),
        Node::leaf(URL, dest),
    ];
    if let Some(title) = quoted_title(source, start)
        && !(source[children[2].range.end..title.start].contains('\n')
            && source[title.clone()].contains('\n'))
    {
        end = title.end;
        children.push(Node::leaf(LinkTitle, title));
    }
    Some(Node::new(LinkReference, range.start..end, children))
}

pub(super) fn bare_url(source: &str, start: usize, open_link: bool) -> Option<usize> {
    static URL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^(?:https?://|www\.)([a-zA-Z_0-9-]+(?:\.[a-zA-Z_0-9-]+)+)(?::[0-9]+)?")
            .unwrap()
    });
    static MAIL_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^[a-zA-Z_0-9.+-]+@[a-zA-Z_0-9-]+(?:\.[a-zA-Z_0-9.-]+)+").unwrap()
    });
    static RESOURCE_PATTERN: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^/[a-zA-Z0-9@.]+").unwrap());
    static ENTITY_END: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"&(?:#[0-9]+|#x[a-f0-9]+|[a-zA-Z_0-9]+);$").unwrap());
    if start > 0 && word(source.as_bytes()[start - 1]) {
        return None;
    }
    let source = &source[start..];
    if source.starts_with("www.") || source.starts_with("http://") || source.starts_with("https://")
    {
        let m = URL_PATTERN.captures(source)?;
        if m[1].rsplit('.').take(2).any(|word| word.contains('_')) {
            return None;
        }
        let mut end = m.get(0)?.end();
        if source.as_bytes().get(end) == Some(&b'/') {
            end = source[end..]
                .char_indices()
                .find(|&(_, ch)| ch == '<' || js_space(ch))
                .map_or(source.len(), |(offset, _)| end + offset);
        }
        let mut excess = parentheses(&source[..end]);
        loop {
            match source.as_bytes()[end - 1] {
                b'?' | b'!' | b'.' | b',' | b':' | b'*' | b'_' | b'~' => end -= 1,
                b')' if excess > 0 => {
                    excess -= 1;
                    end -= 1;
                }
                b';' if let Some(entity) = ENTITY_END.find(&source[..end]) => {
                    end = entity.start();
                    excess = parentheses(&source[..end]);
                }
                _ => break,
            }
        }
        if open_link {
            let mut at = 0;
            let bytes = source.as_bytes();
            while at < end {
                match bytes[at] {
                    b']' => break,
                    b'[' => {
                        let Some(relative) = bytes[at + 1..end]
                            .iter()
                            .position(|b| matches!(b, b'[' | b']'))
                        else {
                            break;
                        };
                        if bytes[at + 1 + relative] == b'[' {
                            break;
                        }
                        at += relative + 2;
                    }
                    _ => at += 1,
                }
            }
            end = at;
        }
        return Some(start + end);
    }
    let prefix = if source.starts_with("xmpp:") {
        5
    } else if source.starts_with("mailto:") {
        7
    } else {
        let local = source
            .bytes()
            .take(101)
            .take_while(|&b| word(b) || matches!(b, b'.' | b'+' | b'-'))
            .count();
        if local > 100 || source.as_bytes().get(local) != Some(&b'@') {
            return None;
        }
        0
    };
    let mut end = prefix + MAIL_PATTERN.find(&source[prefix..])?.end();
    match source.as_bytes()[end - 1] {
        b'_' | b'-' => return None,
        b'.' => end -= 1,
        _ => (),
    }
    if prefix == 5
        && let Some(resource) = RESOURCE_PATTERN.find(&source[end..])
    {
        end += resource.end();
    }
    Some(start + end)
}

fn word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn parentheses(source: &str) -> isize {
    source.bytes().fold(0, |count, b| match b {
        b')' => count + 1,
        b'(' => count - 1,
        _ => count,
    })
}
