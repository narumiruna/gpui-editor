//! Native completion sources from `@codemirror/lang-html` 6.4.12.
//! See `docs/licenses/codemirror-lang-html.txt`.

use std::{collections::HashSet, ops::Range};

use crate::autocomplete::session::{CompletionItem, SourceResult};

use super::{HtmlDocument, HtmlNode, HtmlNodeKind::*, trim_js_space};

mod schema;

use schema::{GLOBAL_ATTRIBUTES, TAGS};

/// Add or replace definitions in the built-in completion schema.
#[derive(Clone, Copy, Debug, Default)]
pub struct HtmlCompletionConfig<'a> {
    pub extra_tags: &'a [HtmlTagSpec<'a>],
    pub extra_global_attributes: &'a [HtmlAttributeSpec<'a>],
}

/// Completion schema for a tag. This does not change the parser.
#[derive(Clone, Copy, Debug)]
pub struct HtmlTagSpec<'a> {
    pub name: &'a str,
    pub attributes: &'a [HtmlAttributeSpec<'a>],
    /// If absent, allow all tags. An empty slice allows no child tags.
    pub children: Option<&'a [&'a str]>,
    /// Include global names in attribute name completion. Defaults to true.
    pub global_attrs: bool,
}

impl<'a> HtmlTagSpec<'a> {
    pub const fn new(name: &'a str) -> Self {
        Self {
            name,
            attributes: &[],
            children: None,
            global_attrs: true,
        }
    }
}

/// An attribute and its suggested values, in source order.
#[derive(Clone, Copy, Debug)]
pub struct HtmlAttributeSpec<'a> {
    pub name: &'a str,
    /// Use `None` for an attribute with no fixed set of values.
    pub values: Option<&'a [&'a str]>,
}

impl<'a> HtmlAttributeSpec<'a> {
    pub const fn new(name: &'a str, values: Option<&'a [&'a str]>) -> Self {
        Self { name, values }
    }
}

/// Return HTML options in source order at a UTF-8 byte position.
/// The replacement range ends at the query position. Out-of-bounds positions,
/// positions inside a UTF-8 character, and mounted language regions return `None`.
/// Completion uses the document's parser dialect and recovery rules.
pub fn html_completion_source(
    doc: &HtmlDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    complete(doc, position, explicit, TAGS, GLOBAL_ATTRIBUTES)
}

/// Complete the stock schema after the installed facet has selected HTML.
pub(crate) fn html_schema_completion_source(
    doc: &HtmlDocument,
    position: usize,
    explicit: bool,
    name_is_mapped: &dyn Fn(Range<usize>) -> bool,
) -> Option<SourceResult> {
    complete_schema(
        doc,
        position,
        explicit,
        TAGS,
        GLOBAL_ATTRIBUTES,
        name_is_mapped,
    )
}

impl HtmlDocument {
    /// Complete a mounted `TagName` with the stock HTML schema. `value` is
    /// the host value node, which supplies the enclosing element ancestry.
    pub(crate) fn mounted_tag_name_completion(
        &self,
        value: HtmlNode<'_>,
        from: usize,
        to: usize,
    ) -> SourceResult {
        tag_result(self.text().as_str(), value, true, from, to, TAGS, &|_| true)
    }
}

/// Build a completion source with extra tags and global attributes.
/// Names are case-sensitive. A replacement definition keeps the original key's position.
pub fn html_completion_source_with(
    config: HtmlCompletionConfig<'_>,
) -> impl Fn(&HtmlDocument, usize, bool) -> Option<SourceResult> {
    let tags = merge(TAGS, config.extra_tags, |tag| tag.name);
    let globals = merge(GLOBAL_ATTRIBUTES, config.extra_global_attributes, |attr| {
        attr.name
    });
    move |doc, position, explicit| complete(doc, position, explicit, &tags, &globals)
}

fn merge<'a, T: Copy>(base: &[T], extra: &[T], name: impl Fn(&T) -> &'a str) -> Vec<T> {
    let mut result = base.to_vec();
    for item in extra {
        if let Some(existing) = result.iter_mut().find(|value| name(value) == name(item)) {
            *existing = *item;
        } else {
            result.push(*item);
        }
    }
    // JavaScript enumerates array-index keys before the other object keys.
    result.sort_by_key(|value| {
        let key = name(value);
        key.parse::<u32>()
            .ok()
            .filter(|&index| index != u32::MAX && index.to_string() == key)
            .unwrap_or(u32::MAX)
    });
    result
}

