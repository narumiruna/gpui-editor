use std::{
    cmp::Reverse,
    collections::HashMap,
    fmt,
    ops::Range,
    sync::{Arc, LazyLock, OnceLock},
};

use super::{GROUP, NodeProp, NodePropSource, NodeType, NodeTypes, SyntaxNode};
use crate::language::{HighlightSpan, HighlightTag};

/// The selected rule on an LR node, before adding any inherited tags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StyleTags<'a> {
    pub tags: &'a [HighlightTag],
    /// Do not visit this node's children when highlighting.
    pub opaque: bool,
    /// Add these tags to all descendants, before their own tags.
    pub inherit: bool,
}

impl<'a> StyleTags<'a> {
    fn from_rule(rule: &'a Rule) -> Self {
        Self {
            tags: &rule.tags,
            opaque: rule.mode == Mode::Opaque,
            inherit: rule.mode == Mode::Inherit,
        }
    }
}

/// An invalid path in an LR style selector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StyleSelectorError(String);

impl StyleSelectorError {
    pub(crate) fn path(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for StyleSelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid LR style path: {}", self.0)
    }
}

impl std::error::Error for StyleSelectorError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Mode {
    Normal,
    Inherit,
    Opaque,
}

#[derive(Clone, Debug)]
struct Rule {
    tags: Arc<[HighlightTag]>,
    parents: Arc<[String]>,
    mode: Mode,
}

#[derive(Clone, Debug)]
pub(crate) struct StyleRules(Vec<Rule>);

impl StyleRules {
    fn combine(old: &Self, new: &Self) -> Self {
        let (mut old, mut new) = (old.0.iter().peekable(), new.0.iter().peekable());
        let mut combined: Vec<Rule> = Vec::new();
        while let Some(rule) = match (old.peek(), new.peek()) {
            (Some(a), Some(b)) if a.parents.len() > b.parents.len() => old.next(),
            (_, Some(_)) => new.next(),
            (_, None) => old.next(),
        } {
            if !combined.last().is_some_and(|prev| {
                prev.mode == rule.mode && prev.parents.is_empty() && rule.parents.is_empty()
            }) {
                combined.push(rule.clone());
            }
        }
        Self(combined)
    }
}

impl StyleRules {
    pub(crate) fn normal(tags: &[HighlightTag]) -> Self {
        Self(vec![Rule {
            tags: Arc::from(tags),
            parents: Arc::from([]),
            mode: Mode::Normal,
        }])
    }
}

pub(crate) static STYLE_RULES: LazyLock<NodeProp<StyleRules>> =
    LazyLock::new(|| NodeProp::with_combine(StyleRules::combine));

impl NodeTypes {
    pub(crate) fn has_different_styles(&self, stock: &Self) -> bool {
        !self.same_set(stock)
            && self.iter().zip(stock.iter()).any(|(row, stock)| {
                row.prop(&STYLE_RULES).map(std::ptr::from_ref)
                    != stock.prop(&STYLE_RULES).map(std::ptr::from_ref)
            })
    }
}

/// Build an LR node-type property source from Lezer style selectors.
///
/// Each entry can contain several space-separated paths. A path can include
/// direct parents separated by `/`. `*` matches one parent, not the final node.
/// Quote node names with JSON string syntax to use selector punctuation as part
/// of a name. `/...` adds the tags to descendants; `!` hides descendants during
/// highlighting. An empty tag list still selects a rule.
///
/// Longer parent paths win. For equal depths, earlier paths in one source win;
/// across sources, later sources win. The final name selects a node-type row
/// by name first, then by group; parent tests use only node names.
pub fn style_tags(
    spec: impl IntoIterator<Item = (impl AsRef<str>, Vec<HighlightTag>)>,
) -> Result<NodePropSource, StyleSelectorError> {
    let mut by_name = HashMap::<String, StyleRules>::new();
    for (selectors, tags) in spec {
        let tags = Arc::from(tags);
        for path in selectors
            .as_ref()
            .split(' ')
            .filter(|part| !part.is_empty())
        {
            let (name, parents, mode) =
                parse_path(path).ok_or_else(|| StyleSelectorError(path.into()))?;
            by_name
                .entry(name)
                .or_insert_with(|| StyleRules(Vec::new()))
                .0
                .push(Rule {
                    tags: Arc::clone(&tags),
                    parents: parents.into(),
                    mode,
                });
        }
    }
    for rules in by_name.values_mut() {
        rules.0.sort_by_key(|rule| Reverse(rule.parents.len()));
    }
    Ok(STYLE_RULES.add(move |row| {
        by_name
            .get(row.name())
            .or_else(|| {
                row.prop(&GROUP)?
                    .iter()
                    .find_map(|group| by_name.get(group))
            })
            .cloned()
    }))
}

