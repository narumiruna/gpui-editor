use std::{
    cmp::Reverse,
    collections::HashMap,
    fmt,
    ops::Range,
    sync::{Arc, OnceLock},
};

use super::{WastNode, WastNodeKind, WastPropSource, node_props::STYLE_RULES};
use crate::language::{HighlightSpan, HighlightTag, lezer::SyntaxNode};

/// The selected rule on a WAST node, before adding any inherited tags.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WastStyleTags<'a> {
    pub tags: &'a [HighlightTag],
    /// Do not visit this node's children when highlighting.
    pub opaque: bool,
    /// Add these tags to all descendants, before their own tags.
    pub inherit: bool,
}

impl<'a> WastStyleTags<'a> {
    fn from_rule(rule: &'a Rule) -> Self {
        Self {
            tags: &rule.tags,
            opaque: rule.mode == Mode::Opaque,
            inherit: rule.mode == Mode::Inherit,
        }
    }
}

/// An invalid path in a WAST style selector.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WastStyleSelectorError(String);

impl fmt::Display for WastStyleSelectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid WAST style path: {}", self.0)
    }
}

impl std::error::Error for WastStyleSelectorError {}

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
pub(super) struct StyleRules(Vec<Rule>);

impl StyleRules {
    pub(super) fn combine(old: &Self, new: &Self) -> Self {
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

pub(super) fn stock_rules(name: &str) -> Option<StyleRules> {
    use WastNodeKind::*;
    let kind = [
        OpenParen,
        CloseParen,
        Identifier,
        Type,
        Keyword,
        Number,
        String,
        LineComment,
        BlockComment,
    ]
    .into_iter()
    .find(|kind| kind.name() == name)?;
    Some(StyleRules(vec![Rule {
        tags: Arc::from(kind.highlight()?),
        parents: Arc::from([]),
        mode: Mode::Normal,
    }]))
}

/// Build a WAST node-type property source from Lezer style selectors.
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
) -> Result<WastPropSource, WastStyleSelectorError> {
    let mut by_name = HashMap::<String, StyleRules>::new();
    for (selectors, tags) in spec {
        let tags = Arc::from(tags);
        for path in selectors
            .as_ref()
            .split(' ')
            .filter(|part| !part.is_empty())
        {
            let (name, parents, mode) =
                parse_path(path).ok_or_else(|| WastStyleSelectorError(path.into()))?;
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
                row.prop(&super::GROUP)?
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

impl<'a> WastNode<'a> {
    /// Select the first style rule whose direct parent path matches this node.
    /// `None` means no rule matched; an explicit empty rule returns `Some`.
    pub fn style_tags(self) -> Option<WastStyleTags<'a>> {
        let ancestors = std::iter::successors(self.parent(), |node| node.parent())
            .map(|node| node.kind().name());
        select_rule(self.prop(&STYLE_RULES), ancestors).map(WastStyleTags::from_rule)
    }
}

impl<'a> SyntaxNode<'a> {
    /// Select the raw WAST style rule for this node and its ancestor names,
    /// in root-to-parent order. Do not include this node's own name.
    /// Return `None` if the tree has no WAST styles or no rule matches.
    pub fn wast_style_tags(self, ancestors: &[&str]) -> Option<WastStyleTags<'a>> {
        select_rule(self.prop(&STYLE_RULES), ancestors.iter().rev().copied())
            .map(WastStyleTags::from_rule)
    }
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

#[derive(Clone, Copy)]
enum StyledNode<'a> {
    Attached(WastNode<'a>),
    Raw(SyntaxNode<'a>),
}

impl<'a> StyledNode<'a> {
    fn child(self, index: usize) -> Option<Self> {
        match self {
            Self::Attached(node) => node.children().nth(index).map(Self::Attached),
            Self::Raw(node) => node.children().nth(index).map(Self::Raw),
        }
    }

    fn range(self) -> Range<usize> {
        match self {
            Self::Attached(node) => node.range(),
            Self::Raw(node) => node.range(),
        }
    }

    fn name(self) -> &'a str {
        match self {
            Self::Attached(node) => node.kind().name(),
            Self::Raw(node) => node.name(),
        }
    }

    fn rules(self) -> Option<&'a StyleRules> {
        match self {
            Self::Attached(node) => node.prop(&STYLE_RULES),
            Self::Raw(node) => node.prop(&STYLE_RULES),
        }
    }
}

struct Frame<'a> {
    node: StyledNode<'a>,
    range: Range<usize>,
    next_child: Option<usize>,
    style: Option<usize>,
    inherited: Option<usize>,
}

impl<'a> Frame<'a> {
    fn next(&mut self) -> Option<StyledNode<'a>> {
        let index = self.next_child.as_mut()?;
        let node = self.node.child(*index)?;
        *index += 1;
        Some(node)
    }
}

impl ConfiguredHighlights {
    pub(super) fn new(root: WastNode<'_>) -> Self {
        Self::build(StyledNode::Attached(root))
    }

    pub(crate) fn from_raw(root: SyntaxNode<'_>) -> Self {
        Self::build(StyledNode::Raw(root))
    }

    fn build(root: StyledNode<'_>) -> Self {
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

    fn frame<'a>(
        &mut self,
        node: StyledNode<'a>,
        rule: Option<&Rule>,
        inherited: Option<usize>,
    ) -> Frame<'a> {
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