fn complete(
    doc: &HtmlDocument,
    pos: usize,
    explicit: bool,
    tags: &[HtmlTagSpec<'_>],
    globals: &[HtmlAttributeSpec<'_>],
) -> Option<SourceResult> {
    let prefix = doc.text().as_str().get(..pos)?;
    let mount = doc
        .mounts()
        .partition_point(|mount| mount.range().end < pos);
    if doc.mounts().get(mount).is_some_and(|mount| {
        let start = mount.range().start;
        // Raw bodies and quoted values are bracketed in Lezer.
        start < pos || start == pos && prefix.ends_with(['>', '\'', '"'])
    }) {
        return None;
    }
    complete_schema(doc, pos, explicit, tags, globals, &|_| true)
}

fn complete_schema(
    doc: &HtmlDocument,
    pos: usize,
    explicit: bool,
    tags: &[HtmlTagSpec<'_>],
    globals: &[HtmlAttributeSpec<'_>],
    name_is_mapped: &dyn Fn(Range<usize>) -> bool,
) -> Option<SourceResult> {
    let source = doc.text().as_str();
    source.get(..pos)?;
    let mut node = resolve_left(doc.root(), pos);
    if let Some(tag) = trailing_open_tag(node, source, pos) {
        node = tag;
    }
    let mut around = node;
    while around.parent().is_some() && around.range().end == pos {
        around = around.parent()?;
    }
    if node.id == around.id {
        // Across whitespace, enter nodes that end in a missing token.
        let mut scan = pos;
        while let Some(before) = node.children().rfind(|child| child.range().start < scan) {
            let Some(last) = before
                .children()
                .next_back()
                .filter(|last| last.kind() == Error && last.range().is_empty())
            else {
                break;
            };
            node = before;
            around = before;
            scan = last.range().start;
        }
    }

    let start = node.range().start;
    let tag = |name: &str| tags.iter().find(|spec| spec.name == name);
    match node.kind() {
        TagName
            if node
                .parent()
                .is_some_and(|parent| matches!(parent.kind(), CloseTag | MismatchedCloseTag)) =>
        {
            Some(close_tag_result(source, node, start, pos, name_is_mapped))
        }
        StartCloseTag | IncompleteCloseTag => {
            Some(close_tag_result(source, node, pos, pos, name_is_mapped))
        }
        StartTag | IncompleteTag | TagName => Some(tag_result(
            source,
            node,
            node.kind() != IncompleteTag,
            if node.kind() == TagName { start } else { pos },
            pos,
            tags,
            name_is_mapped,
        )),
        OpenTag | SelfClosingTag | AttributeName => {
            let spec = element_name(source, parent_element(node, false)).and_then(tag);
            let local = spec.map_or(&[][..], |tag| tag.attributes);
            let global = if spec.is_none_or(|tag| tag.global_attrs) {
                globals
            } else {
                &[]
            };
            let items = merge(&[], local, |attr| attr.name)
                .iter()
                .chain(global)
                .map(|attr| item(attr.name, "property"))
                .collect();
            let from = if node.kind() == AttributeName {
                start
            } else {
                pos
            };
            Some(result(from, pos, items, identifier))
        }
        Is | AttributeValue | UnquotedAttributeValue => {
            let from = if node.kind() == Is { pos } else { start };
            let values = node
                .parent()
                .and_then(|parent| parent.child(AttributeName))
                .and_then(|name| source.get(name.range()))
                .and_then(|name| {
                    let local = element_name(source, parent_element(node, false))
                        .and_then(tag)
                        .map_or(&[][..], |tag| tag.attributes);
                    attribute_values(globals, name).or_else(|| attribute_values(local, name))
                });
            Some(value_result(source, from, pos, values))
        }
        _ if explicit && matches!(around.kind(), Element | Text | Document) => {
            let parent = parent_element(node, false);
            let mut items: Vec<_> = allowed_children(tags, element_name(source, parent))
                .into_iter()
                .map(|name| item(format!("<{name}"), "type"))
                .collect();
            items.extend(
                open_tags(source, node, name_is_mapped)
                    .into_iter()
                    .enumerate()
                    .map(|(i, name)| boosted(item(format!("</{name}>"), "type"), i)),
            );
            Some(result(pos, pos, items, |text| {
                text.strip_prefix('<')
                    .is_some_and(|name| identifier(name.strip_prefix('/').unwrap_or(name)))
            }))
        }
        _ => None,
    }
}

fn allowed_children<'a>(tags: &[HtmlTagSpec<'a>], parent: Option<&str>) -> Vec<&'a str> {
    let child_names = parent
        .and_then(|name| tags.iter().find(|tag| tag.name == name))
        .and_then(|tag| tag.children);
    child_names.map_or_else(|| tags.iter().map(|tag| tag.name).collect(), <[_]>::to_vec)
}

fn tag_result(
    source: &str,
    node: HtmlNode<'_>,
    skip_element: bool,
    from: usize,
    to: usize,
    tags: &[HtmlTagSpec<'_>],
    name_is_mapped: &dyn Fn(Range<usize>) -> bool,
) -> SourceResult {
    let parent = parent_element(node, skip_element);
    let mut items: Vec<_> = allowed_children(tags, element_name(source, parent))
        .into_iter()
        .map(|name| item(name, "type"))
        .collect();
    items.extend(closing_items(source, node, to, "/", name_is_mapped));
    result(from, to, items, |text| {
        identifier(text.strip_prefix('/').unwrap_or(text))
    })
}

fn resolve_left(mut node: HtmlNode<'_>, pos: usize) -> HtmlNode<'_> {
    loop {
        let children = &node.data().children;
        let index = children.partition_point(|&id| node.nodes[id].range.end < pos);
        let Some(&id) = children.get(index) else {
            return node;
        };
        if node.nodes[id].range.start >= pos {
            return node;
        }
        node = HtmlNode { id, ..node };
    }
}

// Local recovery ends an unfinished tag before trailing whitespace. Lezer
// keeps that gap in the tag through the next token boundary.
fn trailing_open_tag<'a>(mut node: HtmlNode<'a>, source: &str, pos: usize) -> Option<HtmlNode<'a>> {
    while matches!(node.kind(), Document | Element) {
        node = node.children().rfind(|child| child.range().start < pos)?;
    }
    let end = node.range().end;
    (matches!(node.kind(), OpenTag | SelfClosingTag)
        && end < pos
        && node
            .children()
            .next_back()
            .is_some_and(|last| last.kind() == Error && last.range().is_empty())
        && super::lexer::space_len(&source[end..pos]) == pos - end)
        .then_some(node)
}

fn parent_element(node: HtmlNode<'_>, skip: bool) -> Option<HtmlNode<'_>> {
    std::iter::successors(Some(node), |node| node.parent())
        .filter(|node| node.kind() == Element)
        .nth(usize::from(skip))
}

fn element_name<'a>(source: &'a str, element: Option<HtmlNode<'_>>) -> Option<&'a str> {
    source.get(element?.children().next()?.child(TagName)?.range())
}

fn open_tags<'a>(
    source: &'a str,
    node: HtmlNode<'_>,
    name_is_mapped: &dyn Fn(Range<usize>) -> bool,
) -> Vec<&'a str> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    for parent in std::iter::successors(parent_element(node, false), |node| {
        parent_element(node.parent()?, false)
    }) {
        let Some(open) = parent.children().next() else {
            continue;
        };
        let Some(range) = open.child(TagName).map(|name| name.range()) else {
            continue;
        };
        if parent
            .children()
            .next_back()
            .is_some_and(|last| last.kind() == CloseTag)
        {
            break;
        }
        if (node.kind() == EndTag || node.range().start >= open.range().end)
            && name_is_mapped(range.clone())
            && let Some(name) = source.get(range)
            && seen.insert(name)
        {
            names.push(name);
        }
    }
    names
}

