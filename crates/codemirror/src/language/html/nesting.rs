use std::{any::Any, collections::BTreeMap, fmt::Debug, ops::Range};

use crate::{Assoc, Text};

use super::{
    super::{
        IndentContext,
        css::CssDocument,
        javascript::{JavascriptDocument, JavascriptOptions},
        json::JsonDocument,
    },
    HighlightSpan, HtmlDocument, HtmlNode,
    HtmlNodeKind::*,
    IndentOptions, StreamSettings, lexer,
};

/// Opening tag attributes passed to a body rule's predicate.
/// Names keep their source spelling. Values exclude quotes but keep entity
/// references as written. A boolean attribute has an empty value. The last
/// occurrence of a duplicate name wins.
pub type HtmlAttributes<'a> = BTreeMap<&'a str, &'a str>;

/// Extra rules to try before the default CodeMirror rules.
#[derive(Clone, Copy, Default)]
pub struct HtmlNesting<'a> {
    pub nested_languages: &'a [HtmlNestedLanguage<'a>],
    pub nested_attributes: &'a [HtmlNestedAttribute<'a>],
}

/// Route the content of a tag. The first rule whose predicate accepts wins.
#[derive(Clone, Copy)]
pub struct HtmlNestedLanguage<'a> {
    /// Lowercase `script`, `style`, and `textarea` select raw syntax nodes.
    /// Other names match the exact spelling of the opening tag.
    pub tag: &'a str,
    /// `None` accepts all attributes.
    pub attrs: Option<&'a dyn Fn(&HtmlAttributes<'_>) -> bool>,
    pub parser: HtmlNestedParser<'a>,
}

/// Route an attribute value. The first matching name and tag scope wins.
#[derive(Clone, Copy)]
pub struct HtmlNestedAttribute<'a> {
    /// Match the exact spelling of the attribute name.
    pub name: &'a str,
    /// Match the exact spelling of the opening tag. `None` or an empty name
    /// matches any tag.
    pub tag_name: Option<&'a str>,
    pub parser: HtmlNestedParser<'a>,
}

/// A parser with a name that identifies its mounts.
#[derive(Clone, Copy)]
pub struct HtmlNestedParser<'a> {
    pub name: &'static str,
    /// Parse only the mounted text. The resulting document owns its data.
    pub parse: &'a dyn Fn(Text) -> Box<dyn HtmlCustomDocument>,
}

/// A native child tree with optional language services. Offsets use UTF-8 bytes
/// relative to the start of the mount.
pub trait HtmlCustomDocument: Any + Debug + Send + Sync {
    /// Return non-overlapping highlight spans for the requested range.
    fn highlights(&self, _range: Range<usize>) -> Vec<HighlightSpan<'_>> {
        Vec::new()
    }

    /// Find a fold that starts on the supplied line and continues past it.
    fn foldable(&self, _line_start: usize, _line_end: usize) -> Option<Range<usize>> {
        None
    }

    /// Compute indentation in columns, or return `None` to use the host rule.
    fn indentation(
        &self,
        _position: usize,
        _settings: StreamSettings,
        _options: IndentOptions<'_>,
    ) -> Option<usize> {
        None
    }
}

impl dyn HtmlCustomDocument {
    /// Borrow the concrete native child tree.
    pub fn downcast_ref<D: HtmlCustomDocument>(&self) -> Option<&D> {
        (self as &dyn Any).downcast_ref()
    }
}

macro_rules! custom_document {
    ($($document:ty),+ $(,)?) => {$(
        impl HtmlCustomDocument for $document {
            fn highlights(&self, range: Range<usize>) -> Vec<HighlightSpan<'_>> {
                self.highlights(range).into_iter().flatten().collect()
            }

            fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
                self.foldable(line_start, line_end)
            }

            fn indentation(
                &self,
                position: usize,
                settings: StreamSettings,
                options: IndentOptions<'_>,
            ) -> Option<usize> {
                self.indentation(position, settings, options)
            }
        }
    )+};
}

custom_document!(CssDocument, JavascriptDocument, JsonDocument);

/// The embedded language selected by the default or custom routing rules.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HtmlEmbeddedLanguage {
    Css,
    CssStyles,
    JavaScript,
    TypeScript,
    Jsx,
    Tsx,
    Json,
    Custom(&'static str),
}

/// A native tree for a routed region.
#[derive(Debug)]
pub enum HtmlEmbeddedDocument {
    Css(CssDocument),
    Javascript(JavascriptDocument),
    Custom(Box<dyn HtmlCustomDocument>),
}

/// A body or attribute value that selects an embedded language.
#[derive(Debug)]
pub struct HtmlMount {
    range: Range<usize>,
    language: HtmlEmbeddedLanguage,
    document: HtmlEmbeddedDocument,
}

