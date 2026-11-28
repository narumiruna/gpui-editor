use std::{borrow::Cow, ops::Range};

use crate::autocomplete::{
    Completion,
    session::{CompletionItem, SourceResult},
};

use super::{Side, XmlDocument, XmlNode, XmlNodeKind::*, trim_js_space};

/// The elements and attributes available to XML schema completion.
#[derive(Clone, Copy, Debug, Default)]
pub struct XmlSchema<'a> {
    pub elements: &'a [XmlElementSpec<'a>],
    pub attributes: &'a [XmlAttrSpec<'a>],
}

#[derive(Clone, Copy, Debug, Default)]
pub struct XmlElementSpec<'a> {
    pub name: &'a str,
    /// If absent, allow all elements. An empty slice allows no child elements.
    pub children: Option<&'a [&'a str]>,
    pub text_content: &'a [&'a str],
    pub top: bool,
    pub attributes: &'a [XmlAttribute<'a>],
    /// Optional metadata for the generated tag completion. The label is ignored.
    pub completion: Option<Completion<'a>>,
}

#[derive(Clone, Copy, Debug)]
pub enum XmlAttribute<'a> {
    Named(&'a str),
    Inline(XmlAttrSpec<'a>),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct XmlAttrSpec<'a> {
    pub name: &'a str,
    pub values: Option<&'a [XmlValue<'a>]>,
    pub global: bool,
    /// Optional metadata for the generated attribute completion. The label is ignored.
    pub completion: Option<Completion<'a>>,
}

#[derive(Clone, Copy, Debug)]
pub enum XmlValue<'a> {
    Text(&'a str),
    Completion(Completion<'a>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlCompletion<'a> {
    pub label: Cow<'a, str>,
    metadata: Completion<'a>,
}

impl<'a> XmlCompletion<'a> {
    fn new(
        label: impl Into<Cow<'a, str>>,
        kind: &'a str,
        metadata: Option<Completion<'a>>,
    ) -> Self {
        let mut metadata = metadata.unwrap_or_else(|| Completion::new(""));
        metadata.kind = metadata.kind.or(Some(kind));
        Self {
            label: label.into(),
            metadata,
        }
    }

    /// Borrow the candidate in the native autocomplete format.
    pub fn as_completion(&self) -> Completion<'_> {
        Completion {
            label: &self.label,
            ..self.metadata
        }
    }
}

/// The replacement range, candidates in source order, and the reuse rule.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct XmlCompletionResult<'a> {
    pub range: Range<usize>,
    pub options: Vec<XmlCompletion<'a>>,
    pub valid_for: XmlCompletionValidity,
}

/// The kind of text that lets the editor reuse a completion result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XmlCompletionValidity {
    Identifier,
    AttributeValue,
    Tag,
    Entity,
}

impl XmlCompletionValidity {
    pub fn matches(self, text: &str) -> bool {
        fn identifier(text: &str) -> bool {
            text.chars().all(|ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '-' | '.') || ch >= '\u{b7}'
            })
        }
        match self {
            Self::Identifier => identifier(text),
            Self::AttributeValue => text
                .strip_prefix('"')
                .is_some_and(|rest| !rest.strip_suffix('"').unwrap_or(rest).contains('"')),
            Self::Tag => text
                .strip_prefix('<')
                .is_some_and(|rest| identifier(rest.strip_prefix('/').unwrap_or(rest))),
            Self::Entity => text
                .strip_prefix('&')
                .is_some_and(|rest| rest.chars().all(|ch| ch.is_ascii_alphabetic())),
        }
    }
}

/// Run XML completion as a native autocomplete source, with optional schema data.
pub fn xml_completion_source(
    doc: &XmlDocument,
    position: usize,
    explicit: bool,
    schema: XmlSchema<'_>,
) -> Option<SourceResult> {
    completion_source(doc, position, doc.complete(position, explicit, schema)?)
}

