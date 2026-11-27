//! Completion from `@codemirror/lang-jinja` 6.0.1.
//! See `docs/licenses/codemirror-lang-jinja.txt`.

use std::sync::{Arc, LazyLock};

use crate::{
    Text,
    autocomplete::session::{CompletionItem, SourceResult},
};

use super::{JinjaDocument, JinjaNode};

/// Resolve an object path, such as `["user", "address"]`, at the query position.
pub type JinjaPropertyCompletion<'a> =
    dyn Fn(&[&str], &JinjaDocument, usize, bool) -> Vec<CompletionItem> + Send + Sync + 'a;

/// Add tag and expression options before the built-in options.
#[derive(Clone, Copy, Default)]
pub struct JinjaCompletionConfig<'a> {
    pub tags: &'a [CompletionItem],
    pub variables: &'a [CompletionItem],
    pub properties: Option<&'a JinjaPropertyCompletion<'a>>,
}

struct Catalog {
    tags: Arc<[CompletionItem]>,
    expressions: Arc<[CompletionItem]>,
}

static BUILTINS: LazyLock<Catalog> = LazyLock::new(|| {
    Catalog {
    tags: items(
        "raw endraw filter endfilter trans pluralize endtrans with endwith autoescape endautoescape \
         if elif else endif for endfor call endcall block endblock set endset macro endmacro import \
         include break continue debug do extends",
        "keyword",
    )
    .collect(),
    expressions: items(
        "boolean callable defined divisibleby eq escaped even filter float ge gt in integer \
         iterable le lower lt mapping ne none number odd sameas sequence string test undefined \
         upper range lipsum dict joiner namespace",
        "function",
    )
    .chain(items(
        "loop super self true false varargs kwargs caller name arguments catch_kwargs catch_varargs caller",
        "keyword",
    ))
    .collect(),
}
});

static FILTERS: LazyLock<Arc<[CompletionItem]>> = LazyLock::new(|| {
    items(
        "abs attr batch capitalize center default dictsort escape filesizeformat first float \
         forceescape format groupby indent int items join last length list lower map max min \
         pprint random reject rejectattr replace reverse round safe select selectattr slice \
         sort string striptags sum title tojson trim truncate unique upper urlencode urlize wordcount \
         wordwrap xmlattr",
        "function",
    )
    .collect()
});

fn items(words: &str, kind: &'static str) -> impl Iterator<Item = CompletionItem> {
    words.split_ascii_whitespace().map(move |label| {
        let mut item = CompletionItem::new(label);
        item.kind = Some(kind.to_owned());
        item
    })
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
pub fn jinja_completion_source(
    doc: &JinjaDocument,
    position: usize,
    explicit: bool,
) -> Option<SourceResult> {
    complete(doc, position, explicit, &BUILTINS, None)
}

/// Build a template source with additional options. Preserve duplicates and order.
pub fn jinja_completion_source_with(
    config: JinjaCompletionConfig<'_>,
) -> impl Fn(&JinjaDocument, usize, bool) -> Option<SourceResult> + '_ {
    let catalog = Catalog {
        tags: prepend(config.tags, &BUILTINS.tags),
        expressions: prepend(config.variables, &BUILTINS.expressions),
    };
    move |doc, position, explicit| complete(doc, position, explicit, &catalog, config.properties)
}

fn complete(
    doc: &JinjaDocument,
    position: usize,
    explicit: bool,
    catalog: &Catalog,
    properties: Option<&JinjaPropertyCompletion<'_>>,
) -> Option<SourceResult> {
    let text = doc.text();
    let prefix = text.as_str().get(..position)?;
    let node = resolve_left(doc.root(), position);
    let before = child_before(node, position).unwrap_or(node).name();
    let (from, options) = if node.name() == "FilterName" {
        (node.range().start, Arc::clone(&FILTERS))
    } else if explicit && matches!(before, "FilterOp" | "filter") {
        (position, Arc::clone(&FILTERS))
    } else if node.name() == "TagName" {
        (node.range().start, Arc::clone(&catalog.tags))
    } else if explicit && (before == "{%" || at_unparsed_tag_start(node, prefix)) {
        (position, Arc::clone(&catalog.tags))
    } else if let Some((target, from)) = member(node, before, position) {
        let properties = properties?;
        let path = property_path(text, target)?;
        (from, properties(&path, doc, position, explicit).into())
    } else {
        let from = match node.name() {
            "VariableName" => node.range().start,
            "Comment" | "StringLiteral" | "NumberLiteral" => return None,
            _ => word_before(text, position).or(explicit.then_some(position))?,
        };
        (from, Arc::clone(&catalog.expressions))
    };
    (!options.is_empty()).then(|| {
        SourceResult::new(from, options).with_valid_for(|text, _, _| text.chars().all(word))
    })
}

fn at_unparsed_tag_start(node: JinjaNode<'_>, prefix: &str) -> bool {
    // An unparsed tail can start with an empty tag (including a trim marker).
    node.name() == "⚠"
        && node
            .parent()
            .is_some_and(|parent| parent.name() == "Template")
        && prefix[node.range().start..]
            .strip_prefix("{%")
            .is_some_and(|rest| {
                let rest = super::trim_space(rest);
                super::trim_space(rest.strip_prefix('-').unwrap_or(rest)).is_empty()
            })
}

fn child_before(node: JinjaNode<'_>, position: usize) -> Option<JinjaNode<'_>> {
    let children = &node.data().children;
    let index = children.partition_point(|&id| node.nodes[id].range.start < position);
    Some(JinjaNode {
        id: *children.get(index.checked_sub(1)?)?,
        ..node
    })
}

fn resolve_left(mut node: JinjaNode<'_>, position: usize) -> JinjaNode<'_> {
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
        if last.name() == "⚠" && last.range().is_empty() {
            node = child;
            scan = children.next_back();
        } else {
            scan = Some(last);
        }
    }
    node
}

fn member<'a>(
    node: JinjaNode<'a>,
    before: &str,
    position: usize,
) -> Option<(JinjaNode<'a>, usize)> {
    let target = match node.name() {
        "PropertyName" | "." => node.parent()?,
        "MemberExpression" if before == "." => node,
        _ => return None,
    };
    let from = if node.name() == "PropertyName" {
        node.range().start
    } else {
        position
    };
    (target.name() == "MemberExpression").then_some((target, from))
}

fn property_path<'a>(text: &'a Text, mut target: JinjaNode<'_>) -> Option<Vec<&'a str>> {
    let mut path = Vec::new();
    loop {
        // The receiver is the first child of a MemberExpression.
        let object = target.children().next()?;
        match object.name() {
            "VariableName" => {
                path.push(text.slice(object.range())?);
                path.reverse();
                return Some(path);
            }
            "MemberExpression" => {
                if let Some(name) = object
                    .children()
                    .find(|child| child.name() == "PropertyName")
                {
                    path.push(text.slice(name.range())?);
                }
                target = object;
            }
            _ => return None,
        }
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