impl HtmlMount {
    /// The region's byte range in the HTML source. Attribute values exclude quotes.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    pub fn language(&self) -> HtmlEmbeddedLanguage {
        self.language
    }

    pub fn document(&self) -> &HtmlEmbeddedDocument {
        &self.document
    }
}

pub(super) fn collect(doc: &HtmlDocument, nesting: HtmlNesting<'_>) -> Vec<HtmlMount> {
    let mut mounts = Vec::new();
    let source = doc.text().as_str();
    let mut overlay = None::<Range<usize>>;
    for node in doc.nodes() {
        let range = node.range();
        // A body overlay masks other routes in its content, but not in its
        // opening tag. Pre-order visits the element before those attributes.
        if overlay
            .as_ref()
            .is_some_and(|cover| cover.start < range.end && range.start < cover.end)
        {
            continue;
        }
        let mount = match node.kind() {
            Element if !nesting.nested_languages.is_empty() => {
                let Some(open) = node.children().next() else {
                    continue;
                };
                let Some(name) = tag_name(open, source).filter(|name| !is_raw_tag(name)) else {
                    continue;
                };
                let Some(parser) = custom_language(nesting, name, open, source) else {
                    continue;
                };
                let range = body_range(node, open, name, source);
                if range.is_empty() {
                    continue;
                }
                overlay = Some(range.clone());
                Route::Custom(parser).mount(source, range)
            }
            ScriptText | StyleText | TextareaText => {
                let Some(open) = node.parent().and_then(|parent| parent.child(OpenTag)) else {
                    continue;
                };
                let name = match node.kind() {
                    ScriptText => "script",
                    StyleText => "style",
                    _ => "textarea",
                };
                let route = custom_language(nesting, name, open, source)
                    .map(Route::Custom)
                    .or_else(|| {
                        let language = match node.kind() {
                            ScriptText => script_language(open, source),
                            StyleText => {
                                style_language(open, source).then_some(HtmlEmbeddedLanguage::Css)
                            }
                            _ => None,
                        };
                        language.map(Route::Stock)
                    });
                let Some(route) = route else { continue };
                route.mount(source, range)
            }
            Attribute => {
                let Some(name) = node.child(AttributeName).map(|name| &source[name.range()]) else {
                    continue;
                };
                let route = nesting
                    .nested_attributes
                    .iter()
                    .find(|rule| {
                        name == rule.name
                            && rule.tag_name.is_none_or(|name| {
                                name.is_empty()
                                    || node.parent().and_then(|tag| tag_name(tag, source))
                                        == Some(name)
                            })
                    })
                    .map(|rule| Route::Custom(rule.parser))
                    .or_else(|| match name {
                        "style" => Some(Route::Stock(HtmlEmbeddedLanguage::CssStyles)),
                        name if event_attribute(name) => {
                            Some(Route::Stock(HtmlEmbeddedLanguage::JavaScript))
                        }
                        _ => None,
                    });
                let Some(route) = route else { continue };
                let Some(range) = value_range(node, source).filter(|range| !range.is_empty())
                else {
                    continue;
                };
                route.mount(source, range)
            }
            _ => continue,
        };
        mounts.push(mount);
    }
    if overlay.is_some() {
        mounts.sort_unstable_by_key(|mount| (mount.range.start, mount.range.end));
    }
    mounts
}

fn tag_name<'a>(node: HtmlNode<'_>, source: &'a str) -> Option<&'a str> {
    node.child(TagName).map(|name| &source[name.range()])
}

fn is_raw_tag(name: &str) -> bool {
    matches!(name, "script" | "style" | "textarea")
}

fn body_range(node: HtmlNode<'_>, open: HtmlNode<'_>, name: &str, source: &str) -> Range<usize> {
    let context = lexer::context_name(name);
    let end = node
        .children()
        .next_back()
        .filter(|last| {
            // The HTML grammar displays raw and noMatch close tag terms as
            // CloseTag, but upstream only ends an overlay at the ordinary term.
            last.kind() == CloseTag
                && !is_raw_tag(&context)
                && tag_name(*last, source).is_some_and(|name| lexer::context_name(name) == context)
        })
        .map_or_else(|| node.range().end, |close| close.range().start);
    open.range().end..end
}

fn custom_language<'a>(
    nesting: HtmlNesting<'a>,
    name: &str,
    open: HtmlNode<'_>,
    source: &str,
) -> Option<HtmlNestedParser<'a>> {
    let mut attrs = None;
    nesting
        .nested_languages
        .iter()
        .find(|rule| {
            rule.tag == name
                && rule.attrs.is_none_or(|accept| {
                    accept(attrs.get_or_insert_with(|| attributes(open, source)))
                })
        })
        .map(|rule| rule.parser)
}

