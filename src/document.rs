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

        let mut sections = Vec::new();
        let mut heading: Option<Section> = None;
        let options = Options::ENABLE_MATH
            | Options::ENABLE_TABLES
            | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
            | Options::ENABLE_STRIKETHROUGH;

        for (event, source) in Parser::new_ext(&self.source, options).into_offset_iter() {
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
                    Some(block(K::FrontMatter, source, text, "---"))
                }
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info)))
                    if info.split_whitespace().next() == Some("mermaid") =>
                {
                    let fence = if text.trim_start().starts_with('~') {
                        "~~~"
                    } else {
                        "```"
                    };
                    Some(block(K::Mermaid, source, text, fence))
                }
                Event::Start(Tag::Link {
                    link_type: LinkType::Inline,
                    dest_url,
                    ..
                }) => Some(link(K::Link, source, text, 1, dest_url.into_string())),
                Event::Start(Tag::Image {
                    link_type: LinkType::Inline,
                    dest_url,
                    ..
                }) => Some(link(K::Image, source, text, 2, dest_url.into_string())),
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
            sections.extend(section);
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
        if self.source.get(range.clone()).is_none() {
            return Err(EditError::InvalidRange);
        }
        if self.sections().iter().any(|section| {
            section.kind == SectionKind::ReadOnly
                && range.start < section.source.end
                && section.source.start < range.end
        }) {
            return Err(EditError::ReadOnly);
        }
        self.source.replace_range(range, text);
        Ok(())
    }
}

fn delimited(kind: SectionKind, source: Range<usize>, delimiter_len: usize) -> Section {
    let content = source.start + delimiter_len..source.end - delimiter_len;
    Section::new(kind, source, content)
}

fn block(kind: SectionKind, source: Range<usize>, text: &str, delimiter: &str) -> Section {
    let start = text.find('\n').map_or(text.len(), |index| index + 1);
    let without_final_newline = text.trim_end_matches(['\r', '\n']);
    let end = without_final_newline
        .rfind('\n')
        .and_then(|index| {
            without_final_newline[index + 1..]
                .trim_start()
                .starts_with(delimiter)
                .then(|| index - usize::from(index > 0 && text.as_bytes()[index - 1] == b'\r'))
        })
        .unwrap_or(text.len())
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

fn link(
    kind: SectionKind,
    source: Range<usize>,
    text: &str,
    prefix: usize,
    destination: String,
) -> Section {
    let end = text
        .rfind("](")
        .filter(|end| *end >= prefix)
        .unwrap_or(prefix);
    let mut section = Section::new(
        kind,
        source.clone(),
        source.start + prefix..source.start + end,
    );
    section.destination = Some(destination);
    section
}