// Pinned xml() does not complete predefined entities.
pub(crate) fn schema_completion_source(
    doc: &XmlDocument,
    position: usize,
    explicit: bool,
    schema: XmlSchema<'_>,
) -> Option<SourceResult> {
    doc.text.as_str().get(..position)?;
    let at = doc.path(position, Side::Left).last()?;
    let completed = doc.complete_schema(position, explicit, schema, at)?;
    completion_source(doc, position, completed)
}

fn completion_source(
    doc: &XmlDocument,
    position: usize,
    completed: XmlCompletionResult<'_>,
) -> Option<SourceResult> {
    let options: Vec<_> = completed
        .options
        .iter()
        .map(|item| CompletionItem::from(item.as_completion()))
        .collect();
    let validity = completed.valid_for;
    let mut result = SourceResult::new(completed.range.start, options)
        .with_valid_for(move |prefix, _, _| validity.matches(prefix));
    let at = doc.path(position, Side::Left).last()?;
    if matches!(
        validity,
        XmlCompletionValidity::AttributeValue | XmlCompletionValidity::Entity
    ) || find_location(at, position)
        .is_some_and(|(location, ..)| matches!(location, Location::CloseTag))
    {
        result.to = Some(completed.range.end);
    }
    Some(result)
}

#[derive(Clone, Copy)]
enum Location {
    OpenTag,
    CloseTag,
    AttrName,
    AttrValue,
    Tag,
}

