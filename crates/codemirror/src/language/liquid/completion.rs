//! Completion from `@codemirror/lang-liquid` 6.3.4.
//! See `docs/licenses/codemirror-lang-liquid.txt`.

use std::sync::{Arc, LazyLock};

use crate::{
    Text,
    autocomplete::session::{CompletionItem, SourceResult},
};

use super::{LiquidDocument, LiquidNode, LiquidNodeKind::*};

/// Resolve an object path, such as `["user", "address"]`, at the query position.
/// String subscripts use their unquoted text. Other subscripts use `"[]"`.
pub type LiquidPropertyCompletion<'a> =
    dyn Fn(&[&str], &LiquidDocument, usize, bool) -> Vec<CompletionItem> + Send + Sync + 'a;

/// Add tag, filter, and expression options before the built-in options.
#[derive(Clone, Copy, Default)]
pub struct LiquidCompletionConfig<'a> {
    pub tags: &'a [CompletionItem],
    pub filters: &'a [CompletionItem],
    pub variables: &'a [CompletionItem],
    pub properties: Option<&'a LiquidPropertyCompletion<'a>>,
}

struct Catalog {
    tags: Arc<[CompletionItem]>,
    filters: Arc<[CompletionItem]>,
    expressions: Arc<[CompletionItem]>,
}

static BUILTINS: LazyLock<Catalog> = LazyLock::new(|| Catalog {
    tags: items(
        "cycle comment endcomment raw endraw echo increment decrement liquid if elsif \
         else endif unless endunless case endcase for endfor tablerow endtablerow break continue \
         assign capture endcapture render include",
        "keyword",
    ),
    filters: items(
        "abs append at_least at_most capitalize ceil compact concat date default \
         divided_by downcase escape escape_once first floor join last lstrip map minus modulo \
         newline_to_br plus prepend remove remove_first replace replace_first reverse round rstrip \
         size slice sort sort_natural split strip strip_html strip_newlines sum times truncate \
         truncatewords uniq upcase url_decode url_encode where",
        "function",
    ),
    expressions: items("empty forloop tablerowloop in with as", "keyword"),
});

static FORLOOP: LazyLock<Arc<[CompletionItem]>> =
    LazyLock::new(|| items("first index index0 last length rindex", "property"));
static TABLEROWLOOP: LazyLock<Arc<[CompletionItem]>> = LazyLock::new(|| {
    items(
        "col col0 col_first col_last first index index0 last length rindex rindex0 row",
        "property",
    )
});

fn items(words: &str, kind: &str) -> Arc<[CompletionItem]> {
    words
        .split_ascii_whitespace()
        .map(|label| {
            let mut item = CompletionItem::new(label);
            item.kind = Some(kind.to_owned());
            item
        })
        .collect()
}

fn prepend(extra: &[CompletionItem], base: &Arc<[CompletionItem]>) -> Arc<[CompletionItem]> {
    if extra.is_empty() {
        Arc::clone(base)
    } else {
        extra.iter().chain(base.iter()).cloned().collect()
    }
}

/// Return the template source's options at a UTF-8 byte position.
/// The caller selects this source or the mounted HTML source.
pub fn liquid_completion_source(
    doc: &LiquidDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    complete(doc, position, explicit, &BUILTINS, None)
}

/// Build a template source with additional options. Preserve duplicates and order.
pub fn liquid_completion_source_with(
    config: LiquidCompletionConfig<'_>,
) -> impl Fn(&LiquidDocument, usize, bool) -> Option<SourceResult> + '_ {
    let catalog = Catalog {
        tags: prepend(config.tags, &BUILTINS.tags),
        filters: prepend(config.filters, &BUILTINS.filters),
        expressions: prepend(config.variables, &BUILTINS.expressions),
    };
    move |doc, position, explicit| complete(doc, position, explicit, &catalog, config.properties)
}

fn complete(
    doc: &LiquidDocument,
    position: usize,
    explicit: bool,
    catalog: &Catalog,
    properties: Option<&LiquidPropertyCompletion<'_>>,
) -> Option<SourceResult> {
    let text = doc.text();
    text.as_str().get(..position)?;
    let node = resolve_left(doc.root(), position);
    let before = child_before(node, position).unwrap_or(node).kind();
    let (from, options) = if node.kind() == FilterName {
        (node.range().start, Arc::clone(&catalog.filters))
    } else if explicit && before == Pipe {
        (position, Arc::clone(&catalog.filters))
    } else if node.kind() == TagName {
        (node.range().start, Arc::clone(&catalog.tags))
    } else if explicit && before == TagStart {
        (position, Arc::clone(&catalog.tags))
    } else if let Some(from) = recovered_end_tag_name(text, node, position) {
        (from, Arc::clone(&catalog.tags))
    } else if let Some((target, from)) = member(node, before, position) {
        (
            from,
            property_options(doc, position, explicit, target, properties)?,
        )
    } else {
        let from = if node.kind() == VariableName {
            node.range().start
        } else {
            word_before(text, position).or_else(|| {
                (explicit
                    && !matches!(
                        node.kind(),
                        CommentText | StringLiteral | NumberLiteral | InlineComment
                    ))
                .then_some(position)
            })?
        };
        (from, Arc::clone(&catalog.expressions))
    };
    (!options.is_empty()).then(|| {
        SourceResult::new(from, options).with_valid_for(|text, _, _| text.chars().all(word))
    })
}

