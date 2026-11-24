use std::ops::Range;

use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SectionKind {
    Heading,
    Bullet,
    Strong,
    Emphasis,
    FrontMatter,
    Mermaid,
    InlineMath,
    DisplayMath,
    Image,
    Link,
    ReadOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Section {
    pub kind: SectionKind,
    pub source: Range<usize>,
    pub content: Range<usize>,
    pub destination: Option<String>,
}

impl Section {
    pub fn new(kind: SectionKind, source: Range<usize>, content: Range<usize>) -> Self {
        Self {
            kind,
            source,
            content,
            destination: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EditError {
    InvalidRange,
    ReadOnly,
}

#[derive(Clone, Debug, Default)]
pub struct Document {
    source: String,
}

impl Document {
    pub fn new(source: String) -> Self {
        Self { source }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn sections(&self) -> Vec<Section> {
        use SectionKind as K;

        let mut sections: Vec<Section> = Vec::new();
        let mut open_links: Vec<usize> = Vec::new();
        let mut heading: Option<Section> = None;
        let mut fenced_body = None;
        let options = Options::ENABLE_MATH
            | Options::ENABLE_TABLES
            | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
            | Options::ENABLE_STRIKETHROUGH;

        for (event, source) in Parser::new_ext(&self.source, options).into_offset_iter() {
            if let Some(&index) = open_links.last() {
                let link = &mut sections[index];
                if matches!(event, Event::End(TagEnd::Link | TagEnd::Image))
                    && link.source == source
                {
                    // The parser covers brackets, escapes, and nested syntax in its
                    // child ranges. Only skipped whitespace can precede the close.
                    if let Some(offset) = self.source[link.content.end..source.end].find("](") {
                        link.content.end += offset;
                    }
                    open_links.pop();
                } else {
                    link.content.end = link.content.end.max(source.end);
                }
            }
            if let Some(heading) = &mut heading
                && !matches!(&event, Event::End(TagEnd::Heading(_)))
            {
                heading.content.start = heading.content.start.min(source.start);
                heading.content.end = source.end.max(heading.content.start);
            }
            let text = &self.source[source.clone()];
            let section = match event {
                Event::Start(Tag::Heading { .. }) => {
                    heading = Some(Section::new(
                        K::Heading,
                        source.clone(),
                        source.end..source.end,
                    ));
                    None
                }
                Event::End(TagEnd::Heading(_)) => heading.take(),
                Event::Start(Tag::Strong) => Some(delimited(K::Strong, source, 2)),
                Event::Start(Tag::Emphasis) => Some(delimited(K::Emphasis, source, 1)),
                Event::Start(Tag::Item) if text.starts_with(['-', '*', '+']) => {
                    let start =
                        source.start + text.len() - text[1..].trim_start_matches([' ', '\t']).len();
                    Some(Section::new(K::Bullet, source.clone(), start..source.end))
                }
                Event::InlineMath(_) => Some(delimited(K::InlineMath, source, 1)),
                Event::DisplayMath(_) => Some(delimited(K::DisplayMath, source, 2)),
                Event::Start(Tag::MetadataBlock(_)) => {
                    fenced_body = Some((K::FrontMatter, source.start));
                    None
                }
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                    if info.split_whitespace().next() == Some("mermaid") =>
                {
                    fenced_body = Some((K::Mermaid, source.start));
                    None
                }
                Event::Text(_) => {
                    if let Some((_, end)) = &mut fenced_body {
                        *end = (*end).max(source.end);
                    }
                    None
                }
                Event::End(TagEnd::CodeBlock | TagEnd::MetadataBlock(_)) => fenced_body
                    .take()
                    .map(|(kind, body_end)| block(kind, source, text, body_end)),
                Event::Start(Tag::Link {
                    link_type: LinkType::Inline,
                    dest_url,
                    ..
                }) => Some(link(K::Link, source, 1, dest_url.into_string())),
                Event::Start(Tag::Image {
                    link_type: LinkType::Inline,
                    dest_url,
                    ..
                }) => Some(link(K::Image, source, 2, dest_url.into_string())),
                Event::Start(
                    Tag::Table(_)
                    | Tag::CodeBlock(_)
                    | Tag::BlockQuote(_)
                    | Tag::List(Some(_))
                    | Tag::Strikethrough
                    | Tag::HtmlBlock
                    | Tag::Link { .. }
                    | Tag::Image { .. },
                )
                | Event::Code(_)
                | Event::Html(_)
                | Event::InlineHtml(_)
                | Event::Rule => Some(Section::new(K::ReadOnly, source.clone(), source)),
                _ => None,
            };
            if let Some(section) = section {
                if matches!(section.kind, K::Link | K::Image) {
                    open_links.push(sections.len());
                }
                sections.push(section);
            }
        }

        let sort_key =
            |section: &Section| (section.source.start, std::cmp::Reverse(section.source.end));
        sections.sort_by_key(sort_key);
        let mut fallback = Vec::new();
        let mut start = 0;
        for section in &sections {
            if matches!(
                section.kind,
                K::Heading | K::Bullet | K::Strong | K::Emphasis
            ) {
                continue;
            }
            if section.source.start > start {
                malformed_math(
                    &self.source[start..section.source.start],
                    start,
                    &mut fallback,
                );
            }
            start = start.max(section.source.end);
        }
        malformed_math(&self.source[start..], start, &mut fallback);
        sections.extend(fallback);
        sections.sort_by_key(sort_key);
        sections
    }

    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Result<(), EditError> {
        self.replacement(range, text, true).map(|_| ())
    }

    // Return the actual old range and inserted byte count for editor offsets.
    // The caret belongs directly after `text`, before any added fence separator.
    pub(crate) fn replacement(
        &mut self,
        mut range: Range<usize>,
        text: &str,
        expand_hidden_delimiters: bool,
    ) -> Result<(Range<usize>, usize), EditError> {
        use SectionKind as K;

        if self.source.get(range.clone()).is_none() {
            return Err(EditError::InvalidRange);
        }
        let sections = self.sections();
        if expand_hidden_delimiters && text.is_empty() && !range.is_empty() {
            // Sections are sorted outermost first. Remove inner spans first so
            // an enclosing span also goes away if it becomes empty.
            for section in sections.iter().rev() {
                if matches!(section.kind, K::Strong | K::Emphasis)
                    && range.start <= section.content.start
                    && section.content.end <= range.end
                {
                    range.start = range.start.min(section.source.start);
                    range.end = range.end.max(section.source.end);
                }
            }
        }
        if sections.iter().any(|section| {
            section.kind == K::ReadOnly
                && range.start < section.source.end
                && section.source.start < range.end
        }) {
            return Err(EditError::ReadOnly);
        }
        // There is no body line when the closing fence follows the opening
        // line directly. Leave a line ending between the new body and fence.
        let needs_separator = !text.is_empty()
            && range.is_empty()
            && sections.iter().any(|section| {
                matches!(section.kind, K::FrontMatter | K::Mermaid)
                    && section.content == range
                    && section.source.end > range.start
                    && !self.source[range.start..].starts_with(['\r', '\n'])
            });
        let inserted = if needs_separator {
            let ending = if self.source[..range.start].ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            self.source
                .replace_range(range.clone(), &format!("{text}{ending}"));
            text.len() + ending.len()
        } else {
            self.source.replace_range(range.clone(), text);
            text.len()
        };
        Ok((range, inserted))
    }
}

fn delimited(kind: SectionKind, source: Range<usize>, delimiter_len: usize) -> Section {
    let content = source.start + delimiter_len..source.end - delimiter_len;
    Section::new(kind, source, content)
}

fn block(kind: SectionKind, source: Range<usize>, text: &str, body_end: usize) -> Section {
    let start = text.find('\n').map_or(text.len(), |index| index + 1);
    // Parser text events cover the body but not a valid closing fence.
    // A fence-like line included in the text stays part of the body.
    let end = text
        .trim_end_matches(['\r', '\n'])
        .rfind('\n')
        .filter(|&index| source.start + index + 1 >= body_end)
        .filter(|&index| !text[index + 1..].trim().is_empty())
        .map_or(text.len(), |index| {
            index - usize::from(index > 0 && text.as_bytes()[index - 1] == b'\r')
        })
        .max(start);
    Section::new(
        kind,
        source.clone(),
        source.start + start..source.start + end,
    )
}

// CommonMark's math extension requires matching brace context at both dollar
// delimiters. Keep malformed but clearly delimited input editable, even after
// reopening. The caller excludes existing math, code, metadata and link ranges.
fn malformed_math(text: &str, base: usize, output: &mut Vec<Section>) {
    use SectionKind::{DisplayMath, InlineMath};

    let mut inline = None;
    let mut display = None;
    let mut escaped = false;
    let mut bytes = text.bytes().enumerate().peekable();
    while let Some((index, byte)) = bytes.next() {
        if escaped {
            escaped = false;
            continue;
        }
        match byte {
            b'\\' => escaped = true,
            b'\n' | b'\r' => inline = None,
            b'$' if bytes.peek().is_some_and(|(_, next)| *next == b'$') => {
                bytes.next();
                inline = None;
                if let Some(start) = display.take() {
                    output.push(delimited(DisplayMath, base + start..base + index + 2, 2));
                } else {
                    display = Some(index);
                }
            }
            b'$' if display.is_none() => {
                let left = text[..index].chars().next_back();
                let right = text[index + 1..].chars().next();
                if let Some(start) = inline.take()
                    && left.is_some_and(|ch| !ch.is_whitespace())
                    && !right.is_some_and(|ch| ch.is_ascii_digit())
                {
                    output.push(delimited(InlineMath, base + start..base + index + 1, 1));
                } else if right.is_some_and(|ch| !ch.is_whitespace()) {
                    inline = Some(index);
                }
            }
            _ => {}
        }
    }
}

fn link(kind: SectionKind, source: Range<usize>, prefix: usize, destination: String) -> Section {
    let start = source.start + prefix;
    let mut section = Section::new(kind, source, start..start);
    section.destination = Some(destination);
    section
}