fn closing_items<'a>(
    source: &'a str,
    node: HtmlNode<'_>,
    pos: usize,
    prefix: &'a str,
    name_is_mapped: &dyn Fn(Range<usize>) -> bool,
) -> impl Iterator<Item = CompletionItem> + use<'a> {
    // The upstream test only searches the first five UTF-16 units after the cursor.
    let end = if source[pos..]
        .encode_utf16()
        .take(5)
        .any(|unit| unit == u16::from(b'>'))
    {
        ""
    } else {
        ">"
    };
    open_tags(source, node, name_is_mapped)
        .into_iter()
        .enumerate()
        .map(move |(i, name)| {
            let mut option = item(format!("{prefix}{name}"), "type");
            option.insert = Some(format!("{}{end}", option.label));
            boosted(option, i)
        })
}

fn close_tag_result(
    source: &str,
    node: HtmlNode<'_>,
    from: usize,
    pos: usize,
    name_is_mapped: &dyn Fn(Range<usize>) -> bool,
) -> SourceResult {
    result(
        from,
        pos,
        closing_items(source, node, pos, "", name_is_mapped).collect(),
        identifier,
    )
}

fn attribute_values<'a>(attrs: &[HtmlAttributeSpec<'a>], name: &str) -> Option<&'a [&'a str]> {
    attrs.iter().rev().find(|attr| attr.name == name)?.values
}

