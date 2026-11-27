use std::borrow::Cow;

use super::{advance, eat_str, remaining, word};
use crate::{
    legacy_modes::{js_line_terminator, js_space},
    stream_parser::StringStream,
};

macro_rules! scan {
    ($stream:expr, $pattern:expr, $consume:expr) => {{
        static RE: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(concat!("^", $pattern)).unwrap());
        $stream.match_regex(&RE, $consume)
    }};
}

macro_rules! space {
    () => {
        r"[\x09-\x0D\x20\xA0\u1680\u2000-\u200A\u2028\u2029\u202F\u205F\u3000\uFEFF]"
    };
}

macro_rules! dot {
    () => {
        r"[^\r\n\u2028\u2029]"
    };
}

macro_rules! attributes {
    () => {
        // A supplementary scalar can satisfy both required selector code units.
        r"(?:\((?:[^(][^)]+|[\u{10000}-\u{10FFFF}][^)]*)\)|\{[^}]+\}|\[[^\[\]]+\]|<>|<|>|=|(?:\(+|\)+){1,2})"
    };
}

macro_rules! block_type {
    () => {
        r"(?:div|fn[0-9]+|h[1-6]|bc|bq|notextile|pre|table|p)"
    };
}

macro_rules! list {
    () => {
        r"(?:#+|\*+)"
    };
}

macro_rules! definition {
    () => {
        concat!("- ", dot!(), "*?:=+")
    };
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    mode: Mode,
    layout: Option<Layout>,
    spanning: bool,
    phrases: [bool; Phrase::Notextile as usize + 1],
    table_heading: bool,
}

