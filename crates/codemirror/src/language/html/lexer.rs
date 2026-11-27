use super::HtmlNodeKind::{self, *};

pub(super) fn context_name(source: &str) -> String {
    let end = prefix(source, |ch| {
        matches!(ch, '-' | '.' | ':' | '_') || ch.is_ascii_alphabetic() || ch >= '\u{a1}'
    });
    source[..end].to_lowercase()
}

fn name_start(ch: char) -> bool {
    matches!(ch, ':' | '_' | 'a'..='z' | 'A'..='Z'
        | '\u{c0}'..='\u{d6}' | '\u{d8}'..='\u{f6}' | '\u{f8}'..='\u{2ff}'
        | '\u{370}'..='\u{37d}' | '\u{37f}'..='\u{1fff}' | '\u{200c}'..='\u{200d}'
        | '\u{2070}'..='\u{218f}' | '\u{2c00}'..='\u{2fef}' | '\u{3001}'..='\u{d7ff}'
        | '\u{f900}'..='\u{fdcf}' | '\u{fdf0}'..='\u{fffd}' | '\u{10000}'..='\u{effff}')
}

pub(super) fn name_len(source: &str) -> usize {
    let Some(first) = source.chars().next().filter(|&ch| name_start(ch)) else {
        return 0;
    };
    let start = first.len_utf8();
    start
        + prefix(&source[start..], |ch| {
            name_start(ch)
                || matches!(ch, '-' | '.' | '0'..='9' | '\u{b7}' | '\u{300}'..='\u{36f}' | '\u{203f}'..='\u{2040}')
        })
}

pub(super) fn prefix(source: &str, accept: impl Fn(char) -> bool) -> usize {
    source.find(|ch| !accept(ch)).unwrap_or(source.len())
}

pub(super) fn space_len(source: &str) -> usize {
    prefix(source, |ch| matches!(ch, ' ' | '\t' | '\r' | '\n'))
}

pub(super) fn attribute_name_len(source: &str) -> usize {
    prefix(
        source,
        |ch| !matches!(ch, '\0'..=' ' | '\u{7f}'..='\u{9f}' | '"' | '\'' | '<' | '>' | '/' | '=' | '\u{fdd0}'..='\u{fdef}' | '\u{fffe}' | '\u{ffff}'),
    )
}

pub(super) fn unquoted_len(source: &str) -> usize {
    prefix(source, |ch| {
        !matches!(
            ch,
            ' ' | '\t' | '\n' | '\r' | '\x0c' | '=' | '<' | '>' | '"' | '\'' | '`'
        )
    })
}

#[derive(Default)]
pub(super) struct References {
    entity_end: usize,
    character_end: usize,
}

impl References {
    pub(super) fn read(&mut self, source: &str, at: usize) -> (HtmlNodeKind, usize) {
        // The grammar accepts any nonempty run other than space or semicolon
        // after '&#', and also excludes '#' for named references. Keep the next
        // stop so a run of invalid ampersands only scans ahead once.
        let character = source[at..].starts_with("&#");
        let (start, stop) = if character {
            (at + 2, &mut self.character_end)
        } else {
            (at + 1, &mut self.entity_end)
        };
        if *stop < start {
            *stop = start
                + prefix(&source[start..], |ch| {
                    !matches!(ch, ';' | ' ') && (character || ch != '#')
                });
        }
        if *stop > start && source.as_bytes().get(*stop) == Some(&b';') {
            (
                if character {
                    CharacterReference
                } else {
                    EntityReference
                },
                *stop + 1,
            )
        } else {
            (InvalidEntity, at + 1)
        }
    }
}

pub(super) fn raw_kind(name: &str) -> Option<HtmlNodeKind> {
    Some(match name {
        "script" => ScriptText,
        "style" => StyleText,
        "textarea" => TextareaText,
        _ => return None,
    })
}

pub(super) fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "command"
            | "embed"
            | "frame"
            | "hr"
            | "img"
            | "input"
            | "keygen"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
            | "menuitem"
    )
}

pub(super) fn is_foreign(name: &str) -> bool {
    matches!(name, "svg" | "math")
}

pub(super) fn implicitly_closed(name: &str) -> bool {
    matches!(
        name,
        "dd" | "li"
            | "optgroup"
            | "option"
            | "p"
            | "rp"
            | "rt"
            | "tbody"
            | "td"
            | "tfoot"
            | "th"
            | "tr"
    )
}

pub(super) fn close_on_open(parent: &str, next: &str) -> bool {
    if raw_kind(next).is_some() || is_void(next) {
        return false;
    }
    match parent {
        "dd" | "dt" => matches!(next, "dd" | "dt"),
        "li" => next == "li",
        "option" => matches!(next, "option" | "optgroup"),
        "optgroup" => next == "optgroup",
        "p" => matches!(
            next,
            "address"
                | "article"
                | "aside"
                | "blockquote"
                | "dir"
                | "div"
                | "dl"
                | "fieldset"
                | "footer"
                | "form"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
                | "header"
                | "hgroup"
                | "hr"
                | "menu"
                | "nav"
                | "ol"
                | "p"
                | "pre"
                | "section"
                | "table"
                | "ul"
        ),
        "rp" | "rt" => matches!(next, "rp" | "rt"),
        "tbody" | "thead" => matches!(next, "tbody" | "tfoot"),
        "td" | "th" => matches!(next, "td" | "th"),
        "tfoot" => next == "tbody",
        "tr" => next == "tr",
        _ => false,
    }
}

// The pinned raw tokenizer only accepts the exact lowercase delimiter. It
// resets an attempted match at a line break, and does not re-use failed bytes.
pub(super) fn raw_end(source: &str, name: &str) -> usize {
    let mut offset = 0;
    while let Some(less) = source[offset..].find('<') {
        offset += less;
        let closing = b"</".iter().copied().chain(name.bytes()).chain([b'>']);
        let matched = closing
            .zip(source[offset..].bytes())
            .take_while(|(a, b)| a == b)
            .count();
        if matched == name.len() + 3 {
            return offset;
        }
        // The first mismatching byte was read as well.
        let fail = offset + matched;
        let Some(next) = source[fail..].chars().next() else {
            break;
        };
        offset = fail + next.len_utf8();
    }
    source.len()
}