fn value_result(source: &str, from: usize, to: usize, values: Option<&[&str]>) -> SourceResult {
    let Some(values) = values else {
        let mut result = SourceResult::new(from, []);
        result.to = Some(to);
        return result;
    };
    let quote = match source[from..to].chars().next() {
        Some(quote @ ('\'' | '"')) => Some(quote),
        _ => None,
    };
    let start_quote = if quote.is_some() { "" } else { "\"" };
    let end_quote = match quote {
        Some(quote) if source[to..].starts_with(quote) => "",
        Some('\'') => "'",
        _ => "\"",
    };
    let items = values
        .iter()
        .map(|value| {
            let mut option = item(*value, "constant");
            option.insert = Some(format!("{start_quote}{value}{end_quote}"));
            option
        })
        .collect();
    result(
        from + usize::from(quote.is_some()),
        to,
        items,
        move |text| {
            if let Some(quote) = quote {
                !text.contains(quote)
            } else {
                !text.chars().any(|ch| {
                    matches!(ch, '<' | '>' | '=' | '\'' | '"')
                        || trim_js_space(ch.encode_utf8(&mut [0; 4])).is_empty()
                })
            }
        },
    )
}

fn identifier(text: &str) -> bool {
    text.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, ':' | '-' | '.' | '_') || ch >= '\u{b7}'
    })
}

fn item(label: impl Into<String>, kind: &str) -> CompletionItem {
    let mut item = CompletionItem::new(label);
    item.kind = Some(kind.into());
    item
}

fn boosted(mut item: CompletionItem, index: usize) -> CompletionItem {
    item.boost = 99 - i32::try_from(index).unwrap_or(i32::MAX);
    item
}

fn result(
    from: usize,
    to: usize,
    items: Vec<CompletionItem>,
    valid: impl Fn(&str) -> bool + Send + Sync + 'static,
) -> SourceResult {
    let mut result = SourceResult::new(from, items).with_valid_for(move |text, _, _| valid(text));
    result.to = Some(to);
    result
}