impl XmlDocument {
    /// Complete tags and attributes from a schema, closing tags from the
    /// document, and the five predefined XML entities in text or quoted values.
    /// `explicit` enables suggestions in plain text without a tag prefix.
    /// An invalid byte position or unsupported context returns `None`.
    pub fn complete<'a>(
        &'a self,
        position: usize,
        explicit: bool,
        schema: XmlSchema<'a>,
    ) -> Option<XmlCompletionResult<'a>> {
        let prefix = self.text.as_str().get(..position)?;
        let at = self.path(position, Side::Left).last()?;
        if let Some(result) = self.complete_entity(prefix, at) {
            return Some(result);
        }
        self.complete_schema(position, explicit, schema, at)
    }

    fn complete_schema<'a>(
        &'a self,
        position: usize,
        explicit: bool,
        schema: XmlSchema<'a>,
        at: XmlNode<'a>,
    ) -> Option<XmlCompletionResult<'a>> {
        let (location, from, context) = find_location(at, position)?;
        let pos = position;
        let text = self.text.as_str();
        let element_name = context.and_then(|node| self.element_name(node));
        let mut to = pos;
        let (options, valid_for) = match location {
            Location::OpenTag => (
                schema
                    .children(element_name, context.is_none())
                    .into_iter()
                    .map(|spec| XmlCompletion::new(spec.name, "type", spec.completion))
                    .collect(),
                XmlCompletionValidity::Identifier,
            ),
            Location::CloseTag => {
                let name = element_name?;
                to += usize::from(text.as_bytes().get(pos) == Some(&b'>'));
                let metadata = schema.element(name).and_then(|spec| spec.completion);
                (
                    vec![XmlCompletion::new(format!("{name}>"), "type", metadata)],
                    XmlCompletionValidity::Identifier,
                )
            }
            Location::AttrName => {
                let tag = self
                    .tag_name(context?)
                    .and_then(|name| schema.element(name));
                let global = schema.attributes.iter().filter(|spec| spec.global).copied();
                let local =
                    tag.into_iter()
                        .flat_map(|tag| tag.attributes)
                        .map(|attr| match *attr {
                            XmlAttribute::Named(name) => schema
                                .attributes
                                .iter()
                                .find(|spec| spec.name == name)
                                .copied()
                                .unwrap_or(XmlAttrSpec {
                                    name,
                                    ..XmlAttrSpec::default()
                                }),
                            XmlAttribute::Inline(spec) => spec,
                        });
                (
                    global
                        .chain(local)
                        .map(|spec| XmlCompletion::new(spec.name, "property", spec.completion))
                        .collect(),
                    XmlCompletionValidity::Identifier,
                )
            }
            Location::AttrValue => {
                let tag = context?;
                let attr = tag.children().find(|child| {
                    child.kind() == Attribute
                        && (child.data().range.start..=child.data().range.end).contains(&from)
                })?;
                let name = text.get(
                    attr.children()
                        .find(|child| child.kind() == AttributeName)?
                        .range(),
                )?;
                let local = self
                    .tag_name(tag)
                    .and_then(|tag| schema.element(tag))
                    .and_then(|element| {
                        element
                            .attributes
                            .iter()
                            .rev()
                            .find_map(|attr| match *attr {
                                XmlAttribute::Inline(spec) if spec.name == name => spec.values,
                                _ => None,
                            })
                    });
                let values = local.or_else(|| {
                    schema
                        .attributes
                        .iter()
                        .rev()
                        .find_map(|spec| (spec.name == name).then_some(spec.values).flatten())
                })?;
                if values.is_empty() {
                    return None;
                }
                to += usize::from(text.as_bytes().get(pos) == Some(&b'"'));
                (
                    values.iter().copied().map(value_completion).collect(),
                    XmlCompletionValidity::AttributeValue,
                )
            }
            Location::Tag => {
                if !explicit {
                    return None;
                }
                let mut options = Vec::new();
                let spec = element_name.and_then(|name| schema.element(name));
                if let Some(name) = element_name
                    && !context
                        .and_then(|node| node.children().next_back())
                        .is_some_and(|last| {
                            last.kind() == CloseTag && self.tag_name(last) == Some(name)
                        })
                {
                    let mut close = XmlCompletion::new(
                        format!("</{name}>"),
                        "type",
                        spec.and_then(|s| s.completion),
                    );
                    close.metadata.boost = 2;
                    options.push(close);
                }
                options.extend(
                    schema
                        .children(element_name, context.is_none())
                        .into_iter()
                        .map(|spec| {
                            XmlCompletion::new(format!("<{}", spec.name), "type", spec.completion)
                        }),
                );
                if let (Some(spec), Some(open)) =
                    (spec, context.and_then(|node| node.children().next()))
                    && let Some(before) = text.get(open.data().range.end..pos)
                    && before.encode_utf16().take(20).count() < 20
                    && trim_js_space(before).is_empty()
                {
                    options.extend(
                        spec.text_content
                            .iter()
                            .map(|&text| XmlCompletion::new(text, "text", None)),
                    );
                }
                (options, XmlCompletionValidity::Tag)
            }
        };
        Some(XmlCompletionResult {
            range: from..to,
            options,
            valid_for,
        })
    }

    fn tag_name(&self, tag: XmlNode<'_>) -> Option<&str> {
        self.text.slice(
            tag.children()
                .find(|child| child.kind() == TagName)?
                .range(),
        )
    }

    fn element_name(&self, element: XmlNode<'_>) -> Option<&str> {
        self.tag_name(
            element
                .children()
                .next()
                .filter(|node| node.kind() == OpenTag)?,
        )
    }

    // The pinned upstream source has no entity completion. Provide predefined
    // names only, without reading a DTD or interpreting any reference.
    fn complete_entity(
        &self,
        prefix: &str,
        at: XmlNode<'_>,
    ) -> Option<XmlCompletionResult<'static>> {
        let parent = at.parent()?.kind();
        if !matches!(
            (at.kind(), parent),
            (Text | EntityReference | Error, Document | Element)
                | (EntityReference | Error, AttributeValue)
        ) && at.kind() != AttributeValue
        {
            return None;
        }
        const ENTITIES: [&str; 5] = ["&amp;", "&apos;", "&gt;", "&lt;", "&quot;"];
        // An unfinished predefined name has at most five ASCII bytes including `&`.
        let pos = prefix.len();
        let tail = &prefix.as_bytes()[pos.saturating_sub(5)..];
        let start = pos - tail.len() + tail.iter().rposition(|&byte| byte == b'&')?;
        let partial = &prefix[start..];
        if !ENTITIES
            .iter()
            .any(|label| label.trim_end_matches(';').starts_with(partial))
        {
            return None;
        }
        let options = ENTITIES
            .into_iter()
            .map(|label| XmlCompletion::new(label, "constant", None))
            .collect();
        let to = pos + usize::from(self.text.as_str().as_bytes().get(pos) == Some(&b';'));
        Some(XmlCompletionResult {
            range: start..to,
            options,
            valid_for: XmlCompletionValidity::Entity,
        })
    }
}