pub(crate) fn has_recovered_end_tag_name(doc: &LiquidDocument, position: usize) -> bool {
    recovered_end_tag_name(doc.text(), resolve_left(doc.root(), position), position).is_some()
}

fn recovered_end_tag_name(text: &Text, node: LiquidNode<'_>, position: usize) -> Option<usize> {
    if node.kind() != Text {
        return None;
    }
    let start = node.range().start;
    let previous = child_before(node.parent()?, start)?;
    // Native recovery can leave a custom `end...` tag name in a Text node.
    // Reserved end tags in this position are errors upstream too.
    if previous.kind() != EndTag
        || previous.range().end != start
        || !previous
            .children()
            .next_back()
            .is_some_and(|child| child.kind() == Error && child.range() == (start..start))
    {
        return None;
    }
    let tail = text.slice(node.range())?;
    let content = super::trim_space(tail);
    let from = start + tail.len() - content.len();
    let end = content
        .find(|ch| !word(ch) && ch != '-')
        .unwrap_or(content.len());
    let name = &content[..end];
    (from < position
        && position <= from + end
        && name.starts_with("end")
        && !matches!(
            name,
            "endif"
                | "endunless"
                | "endcase"
                | "endfor"
                | "endtablerow"
                | "endcapture"
                | "endraw"
                | "endcomment"
        ))
    .then_some(from)
}

fn child_before(node: LiquidNode<'_>, position: usize) -> Option<LiquidNode<'_>> {
    let children = &node.data().children;
    let index = children.partition_point(|&id| node.nodes[id].range.start < position);
    Some(LiquidNode {
        id: *children.get(index.checked_sub(1)?)?,
        ..node
    })
}

fn resolve_left(mut node: LiquidNode<'_>, position: usize) -> LiquidNode<'_> {
    while let Some(child) =
        child_before(node, position).filter(|child| position <= child.range().end)
    {
        node = child;
    }
    // Lezer enters nodes that end with an inserted zero-width error.
    let mut scan = child_before(node, position);
    while let Some(child) = scan {
        let mut children = child.children();
        let Some(last) = children
            .next_back()
            .filter(|last| last.range().end == child.range().end)
        else {
            break;
        };
        if last.kind() == Error && last.range().is_empty() {
            node = child;
            scan = children.next_back();
        } else {
            scan = Some(last);
        }
    }
    node
}

fn member<'a>(
    node: LiquidNode<'a>,
    before: super::LiquidNodeKind,
    position: usize,
) -> Option<(LiquidNode<'a>, usize)> {
    let target = match node.kind() {
        PropertyName | Dot => node.parent()?,
        MemberExpression if before == Dot => node,
        _ => return None,
    };
    let from = if node.kind() == PropertyName {
        node.range().start
    } else {
        position
    };
    (target.kind() == MemberExpression).then_some((target, from))
}

fn property_options(
    doc: &LiquidDocument,
    position: usize,
    explicit: bool,
    mut target: LiquidNode<'_>,
    properties: Option<&LiquidPropertyCompletion<'_>>,
) -> Option<Arc<[CompletionItem]>> {
    let text = doc.text();
    let mut path = Vec::new();
    loop {
        let object = target.children().next()?;
        match object.kind() {
            VariableName | ForLoop | TableRowLoop => {
                let name = text.slice(object.range())?;
                match name {
                    "forloop" => return path.is_empty().then(|| Arc::clone(&FORLOOP)),
                    "tablerowloop" => return path.is_empty().then(|| Arc::clone(&TABLEROWLOOP)),
                    _ => path.push(name),
                }
                path.reverse();
                return Some(properties?(&path, doc, position, explicit).into());
            }
            MemberExpression => {
                if let Some(name) = object.children().find(|child| child.kind() == PropertyName) {
                    path.push(text.slice(name.range())?);
                }
            }
            SubscriptExpression => {
                // Brackets are anonymous in the pinned grammar.
                let key = object
                    .children()
                    .nth(1)
                    .filter(|child| child.kind() == StringLiteral);
                path.push(if let Some(key) = key {
                    let range = key.range();
                    text.slice(range.start + 1..range.end - 1)?
                } else {
                    "[]"
                });
            }
            _ => return None,
        }
        target = object;
    }
}

fn word(ch: char) -> bool {
    // JavaScript includes both halves of a surrogate pair in \u00c0-\uffff.
    ch.is_ascii_alphanumeric() || ch == '_' || ch >= '\u{c0}'
}

fn word_before(text: &Text, position: usize) -> Option<usize> {
    let line = text.line_at(position)?;
    let end = position.min(line.range.end);
    let mut units = 0;
    line.text[..end - line.range.start]
        .char_indices()
        .rev()
        .take_while(|&(_, ch)| {
            units += ch.len_utf16();
            units <= 250 && word(ch)
        })
        .last()
        .map(|(start, _)| line.range.start + start)
}