fn attributes<'a>(node: HtmlNode<'_>, source: &'a str) -> HtmlAttributes<'a> {
    node.children()
        .filter(|child| child.kind() == Attribute)
        .filter_map(|attr| {
            let name = attr.child(AttributeName)?;
            let value = value_range(attr, source).map_or("", |range| &source[range]);
            Some((&source[name.range()], value))
        })
        .collect()
}

enum Route<'a> {
    Stock(HtmlEmbeddedLanguage),
    Custom(HtmlNestedParser<'a>),
}

impl Route<'_> {
    fn mount(self, source: &str, range: Range<usize>) -> HtmlMount {
        use HtmlEmbeddedLanguage::*;
        let text = Text::new(&source[range.clone()]);
        let (language, document) = match self {
            Self::Custom(parser) => (
                Custom(parser.name),
                HtmlEmbeddedDocument::Custom((parser.parse)(text)),
            ),
            Self::Stock(language) => {
                let document = match language {
                    Css => HtmlEmbeddedDocument::Css(CssDocument::parse(text)),
                    CssStyles => HtmlEmbeddedDocument::Css(CssDocument::parse_styles(text)),
                    Json => HtmlEmbeddedDocument::Javascript(JavascriptDocument::parse_expression(
                        text,
                        JavascriptOptions::default(),
                    )),
                    JavaScript | TypeScript | Jsx | Tsx => {
                        HtmlEmbeddedDocument::Javascript(JavascriptDocument::parse_with_options(
                            text,
                            JavascriptOptions {
                                jsx: matches!(language, Jsx | Tsx),
                                typescript: matches!(language, TypeScript | Tsx),
                            },
                        ))
                    }
                    Custom(_) => unreachable!("custom routes have their own parser"),
                };
                (language, document)
            }
        };
        HtmlMount {
            range,
            language,
            document,
        }
    }
}

fn value_range(node: HtmlNode<'_>, source: &str) -> Option<Range<usize>> {
    let value = node.children().next_back()?;
    let range = value.range();
    match value.kind() {
        AttributeValue => {
            let has_close = range.len() > 1
                && source.as_bytes()[range.start] == source.as_bytes()[range.end - 1]
                && !value
                    .children()
                    .next_back()
                    .is_some_and(|last| last.kind() == Error);
            Some(range.start + 1..range.end - usize::from(has_close))
        }
        UnquotedAttributeValue => Some(range),
        _ => None,
    }
}

fn attribute_values<'a>(node: HtmlNode<'_>, source: &'a str) -> (&'a str, &'a str) {
    let (mut type_, mut lang) = ("", "");
    for attr in node.children().filter(|child| child.kind() == Attribute) {
        let Some(name) = attr.child(AttributeName).map(|name| &source[name.range()]) else {
            continue;
        };
        let result = match name {
            "type" => &mut type_,
            "lang" => &mut lang,
            _ => continue,
        };
        *result = value_range(attr, source).map_or("", |range| &source[range]);
    }
    (type_, lang)
}

fn script_language(node: HtmlNode<'_>, source: &str) -> Option<HtmlEmbeddedLanguage> {
    use HtmlEmbeddedLanguage::*;
    let (type_, lang) = attribute_values(node, source);
    if type_ == "text/typescript" || lang == "ts" {
        return Some(TypeScript);
    }
    match type_ {
        "text/babel" | "text/jsx" => return Some(Jsx),
        "text/typescript-jsx" => return Some(Tsx),
        _ => {}
    }
    let lower = type_.to_ascii_lowercase();
    let application = lower.strip_prefix("application/");
    if matches!(lower.as_str(), "importmap" | "speculationrules")
        || application.is_some_and(|value| {
            value == "json"
                || value.strip_suffix("+json").is_some_and(|prefix| {
                    !prefix.is_empty() && !prefix.contains(['\r', '\n', '\u{2028}', '\u{2029}'])
                })
        })
    {
        return Some(Json);
    }
    if matches!(lower.as_str(), "" | "module") {
        return Some(JavaScript);
    }
    let subtype = lower.strip_prefix("text/").or(application)?;
    matches!(
        subtype.strip_prefix("x-").unwrap_or(subtype),
        "javascript" | "ecmascript"
    )
    .then_some(JavaScript)
}

fn style_language(node: HtmlNode<'_>, source: &str) -> bool {
    let (type_, lang) = attribute_values(node, source);
    if !lang.is_empty() && lang != "css" {
        return false;
    }
    let lower = type_.to_ascii_lowercase();
    let subtype = lower.strip_prefix("text/").unwrap_or(&lower);
    lower.is_empty()
        || matches!(
            subtype.strip_prefix("x-").unwrap_or(subtype),
            "stylesheet" | "css"
        )
}

