use std::{borrow::Cow, ops::Range};

pub use pulldown_cmark::Alignment;
use pulldown_cmark::{CodeBlockKind, Event, LinkType, Options, Parser, Tag, TagEnd};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SectionKind {
    Heading,
    Bullet,
    Strong,
    Emphasis,
    InlineCode,
    FrontMatter,
    Mermaid,
    InlineMath,
    DisplayMath,
    Image,
    Link,
    DraftCode,
    ReadOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Heading {
    /// Markdown heading level, from 1 through 6.
    pub level: u8,
    /// Plain text from the heading's inline Markdown. May be empty.
    pub label: String,
    /// UTF-8 byte offset of the heading's start in the source.
    pub offset: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub kind: SectionKind,
    pub source: Range<usize>,
    pub content: Range<usize>,
    pub destination: Option<String>,
    pub preview: Option<BlockPreview>,
    /// Normalized inline code when the preview does not match the source content.
    pub inline_code: Option<Box<str>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BlockPreview {
    Code {
        body: String,
        language: Option<String>,
    },
    Table {
        alignments: Vec<Alignment>,
        rows: Vec<Vec<String>>,
    },
}

impl BlockPreview {
    fn consume(&mut self, event: Event<'_>) -> bool {
        match (self, event) {
            (Self::Code { body, .. }, Event::Text(text)) => body.push_str(&text),
            (Self::Table { rows, .. }, Event::Start(Tag::TableHead | Tag::TableRow)) => {
                rows.push(Vec::new());
            }
            (Self::Table { rows, .. }, Event::Start(Tag::TableCell)) => {
                rows.last_mut().unwrap().push(String::new());
            }
            (
                Self::Table { rows, .. },
                Event::Text(text)
                | Event::Code(text)
                | Event::InlineMath(text)
                | Event::DisplayMath(text)
                | Event::InlineHtml(text),
            ) => rows.last_mut().unwrap().last_mut().unwrap().push_str(&text),
            (Self::Table { rows, .. }, Event::SoftBreak | Event::HardBreak) => {
                rows.last_mut().unwrap().last_mut().unwrap().push(' ');
            }
            (Self::Code { .. }, Event::End(TagEnd::CodeBlock))
            | (Self::Table { .. }, Event::End(TagEnd::Table)) => return true,
            _ => {}
        }
        false
    }
}

impl Section {
    pub fn new(kind: SectionKind, source: Range<usize>, content: Range<usize>) -> Self {
        Self {
            kind,
            source,
            content,
            destination: None,
            preview: None,
            inline_code: None,
        }
    }

    fn with_preview(source: Range<usize>, preview: BlockPreview) -> Self {
        let mut section = Self::new(SectionKind::ReadOnly, source.clone(), source);
        section.preview = Some(preview);
        section
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

    /// Return headings in source order, using the same Markdown rules as the editor.
    pub fn headings(&self) -> Vec<Heading> {
        Markdown::new(&self.source).headings()
    }

    pub fn sections(&self) -> Vec<Section> {
        Markdown::new(&self.source).sections()
    }

    pub fn replace(&mut self, range: Range<usize>, text: &str) -> Result<(), EditError> {
        self.replacement(range, text, None).map(|_| ())
    }

    /// Apply the edit and return the actual old range and inserted byte count.
    /// Place the caret after `text`, before any added fence separator.
    /// An active code draft permits edits to its own code preview, not other read-only sections.
    pub fn replacement(
        &mut self,
        range: Range<usize>,
        text: &str,
        active: Option<&Section>,
    ) -> Result<(Range<usize>, usize), EditError> {
        let (range, text) = Markdown::new(&self.source).plan_replacement(range, text, active)?;
        self.source.replace_range(range.clone(), &text);
        Ok((range, text.len()))
    }
}

/// The range and text after applying Markdown edit rules.
pub type PlannedReplacement<'text> = (Range<usize>, Cow<'text, str>);

/// Parse Markdown and plan edits to its borrowed source.
pub struct Markdown<'a> {
    source: &'a str,
}

impl<'a> Markdown<'a> {
    pub fn new(source: &'a str) -> Self {
        Self { source }
    }

    fn parser(&self) -> Parser<'a> {
        Parser::new_ext(
            self.source,
            Options::ENABLE_MATH
                | Options::ENABLE_TABLES
                | Options::ENABLE_YAML_STYLE_METADATA_BLOCKS
                | Options::ENABLE_STRIKETHROUGH,
        )
    }

    /// Return headings in source order, using the same Markdown rules as the editor.
    pub fn headings(&self) -> Vec<Heading> {
        let mut headings = Vec::new();
        let mut current: Option<Heading> = None;
        for (event, source) in self.parser().into_offset_iter() {
            match event {
                Event::Start(Tag::Heading { level, .. }) => {
                    current = Some(Heading {
                        level: level as u8,
                        label: String::new(),
                        offset: source.start,
                    });
                }
                Event::End(TagEnd::Heading(_)) => {
                    let mut heading = current.take().unwrap();
                    heading.label = heading
                        .label
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    headings.push(heading);
                }
                _ => {
                    let Some(heading) = &mut current else {
                        continue;
                    };
                    match event {
                        Event::Text(text)
                        | Event::Code(text)
                        | Event::InlineMath(text)
                        | Event::DisplayMath(text) => heading.label.push_str(&text),
                        Event::SoftBreak | Event::HardBreak => heading.label.push(' '),
                        _ => {}
                    }
                }
            }
        }
        headings
    }

    pub fn sections(&self) -> Vec<Section> {
        use SectionKind as K;

        let mut sections: Vec<Section> = Vec::new();
        let mut open_links: Vec<usize> = Vec::new();
        let mut heading: Option<Section> = None;
        let mut fenced_body = None;
        let mut block_preview: Option<(Section, Option<usize>)> = None;
        for (event, source) in self.parser().into_offset_iter() {
            if let Some((section, body_end)) = &mut block_preview {
                if let Some(end) = body_end
                    && matches!(&event, Event::Text(_))
                {
                    *end = (*end).max(source.end);
                }
                if section.preview.as_mut().unwrap().consume(event) {
                    let (section, body_end) = block_preview.take().unwrap();
                    let range = &section.source;
                    if body_end.is_some_and(|end| {
                        !has_closing_code_fence(&self.source[range.clone()], end - range.start)
                    }) {
                        sections.push(Section::new(K::DraftCode, range.clone(), range.clone()));
                    } else {
                        sections.push(section);
                    }
                }
                continue;
            }
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
                Event::Code(code) => {
                    let ticks = text.bytes().take_while(|&byte| byte == b'`').count();
                    let mut section = delimited(K::InlineCode, source, ticks);
                    let body = &self.source[section.content.clone()];
                    if body != code.as_ref() {
                        if body
                            .strip_prefix(' ')
                            .and_then(|body| body.strip_suffix(' '))
                            == Some(code.as_ref())
                        {
                            section.content.start += 1;
                            section.content.end -= 1;
                        } else {
                            section.inline_code = Some(code.into_string().into_boxed_str());
                        }
                    }
                    Some(section)
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
                Event::Start(Tag::CodeBlock(kind)) => {
                    let body_end = matches!(kind, CodeBlockKind::Fenced(_)).then_some(source.start);
                    let language = match kind {
                        CodeBlockKind::Fenced(info) => {
                            info.split_whitespace().next().map(str::to_owned)
                        }
                        CodeBlockKind::Indented => None,
                    };
                    block_preview = Some((
                        Section::with_preview(
                            source,
                            BlockPreview::Code {
                                body: String::new(),
                                language,
                            },
                        ),
                        body_end,
                    ));
                    None
                }
                Event::Start(Tag::Table(alignments)) => {
                    block_preview = Some((
                        Section::with_preview(
                            source,
                            BlockPreview::Table {
                                alignments,
                                rows: Vec::new(),
                            },
                        ),
                        None,
                    ));
                    None
                }
                Event::Text(_) => {
                    if let Some((_, end)) = &mut fenced_body {
                        *end = (*end).max(source.end);
                    }
                    None
                }
                Event::End(TagEnd::CodeBlock | TagEnd::MetadataBlock(_)) => {
                    fenced_body.take().map(|(kind, body_end)| {
                        if kind == K::Mermaid
                            && !has_closing_code_fence(text, body_end - source.start)
                        {
                            Section::new(K::DraftCode, source.clone(), source)
                        } else {
                            block(kind, source, text, body_end)
                        }
                    })
                }
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
                    Tag::BlockQuote(_)
                    | Tag::List(Some(_))
                    | Tag::Strikethrough
                    | Tag::HtmlBlock
                    | Tag::Link { .. }
                    | Tag::Image { .. },
                )
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

    /// Return the actual range and replacement text without changing the source.
    /// The range uses UTF-8 byte offsets. The result borrows `text` unless a fence separator is added.
    /// An active code draft permits edits to its own code preview, not other read-only sections.
    pub fn plan_replacement<'text>(
        &self,
        range: Range<usize>,
        text: &'text str,
        active: Option<&Section>,
    ) -> Result<PlannedReplacement<'text>, EditError> {
        self.plan(&self.sections(), range, text, active)
    }

    /// Plan multiple edits against the same source using one parse.
    /// The caller must check the planned ranges for overlap before applying them.
    pub fn plan_replacements<'text>(
        &self,
        edits: impl IntoIterator<Item = (Range<usize>, &'text str)>,
    ) -> Result<Vec<PlannedReplacement<'text>>, EditError> {
        let sections = self.sections();
        edits
            .into_iter()
            .map(|(range, text)| self.plan(&sections, range, text, None))
            .collect()
    }

    fn plan<'text>(
        &self,
        sections: &[Section],
        mut range: Range<usize>,
        text: &'text str,
        active: Option<&Section>,
    ) -> Result<PlannedReplacement<'text>, EditError> {
        use SectionKind as K;

        if self.source.get(range.clone()).is_none() {
            return Err(EditError::InvalidRange);
        }
        if active.is_none() && text.is_empty() && !range.is_empty() {
            // Sections are sorted outermost first. Remove inner spans first so
            // an enclosing span also goes away if it becomes empty.
            for section in sections.iter().rev() {
                if matches!(section.kind, K::Strong | K::Emphasis | K::InlineCode)
                    && range.start <= section.content.start
                    && section.content.end <= range.end
                {
                    range.start = range.start.min(section.source.start);
                    range.end = range.end.max(section.source.end);
                }
            }
        }
        let draft = active.filter(|section| {
            section.kind == K::DraftCode
                && section.source.start <= range.start
                && range.end <= section.source.end
        });
        if sections.iter().any(|section| {
            section.kind == K::ReadOnly
                && range.start < section.source.end
                && section.source.start < range.end
                && !(matches!(section.preview, Some(BlockPreview::Code { .. }))
                    && draft.is_some_and(|draft| draft.source == section.source))
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
        let replacement = if needs_separator {
            let ending = if self.source[..range.start].ends_with("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            Cow::Owned(format!("{text}{ending}"))
        } else {
            Cow::Borrowed(text)
        };
        Ok((range, replacement))
    }
}

fn delimited(kind: SectionKind, source: Range<usize>, delimiter_len: usize) -> Section {
    let content = source.start + delimiter_len..source.end - delimiter_len;
    Section::new(kind, source, content)
}

fn fence_body_start(text: &str) -> usize {
    text.find('\n').map_or(text.len(), |index| index + 1)
}

fn has_closing_code_fence(text: &str, body_end: usize) -> bool {
    let bytes = text.as_bytes();
    let len = bytes.iter().take_while(|&&byte| byte == bytes[0]).count();
    // The parser also ends an unclosed block at EOF. A closing fence is outside its text events.
    bytes[body_end.max(fence_body_start(text))..]
        .trim_ascii_end()
        .ends_with(&bytes[..len])
}

fn block(kind: SectionKind, source: Range<usize>, text: &str, body_end: usize) -> Section {
    let start = fence_body_start(text);
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