fn parse_path(mut rest: &str) -> Option<(String, Vec<String>, Mode)> {
    let mut names = Vec::new();
    let mode = loop {
        if rest == "..." && !names.is_empty() {
            break Mode::Inherit;
        }
        let (name, next) = parse_name(rest)?;
        names.push(name);
        match next {
            "" => break Mode::Normal,
            "!" => break Mode::Opaque,
            _ => rest = next.strip_prefix('/')?,
        }
    };
    let name = names.pop()?;
    (!name.is_empty()).then_some((name, names, mode))
}

fn parse_name(rest: &str) -> Option<(String, &str)> {
    if let Some(quoted) = rest.strip_prefix('"') {
        let mut escaped = false;
        for (i, byte) in quoted.bytes().enumerate() {
            if !escaped && byte == b'"' {
                let (json, tail) = rest.split_at(i + 2);
                return Some((serde_json::from_str(json).ok()?, tail));
            }
            escaped = !escaped && byte == b'\\';
        }
        return None;
    }
    let (name, tail) = rest.split_at(rest.find(['/', '!']).unwrap_or(rest.len()));
    (!name.is_empty()).then(|| ((if name == "*" { "" } else { name }).into(), tail))
}

impl<'a> SyntaxNode<'a> {
    /// Select a raw rule with ancestor names in root-to-parent order.
    /// Do not include this node's name. Return `None` if no rule matches.
    pub fn style_tags(self, ancestors: &[&str]) -> Option<StyleTags<'a>> {
        select_tags(self.node_type()?, ancestors.iter().rev().copied())
    }
}

pub(crate) fn select_tags<'a, 'name>(
    row: &'a NodeType,
    ancestors: impl Iterator<Item = &'name str> + Clone,
) -> Option<StyleTags<'a>> {
    select_rule(row.prop(&STYLE_RULES), ancestors).map(StyleTags::from_rule)
}

fn select_rule<'a, 'name>(
    rules: Option<&'a StyleRules>,
    ancestors: impl Iterator<Item = &'name str> + Clone,
) -> Option<&'a Rule> {
    rules?.0.iter().find(|rule| {
        let mut ancestors = ancestors.clone();
        rule.parents.iter().rev().all(|parent| {
            ancestors
                .next()
                .is_some_and(|name| parent.is_empty() || parent == name)
        })
    })
}

#[derive(Debug, Default)]
pub(crate) struct ConfiguredHighlights {
    styles: Vec<Style>,
    runs: Vec<Run>,
}

#[derive(Debug)]
struct Style {
    local: Arc<[HighlightTag]>,
    inherited: Option<usize>,
    combined: OnceLock<Vec<HighlightTag>>,
}

#[derive(Debug)]
struct Run {
    range: Range<usize>,
    style: usize,
}

pub(crate) trait StyledNode<'a>: Copy {
    fn child(self, index: usize) -> Option<Self>;
    fn range(self) -> Range<usize>;
    fn name(self) -> &'a str;
    fn node_type(self) -> Option<&'a NodeType>;

    fn rules(self) -> Option<&'a StyleRules> {
        self.node_type()?.prop(&STYLE_RULES)
    }
}