fn event_attribute(name: &str) -> bool {
    matches!(
        name,
        "onbeforeunload"
            | "oncopy"
            | "oncut"
            | "ondragstart"
            | "ondragover"
            | "ondragleave"
            | "ondragenter"
            | "ondragend"
            | "ondrag"
            | "onpaste"
            | "onfocus"
            | "onblur"
            | "onchange"
            | "onclick"
            | "onload"
            | "onmousedown"
            | "onmouseenter"
            | "onmouseleave"
            | "onmouseup"
            | "onkeydown"
            | "onkeyup"
            | "onresize"
            | "onscroll"
            | "onunload"
    )
}

pub(super) fn highlights<'a>(
    doc: &'a HtmlDocument,
    range: &Range<usize>,
    spans: &mut Vec<HighlightSpan<'a>>,
) {
    let start = doc
        .mounts
        .partition_point(|mount| mount.range.end <= range.start);
    for mount in &doc.mounts[start..] {
        if mount.range.start >= range.end {
            break;
        }
        let offset = mount.range.start;
        let query = range.start.saturating_sub(offset)..range.end.min(mount.range.end) - offset;
        let child = mount.document();
        let shift = |span: HighlightSpan<'a>| HighlightSpan {
            range: offset + span.range.start..offset + span.range.end,
            tags: span.tags,
        };
        match child {
            HtmlEmbeddedDocument::Css(css) => {
                spans.extend(css.highlights(query).into_iter().flatten().map(shift))
            }
            HtmlEmbeddedDocument::Javascript(js) => {
                spans.extend(js.highlights(query).into_iter().flatten().map(shift))
            }
            HtmlEmbeddedDocument::Custom(custom) => {
                let source = &doc.text().as_str()[mount.range.clone()];
                spans.extend(
                    custom
                        .highlights(query.clone())
                        .into_iter()
                        .filter_map(|span| {
                            let range =
                                span.range.start.max(query.start)..span.range.end.min(query.end);
                            (!range.is_empty() && source.get(range.clone()).is_some()).then(|| {
                                shift(HighlightSpan {
                                    range,
                                    tags: span.tags,
                                })
                            })
                        }),
                );
            }
        }
    }
}

pub(super) fn foldable(
    doc: &HtmlDocument,
    line_start: usize,
    line_end: usize,
) -> Option<Range<usize>> {
    let mount = mount_at(doc, line_end)?;
    let offset = mount.range.start;
    let from = line_start.saturating_sub(offset);
    let to = line_end - offset;
    let local = match mount.document() {
        HtmlEmbeddedDocument::Css(css) => css.foldable(from, to),
        HtmlEmbeddedDocument::Javascript(js) => js.foldable(from, to),
        HtmlEmbeddedDocument::Custom(custom) => {
            let source = &doc.text().as_str()[mount.range.clone()];
            custom.foldable(from, to).filter(|range| {
                (from..=to).contains(&range.start)
                    && to < range.end
                    && source.get(range.clone()).is_some()
            })
        }
    }?;
    Some(offset + local.start..offset + local.end)
}

pub(super) fn indentation(
    doc: &HtmlDocument,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'_>,
) -> Option<usize> {
    let mount = mount_at(doc, position)?;
    let offset = mount.range.start;
    let cx = IndentContext::new(doc.text(), position, settings, options)?;
    let over = |position| {
        // The first embedded line can start after an HTML tag. Its base
        // indentation still comes from the original line.
        if position == 0 {
            cx.line_indent(offset, Assoc::After)
        } else {
            options
                .override_indentation
                .and_then(|get| get(position + offset))
        }
    };
    let local = IndentOptions {
        override_indentation: Some(&over),
        simulate_break: options
            .simulate_break
            .filter(|pos| mount.range.contains(pos))
            .map(|pos| pos - offset),
        simulate_double_break: options.simulate_double_break,
    };
    match mount.document() {
        HtmlEmbeddedDocument::Css(css) => css.indentation(position - offset, settings, local),
        HtmlEmbeddedDocument::Javascript(js) => js.indentation(position - offset, settings, local),
        HtmlEmbeddedDocument::Custom(custom) => {
            custom.indentation(position - offset, settings, local)
        }
    }
}

fn mount_at(doc: &HtmlDocument, position: usize) -> Option<&HtmlMount> {
    let start = doc
        .mounts
        .partition_point(|mount| mount.range.end <= position);
    doc.mounts
        .get(start)
        .filter(|mount| mount.range.start < position)
}
