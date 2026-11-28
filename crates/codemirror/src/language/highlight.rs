use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

mod default_style;
mod modifiers;
mod resolve;

pub use default_style::DEFAULT_HIGHLIGHT_STYLE;
use modifiers::Modifiers;
pub use modifiers::{CustomTagModifier, TagModifier};
pub use resolve::{
    Color, HighlightContext, HighlightLayer, HighlightProperties, HighlightResets, HighlightRule,
    HighlightSelector, HighlightStyle, HighlightTheme, TextDecoration, highlighting_for,
    highlighting_for_context, highlighting_for_native,
};

/// Custom base names in a mode's token styles. A name can map to multiple tags.
/// An exact legacy name such as `variable` keeps its built-in meaning.
pub type TokenTable = HashMap<String, Vec<HighlightTag>>;

/// A highlight category and its modifiers, independent of themes and rendering.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HighlightTag {
    base: Base,
    modifiers: Modifiers,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Base {
    Named(String),
    Defined(Arc<Definition>),
}

#[derive(Debug, Eq, PartialEq)]
struct Definition {
    id: usize,
    name: String,
    parents: Vec<Parent>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Parent {
    Named(String),
    Defined(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TagKey<'a> {
    Named(&'a str),
    Defined(usize),
}

impl Definition {
    fn new(name: String, parents: Vec<Parent>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .expect("highlight tag identities exhausted");
        Self { id, name, parents }
    }
}

impl HighlightTag {
    /// Make a named tag. Tags made this way match selectors by name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            base: Base::Named(name.into()),
            modifiers: Modifiers::default(),
        }
    }

    /// Define a custom tag. Its name is descriptive; a selector made from a
    /// different definition or from the same name does not match it.
    pub fn define(name: impl Into<String>) -> Self {
        Self {
            base: Base::Defined(Arc::new(Definition::new(name.into(), Vec::new()))),
            modifiers: Modifiers::default(),
        }
    }

    /// Define a distinct custom tag with a less-specific parent. Return `None`
    /// if the parent has modifiers. Matching tries the new tag first, then
    /// each parent in order, with the same modifier subsets at each level.
    pub fn derive(name: impl Into<String>, parent: &Self) -> Option<Self> {
        if !parent.modifiers.is_empty() {
            return None;
        }
        let parents = match &parent.base {
            Base::Named(name) => vec![Parent::Named(name.clone())],
            Base::Defined(definition) => std::iter::once(Parent::Defined(definition.id))
                .chain(definition.parents.iter().cloned())
                .collect(),
        };
        Some(Self {
            base: Base::Defined(Arc::new(Definition::new(name.into(), parents))),
            modifiers: Modifiers::default(),
        })
    }

    pub fn with_modifier(mut self, modifier: TagModifier) -> Self {
        self.modifiers.add_standard(modifier);
        self
    }

    /// Add a custom modifier by identity. Order and repeated application do
    /// not change the result.
    #[must_use]
    pub fn with_custom_modifier(mut self, modifier: &CustomTagModifier) -> Self {
        self.modifiers.add_custom(modifier);
        self
    }

    pub fn name(&self) -> &str {
        match &self.base {
            Base::Named(name) => name,
            Base::Defined(definition) => &definition.name,
        }
    }

    pub(super) fn key(&self) -> TagKey<'_> {
        match &self.base {
            Base::Named(name) => TagKey::Named(name),
            Base::Defined(definition) => TagKey::Defined(definition.id),
        }
    }

    pub(super) fn parent_keys(&self) -> impl Iterator<Item = TagKey<'_>> {
        let parents = match &self.base {
            Base::Named(_) => &[][..],
            Base::Defined(definition) => definition.parents.as_slice(),
        };
        parents.iter().map(|parent| match parent {
            Parent::Named(name) => TagKey::Named(name),
            Parent::Defined(id) => TagKey::Defined(*id),
        })
    }

    /// Standard modifiers in definition order.
    pub fn modifiers(&self) -> &[TagModifier] {
        self.modifiers.standard()
    }

    /// Custom modifiers in definition order.
    pub fn custom_modifiers(&self) -> &[CustomTagModifier] {
        self.modifiers.custom()
    }
}

#[derive(Eq, PartialEq)]
pub(super) struct Style {
    // Equal tag lists can still come from different node names. Only merge
    // adjacent tokens that would have the same node type in StreamLanguage.
    name: String,
    pub(super) tags: Vec<HighlightTag>,
}

impl Style {
    pub(super) fn resolve(raw: &str, custom: &TokenTable) -> Option<Self> {
        let alias = legacy_alias(raw);
        let name = alias.unwrap_or(raw);
        let custom = alias.is_none().then_some(custom);
        let mut tags = Vec::new();
        for group in name.split(' ') {
            let start = tags.len();
            for part in group.split('.') {
                if let Some(custom_tags) = custom.and_then(|table| table.get(part)) {
                    if tags.len() == start {
                        tags.extend_from_slice(custom_tags);
                    }
                } else if let Some(modifier) = TagModifier::from_name(part) {
                    for tag in &mut tags[start..] {
                        tag.modifiers.add_standard(modifier);
                    }
                } else if tags.len() == start && BUILTIN_TAGS.contains(&part) {
                    tags.push(HighlightTag::new(part));
                }
            }
        }
        (!tags.is_empty()).then(|| Self {
            name: name.replace(' ', "_"),
            tags,
        })
    }
}

fn legacy_alias(raw: &str) -> Option<&'static str> {
    Some(match raw {
        "variable" => "variableName",
        "variable-2" => "variableName.special",
        "string-2" => "string.special",
        "def" => "variableName.definition",
        "tag" => "tagName",
        "attribute" => "attributeName",
        "type" => "typeName",
        "builtin" => "variableName.standard",
        "qualifier" => "modifier",
        "error" => "invalid",
        "header" => "heading",
        "property" => "propertyName",
        _ => return None,
    })
}

#[rustfmt::skip]
const BUILTIN_TAGS: &[&str] = &[
    "comment", "lineComment", "blockComment", "docComment",
    "name", "variableName", "typeName", "tagName", "propertyName", "attributeName",
    "className", "labelName", "namespace", "macroName",
    "literal", "string", "docString", "character", "attributeValue", "number",
    "integer", "float", "bool", "regexp", "escape", "color", "url",
    "keyword", "self", "null", "atom", "unit", "modifier", "operatorKeyword",
    "controlKeyword", "definitionKeyword", "moduleKeyword",
    "operator", "derefOperator", "arithmeticOperator", "logicOperator", "bitwiseOperator",
    "compareOperator", "updateOperator", "definitionOperator", "typeOperator", "controlOperator",
    "punctuation", "separator", "bracket", "angleBracket", "squareBracket", "paren", "brace",
    "content", "heading", "heading1", "heading2", "heading3", "heading4", "heading5", "heading6",
    "contentSeparator", "list", "quote", "emphasis", "strong", "link", "monospace", "strikethrough",
    "inserted", "deleted", "changed", "invalid", "meta", "documentMeta", "annotation",
    "processingInstruction",
];