impl<'a> StyledNode<'a> for SyntaxNode<'a> {
    fn child(self, index: usize) -> Option<Self> {
        self.children().nth(index)
    }

    fn range(self) -> Range<usize> {
        self.range()
    }

    fn name(self) -> &'a str {
        self.name()
    }

    fn node_type(self) -> Option<&'a NodeType> {
        self.node_type()
    }
}

struct Frame<Node> {
    node: Node,
    range: Range<usize>,
    next_child: Option<usize>,
    style: Option<usize>,
    inherited: Option<usize>,
}

impl<'a, Node: StyledNode<'a>> Frame<Node> {
    fn next(&mut self) -> Option<Node> {
        let index = self.next_child.as_mut()?;
        let node = self.node.child(*index)?;
        *index += 1;
        Some(node)
    }
}

impl ConfiguredHighlights {
    pub(crate) fn build<'a>(root: impl StyledNode<'a>) -> Self {
        let mut result = Self::default();
        let root_rule = select_rule(root.rules(), std::iter::empty());
        let mut stack = vec![result.frame(root, root_rule, None)];
        while let Some(frame) = stack.last_mut() {
            if let Some(node) = frame.next() {
                let range = node.range();
                if !range.is_empty() {
                    result.push(frame.range.start..range.start, frame.style);
                    frame.range.start = range.end;
                    let inherited = frame.inherited;
                    let parents = stack.iter().rev().map(|frame| frame.node.name());
                    let rule = select_rule(node.rules(), parents);
                    stack.push(result.frame(node, rule, inherited));
                }
            } else {
                let frame = stack.pop().unwrap();
                result.push(frame.range, frame.style);
            }
        }
        result
    }

    fn frame<'a, Node: StyledNode<'a>>(
        &mut self,
        node: Node,
        rule: Option<&Rule>,
        inherited: Option<usize>,
    ) -> Frame<Node> {
        let style = if let Some(rule) = rule.filter(|rule| !rule.tags.is_empty()) {
            let id = self.styles.len();
            self.styles.push(Style {
                local: Arc::clone(&rule.tags),
                inherited,
                combined: OnceLock::new(),
            });
            Some(id)
        } else {
            inherited
        };
        Frame {
            node,
            range: node.range(),
            style,
            inherited: if rule.is_some_and(|rule| rule.mode == Mode::Inherit) {
                style
            } else {
                inherited
            },
            next_child: (!rule.is_some_and(|rule| rule.mode == Mode::Opaque)).then_some(0),
        }
    }

    fn push(&mut self, range: Range<usize>, style: Option<usize>) {
        let Some(style) = style.filter(|_| !range.is_empty()) else {
            return;
        };
        if let Some(last) = self.runs.last_mut()
            && last.style == style
            && last.range.end == range.start
        {
            last.range.end = range.end;
        } else {
            self.runs.push(Run { range, style });
        }
    }

    fn tags(&self, id: usize) -> &[HighlightTag] {
        let style = &self.styles[id];
        if style.inherited.is_none() {
            return &style.local;
        }
        // Flatten only a style that is requested. Building a snapshot of a deep
        // tree takes linear storage even when tags accumulate at each level.
        style.combined.get_or_init(|| {
            let parts: Vec<_> = std::iter::successors(Some(id), |&i| self.styles[i].inherited)
                .map(|i| self.styles[i].local.as_ref())
                .collect();
            parts.into_iter().rev().flatten().cloned().collect()
        })
    }

    pub(crate) fn clipped(&self, range: Range<usize>) -> impl Iterator<Item = HighlightSpan<'_>> {
        let first = self
            .runs
            .partition_point(|run| run.range.end <= range.start);
        self.runs[first..]
            .iter()
            .take_while(move |run| run.range.start < range.end)
            .filter_map(move |run| {
                let start = run.range.start.max(range.start);
                let end = run.range.end.min(range.end);
                (start < end).then(|| HighlightSpan {
                    range: start..end,
                    tags: self.tags(run.style),
                })
            })
    }
}