impl<'a> XmlSchema<'a> {
    fn element(self, name: &str) -> Option<XmlElementSpec<'a>> {
        self.elements
            .iter()
            .rev()
            .find(|spec| spec.name == name)
            .copied()
    }

    fn children(self, context: Option<&str>, top: bool) -> Vec<XmlElementSpec<'a>> {
        if let Some(names) = context
            .and_then(|name| self.element(name))
            .and_then(|spec| spec.children)
        {
            names.iter().filter_map(|name| self.element(name)).collect()
        } else {
            let marked = top && self.elements.iter().any(|spec| spec.top);
            self.elements
                .iter()
                .filter(|spec| !marked || spec.top)
                .copied()
                .collect()
        }
    }
}

fn value_completion(value: XmlValue<'_>) -> XmlCompletion<'_> {
    match value {
        XmlValue::Text(text) => XmlCompletion::new(format!("\"{text}\""), "constant", None),
        XmlValue::Completion(completion) => XmlCompletion {
            label: if completion.label.starts_with('"') {
                completion.label.into()
            } else {
                format!("\"{}\"", completion.label).into()
            },
            metadata: completion,
        },
    }
}

fn find_parent_element(node: XmlNode<'_>) -> Option<XmlNode<'_>> {
    std::iter::successors(node.parent(), |node| node.parent()).find(|n| n.kind() == Element)
}

fn find_location(
    mut at: XmlNode<'_>,
    pos: usize,
) -> Option<(Location, usize, Option<XmlNode<'_>>)> {
    let tag = std::iter::successors(Some(at), |node| node.parent()).find(|node| {
        matches!(
            node.kind(),
            OpenTag | CloseTag | SelfClosingTag | MismatchedCloseTag
        )
    });
    if let Some(tag) = tag.filter(|tag| tag.data().range.end > pos || tag.ends_in_error()) {
        let element = tag.parent()?;
        let start = at.data().range.start;
        let parent = find_parent_element(element);
        return match at.kind() {
            TagName if matches!(tag.kind(), CloseTag | MismatchedCloseTag) => {
                Some((Location::CloseTag, start, Some(element)))
            }
            TagName => Some((Location::OpenTag, start, parent)),
            AttributeName => Some((Location::AttrName, start, Some(tag))),
            AttributeValue => Some((Location::AttrValue, start, Some(tag))),
            _ => {
                let before = if at.id == tag.id || at.kind() == Attribute {
                    at.child_before(pos)?
                } else {
                    at
                };
                Some(match before.kind() {
                    StartTag => (Location::OpenTag, pos, parent),
                    StartCloseTag if before.data().range.end <= pos => {
                        (Location::CloseTag, pos, Some(element))
                    }
                    Is => (Location::AttrValue, pos, Some(tag)),
                    _ => (Location::AttrName, pos, Some(tag)),
                })
            }
        };
    } else if at.kind() == StartCloseTag {
        return Some((Location::CloseTag, pos, at.parent()));
    }
    while let Some(parent) = at
        .parent()
        .filter(|_| at.data().range.end == pos && !at.ends_in_error())
    {
        at = parent;
    }
    match at.kind() {
        Element => Some((Location::Tag, pos, Some(at))),
        Text | Document => Some((Location::Tag, pos, find_parent_element(at))),
        _ => None,
    }
}