#[derive(Clone, Copy, Debug, Default)]
enum Mode {
    #[default]
    NewLayout,
    Attributes,
    LayoutLength,
    Text,
    TableCell,
    TableCellAttributes,
    TableText,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Layout {
    Header(Option<u8>),
    Quote,
    Code,
    Footnote,
    Notextile,
    Pre,
    Div,
    Table,
    List,
    SpecialList,
    DefinitionList,
}

impl Layout {
    fn style(self) -> &'static str {
        match self {
            Self::Header(_) => "heading",
            Self::Quote => "bracket",
            Self::Code => "monospace",
            Self::Footnote => "variable",
            Self::Notextile => "string.special",
            Self::Pre => "operator",
            Self::Div => "punctuation",
            Self::Table => "variableName.special",
            Self::List | Self::DefinitionList => "list",
            Self::SpecialList => "list.special",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Phrase {
    Addition,
    Bold,
    Cite,
    Code,
    Deletion,
    Em,
    FootCite,
    Image,
    Italic,
    Span,
    Strong,
    Sub,
    Sup,
    Notextile,
}

const PHRASES: [(Phrase, &str); 13] = [
    (Phrase::Addition, "inserted"),
    (Phrase::Bold, "strong"),
    (Phrase::Cite, "keyword"),
    (Phrase::Code, "monospace"),
    (Phrase::Deletion, "deleted"),
    (Phrase::Em, "emphasis"),
    (Phrase::FootCite, "qualifier"),
    (Phrase::Image, "atom"),
    (Phrase::Italic, "emphasis"),
    (Phrase::Span, "quote"),
    (Phrase::Strong, "strong"),
    (Phrase::Sub, "content.special"),
    (Phrase::Sup, "content.special"),
];

impl State {
    pub(super) fn blank_line(&mut self) {
        let layout = if self.spanning {
            self.layout.map(|layout| match layout {
                Layout::Header(_) => Layout::Header(None),
                layout => layout,
            })
        } else {
            None
        };
        *self = Self {
            layout,
            spanning: self.spanning,
            ..Self::default()
        };
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<Cow<'static, str>> {
        if stream.sol() {
            self.mode = Mode::NewLayout;
            self.table_heading = false;
            if self.layout == Some(Layout::DefinitionList)
                && self.spanning
                && scan!(stream, concat!(dot!(), "*=:", space!(), "*$"), false).is_some()
            {
                self.spanning = false;
            }
        }
        match self.mode {
            Mode::NewLayout => self.new_layout(stream),
            Mode::Attributes => {
                self.mode = Mode::LayoutLength;
                if scan!(stream, concat!(attributes!(), "+"), true).is_some() {
                    return Some(Cow::Borrowed("propertyName"));
                }
                self.styles()
            }
            Mode::LayoutLength => {
                if stream.eat('.').is_some() && stream.eat('.').is_some() {
                    self.spanning = true;
                }
                self.mode = Mode::Text;
                self.styles()
            }
            Mode::Text => self.text(stream),
            Mode::TableCell => self.table_cell(stream),
            Mode::TableCellAttributes => {
                self.mode = Mode::TableText;
                if scan!(
                    stream,
                    concat!(r"(?:[/\\][0-9]+|", attributes!(), r")+\."),
                    true
                )
                .is_some()
                {
                    return Some(Cow::Borrowed("propertyName"));
                }
                self.styles()
            }
            Mode::TableText => {
                if scan!(stream, r#"[^"_*\[(?+~^%@|\-]+"#, true).is_some() {
                    return self.styles();
                }
                if stream.peek() == Some('|') {
                    self.mode = Mode::TableCell;
                    return self.styles();
                }
                let ch = stream.next();
                self.phrase(stream, ch)
            }
        }
    }

    fn new_layout(&mut self, stream: &mut StringStream<'_>) -> Option<Cow<'static, str>> {
        if scan!(
            stream,
            concat!(block_type!(), attributes!(), r"*\.\.?(?:", space!(), "+|$)"),
            false
        )
        .is_some()
        {
            self.spanning = false;
            let m = scan!(stream, block_type!(), true).unwrap();
            self.layout = match &m[0] {
                "bc" => Some(Layout::Code),
                "bq" => Some(Layout::Quote),
                "div" => Some(Layout::Div),
                "notextile" => Some(Layout::Notextile),
                "pre" => Some(Layout::Pre),
                "table" => Some(Layout::Table),
                "p" => None,
                word if word.starts_with("fn") => Some(Layout::Footnote),
                word => Some(Layout::Header(Some(word.as_bytes()[1] - b'0'))),
            };
            self.mode = Mode::Attributes;
            return self.styles();
        }
        if self.disabled().is_none() {
            if scan!(
                stream,
                concat!(list!(), attributes!(), "*", space!(), "+"),
                false
            )
            .is_some()
            {
                let depth = scan!(stream, list!(), true).unwrap()[0].len();
                self.layout = Some(if depth % 3 == 2 {
                    Layout::SpecialList
                } else {
                    Layout::List
                });
                self.mode = Mode::Attributes;
                return self.styles();
            }
            if scan!(stream, concat!(r"\|", dot!(), r"*\|$"), false).is_some() {
                self.layout = Some(Layout::Table);
                return self.table_cell(stream);
            }
            if scan!(
                stream,
                concat!(r"\[[^\]", space!(), r"]+\][^", space!(), "]+$"),
                false
            )
            .is_some()
            {
                stream.skip_to_end();
                return Some(Cow::Borrowed("link"));
            }
            if scan!(stream, definition!(), true).is_some() {
                scan!(stream, definition!(), true);
                self.layout = Some(Layout::DefinitionList);
                if scan!(stream, concat!(space!(), "*$"), true).is_some() {
                    self.spanning = true;
                } else {
                    self.mode = Mode::Attributes;
                }
                return self.styles();
            }
            if block_html(remaining(stream)) {
                stream.skip_to_end();
                return Some(Cow::Borrowed("comment"));
            }
        }
        self.mode = Mode::Text;
        self.text(stream)
    }

    fn table_cell(&mut self, stream: &mut StringStream<'_>) -> Option<Cow<'static, str>> {
        if eat_str(stream, "|_.") {
            self.table_heading = true;
        } else {
            stream.eat('|');
        }
        self.mode = Mode::TableCellAttributes;
        self.styles()
    }

    fn text(&mut self, stream: &mut StringStream<'_>) -> Option<Cow<'static, str>> {
        if scan!(stream, r#"[^!"_=*\[(<?+~^%@\-]+"#, true).is_some() {
            return self.styles();
        }
        let ch = stream.next();
        if ch == Some('"') {
            let rest = remaining(stream);
            if let Some(end) = rest.find('"').filter(|&end| end > 0)
                && let Some(url) = rest[end + 1..].strip_prefix(':')
                && url.starts_with(|ch| !js_space(ch))
            {
                advance(stream, end + 2);
                stream.eat_while(|ch| !js_space(ch));
                return Some(Cow::Borrowed("link"));
            }
            return self.styles();
        }
        self.phrase(stream, ch)
    }

    fn phrase(
        &mut self,
        stream: &mut StringStream<'_>,
        ch: Option<char>,
    ) -> Option<Cow<'static, str>> {
        let (phrase, marker) = match ch {
            Some('_') if stream.eat('_').is_some() => (Phrase::Italic, "__"),
            Some('_') => (Phrase::Em, "_"),
            Some('*') if stream.eat('*').is_some() => (Phrase::Bold, "**"),
            Some('*') => (Phrase::Strong, "*"),
            Some('?') if stream.eat('?').is_some() => (Phrase::Cite, "??"),
            Some('=') if stream.eat('=').is_some() => (Phrase::Notextile, "=="),
            Some('-') if stream.eat('-').is_none() => (Phrase::Deletion, "-"),
            Some('+') => (Phrase::Addition, "+"),
            Some('~') => (Phrase::Sub, "~"),
            Some('^') => (Phrase::Sup, "^"),
            Some('%') => (Phrase::Span, "%"),
            Some('@') => (Phrase::Code, "@"),
            Some('!') => (Phrase::Image, "!"),
            Some('[') => {
                if scan!(stream, r"[0-9]+\]", true).is_some() {
                    self.phrases[Phrase::FootCite as usize] = true;
                }
                return self.styles();
            }
            Some('(') if scan!(stream, r"(?:r|tm|c)\)", true).is_some() => {
                return Some(Cow::Borrowed("character"));
            }
            Some('<') => {
                if let Some(end) = inline_html(remaining(stream)) {
                    advance(stream, end);
                    return Some(Cow::Borrowed("comment"));
                }
                return self.styles();
            }
            _ => return self.styles(),
        };
        let style = self.toggle_phrase(stream, phrase, marker);
        if matches!(phrase, Phrase::Image)
            && let Some(url) = remaining(stream).strip_prefix(':')
            && url.starts_with(|ch| !js_space(ch))
        {
            stream.next();
            stream.eat_while(|ch| !js_space(ch));
        }
        style
    }

    fn toggle_phrase(
        &mut self,
        stream: &StringStream<'_>,
        phrase: Phrase,
        marker: &str,
    ) -> Option<Cow<'static, str>> {
        let before = stream.line()[..stream.pos() - marker.len()]
            .chars()
            .next_back();
        let after = stream.peek();
        if self.phrases[phrase as usize] {
            if !after.is_some_and(word) && before.is_some_and(|ch| !js_space(ch)) {
                let style = self.styles();
                self.phrases[phrase as usize] = false;
                return style;
            }
        } else if !before.is_some_and(word)
            && after.is_some_and(|ch| !js_space(ch))
            && phrase_closes(remaining(stream), marker, matches!(phrase, Phrase::Image))
        {
            self.phrases[phrase as usize] = true;
            self.mode = Mode::Attributes;
        }
        self.styles()
    }

    fn disabled(&self) -> Option<Cow<'static, str>> {
        match self.layout {
            Some(layout @ (Layout::Notextile | Layout::Code | Layout::Pre)) => {
                Some(Cow::Borrowed(layout.style()))
            }
            _ if self.phrases[Phrase::Notextile as usize] => join_styles(
                [Some("string.special"), self.layout.map(Layout::style)]
                    .into_iter()
                    .flatten(),
            ),
            _ => None,
        }
    }

    fn styles(&self) -> Option<Cow<'static, str>> {
        self.disabled().or_else(|| {
            let header = if let Some(Layout::Header(level)) = self.layout {
                Some(match level {
                    Some(1) => "heading-1",
                    Some(2) => "heading-2",
                    Some(3) => "heading-3",
                    Some(4) => "heading-4",
                    Some(5) => "heading-5",
                    Some(6) => "heading-6",
                    _ => "heading-undefined",
                })
            } else {
                None
            };
            join_styles(
                self.layout
                    .map(Layout::style)
                    .into_iter()
                    .chain(PHRASES.iter().copied().filter_map(|(phrase, style)| {
                        self.phrases[phrase as usize].then_some(style)
                    }))
                    .chain(self.table_heading.then_some("operator"))
                    .chain(header),
            )
        })
    }
}

fn join_styles(mut styles: impl Iterator<Item = &'static str>) -> Option<Cow<'static, str>> {
    let first = styles.next()?;
    let Some(second) = styles.next() else {
        return Some(Cow::Borrowed(first));
    };
    let mut joined = format!("{first} {second}");
    for style in styles {
        joined.push(' ');
        joined.push_str(style);
    }
    Some(Cow::Owned(joined))
}

fn phrase_closes(text: &str, marker: &str, image: bool) -> bool {
    text.char_indices()
        .take_while(|&(_, ch)| !js_line_terminator(ch))
        .filter(|&(_, ch)| !js_space(ch))
        .any(|(offset, ch)| {
            let suffix = &text[offset + ch.len_utf8()..];
            let tail = suffix.strip_prefix(marker).or_else(|| {
                if !image {
                    return None;
                }
                let attributes = suffix.strip_prefix('(')?;
                let end = attributes.find(')').filter(|&end| end > 0)?;
                attributes[end + 1..].strip_prefix('!')
            });
            tail.is_some_and(|tail| !tail.starts_with(word))
        })
}

fn block_html(mut text: &str) -> bool {
    let mut first_name = None;
    let mut close_name = None;
    while !text.is_empty() {
        let Some(body) = text.trim_start_matches(js_space).strip_prefix('<') else {
            return false;
        };
        let body = body.strip_prefix('/').unwrap_or(body);
        let Some((name, open_end)) = html_open(body) else {
            return false;
        };
        let first = *first_name.get_or_insert(name);
        text = &body[open_end + 1..];
        if !text.is_empty() && !text.trim_start_matches(js_space).starts_with('<') {
            let Some((name, end)) = html_close(text) else {
                return false;
            };
            if name.is_empty()
                || !first.starts_with(name)
                || *close_name.get_or_insert(name) != name
            {
                return false;
            }
            text = &text[end..];
        }
    }
    first_name.is_some()
}

fn inline_html(body: &str) -> Option<usize> {
    let (opening, end) = html_open(body)?;
    let (closing, close_end) = html_close(&body[end + 1..])?;
    (!closing.is_empty() && closing.len() < end && opening.starts_with(closing))
        .then_some(end + 1 + close_end)
}

fn html_open(body: &str) -> Option<(&str, usize)> {
    let name_len = body.bytes().take_while(|&ch| word(char::from(ch))).count();
    let end = body.find('>')?;
    (name_len > 0).then_some((&body[..name_len], end))
}

fn html_close(text: &str) -> Option<(&str, usize)> {
    let start = text.find('<').filter(|&start| start > 0)?;
    let close = text[start..].strip_prefix("</")?;
    let end = close.find('>')?;
    Some((&close[..end], start + 3 + end))
}
