use crate::{
    extension::FacetId,
    language::wast::{WastDataFacet, WastNodeType, WastNodeTypeId},
    language_data::NativeLanguage,
};

use super::{HighlightTag, TagKey, TagModifier, modifiers::ModifierPattern};

pub use crate::theme_one_dark::{Color, HighlightStyle as HighlightProperties, TextDecoration};

/// A native tag selector. `new` matches named tags. Use `From<&HighlightTag>`
/// to match a custom definition, independently of its name. A more specific
/// tag can fall back to this selector.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightSelector<'a> {
    key: TagKey<'a>,
    modifiers: ModifierPattern<'a>,
}

impl<'a> HighlightSelector<'a> {
    pub const fn new(name: &'a str) -> Self {
        Self {
            key: TagKey::Named(name),
            modifiers: ModifierPattern::EMPTY,
        }
    }

    #[must_use]
    pub const fn with_modifier(mut self, modifier: TagModifier) -> Self {
        self.modifiers.standard |= modifier.bit();
        self
    }
}

impl<'a> From<&'a HighlightTag> for HighlightSelector<'a> {
    fn from(tag: &'a HighlightTag) -> Self {
        Self {
            key: tag.key(),
            modifiers: tag.modifiers.pattern(),
        }
    }
}

/// Explicit resets for competing highlight declarations on the same span.
/// A `false` field does not declare a reset.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HighlightResets {
    /// Set font weight to normal.
    pub normal_weight: bool,
    /// Set font style to normal.
    pub normal_style: bool,
    /// Set text decoration to none.
    pub no_decoration: bool,
}

impl HighlightResets {
    pub const NONE: Self = Self {
        normal_weight: false,
        normal_style: false,
        no_decoration: false,
    };
}

/// Native declarations shared by one or more tag selectors. Construct a
/// rule with [`Self::new`] and add explicit resets with [`Self::with_resets`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightRule<'a> {
    pub selectors: &'a [HighlightSelector<'a>],
    /// Property values before explicit resets. `false` and `None` are unset.
    pub properties: HighlightProperties,
    resets: HighlightResets,
}

impl<'a> HighlightRule<'a> {
    pub const fn new(
        selectors: &'a [HighlightSelector<'a>],
        properties: HighlightProperties,
    ) -> Self {
        Self {
            selectors,
            properties,
            resets: HighlightResets::NONE,
        }
    }

    /// Declare resets that override the corresponding values in `properties`.
    #[must_use]
    pub const fn with_resets(mut self, resets: HighlightResets) -> Self {
        self.resets = resets;
        self
    }
}

/// Appearance used to select active styles.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightTheme {
    Light,
    Dark,
}

/// Scope information for a parsed span. For WAST, it includes the exact
/// root row and the effective language-data facet on that row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightContext {
    language: NativeLanguage,
    wast_node_type: Option<WastNodeTypeId>,
    wast_facet: Option<FacetId>,
}

impl HighlightContext {
    pub fn language(self) -> NativeLanguage {
        self.language
    }

    pub(crate) fn native(language: NativeLanguage) -> Self {
        Self {
            language,
            wast_node_type: None,
            wast_facet: None,
        }
    }

    pub(crate) fn from_wast_root(row: &WastNodeType) -> Option<Self> {
        row.is_top().then(|| Self {
            language: NativeLanguage::Wast,
            wast_node_type: Some(row.id()),
            wast_facet: row.data_facet().map(WastDataFacet::id),
        })
    }
}

/// An ordered set of native syntax highlight rules for one span.
///
/// This resolves foreground, bold, italic, and a single text decoration. It
/// does not generate CSS classes or traverse a syntax tree. Resets cancel
/// competing highlight declarations on this span only. A winning reset is
/// returned as `false` or `None`, like an unset property. It does not reset
/// the host's base style.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HighlightStyle<'a> {
    rules: &'a [HighlightRule<'a>],
    scope: Option<Scope<'a>>,
    all: HighlightProperties,
    all_resets: HighlightResets,
    theme: Option<HighlightTheme>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope<'a> {
    Tag(&'a HighlightTag),
    Native(NativeLanguage),
    WastFacet(FacetId),
    WastNodeType(WastNodeTypeId),
}

#[derive(Clone, Copy)]
enum ScopeInput<'a> {
    Tag(Option<&'a HighlightTag>),
    Parsed(Option<HighlightContext>),
}

impl Scope<'_> {
    fn matches(self, input: ScopeInput<'_>) -> bool {
        match (self, input) {
            (Self::Tag(scope), ScopeInput::Tag(Some(tag))) => scope == tag,
            (Self::Native(scope), ScopeInput::Parsed(Some(context))) => scope == context.language,
            (Self::WastFacet(scope), ScopeInput::Parsed(Some(context))) => {
                Some(scope) == context.wast_facet
            }
            (Self::WastNodeType(scope), ScopeInput::Parsed(Some(context))) => {
                Some(scope) == context.wast_node_type
            }
            _ => false,
        }
    }
}

impl<'a> HighlightStyle<'a> {
    pub const fn define(rules: &'a [HighlightRule<'a>]) -> Self {
        Self {
            rules,
            scope: None,
            all: HighlightProperties::EMPTY,
            all_resets: HighlightResets::NONE,
            theme: None,
        }
    }

    /// Restrict this style to a matching caller-supplied scope tag (including
    /// its modifiers). The scope is separate from the span's token tags.
    /// The host supplies the tag for the current syntax region; matching uses
    /// value equality, not parser or node-type identity.
    #[must_use]
    pub const fn with_scope(mut self, scope: &'a HighlightTag) -> Self {
        self.scope = Some(Scope::Tag(scope));
        self
    }

    /// Restrict this style to this native language variant, including its options.
    /// This replaces any previous scope. Use [`highlighting_for_native`] or
    /// [`highlighting_for_context`] to resolve this style.
    #[must_use]
    pub const fn with_native_scope(mut self, language: NativeLanguage) -> Self {
        self.scope = Some(Scope::Native(language));
        self
    }

    /// Restrict this style to WAST roots with this effective data facet.
    /// Sublanguage predicates do not affect the match. The style stores the
    /// facet identity by value and replaces any previous scope. Use
    /// [`highlighting_for_context`] to resolve this style.
    #[must_use]
    pub fn with_wast_facet_scope(mut self, facet: &WastDataFacet) -> Self {
        self.scope = Some(Scope::WastFacet(facet.id()));
        self
    }

    /// Restrict this style to this exact WAST root row, not its name or term.
    /// A non-root row cannot match a parsed span. The style stores the row
    /// identity by value and replaces any previous scope. Use
    /// [`highlighting_for_context`] to resolve this style.
    #[must_use]
    pub fn with_wast_node_type_scope(mut self, row: &WastNodeType) -> Self {
        self.scope = Some(Scope::WastNodeType(row.id()));
        self
    }

    /// Set properties for every span in scope, including spans with no tags.
    /// Declarations from matching rules take precedence.
    #[must_use]
    pub const fn with_all(mut self, properties: HighlightProperties) -> Self {
        self.all = properties;
        self
    }

    /// Declare resets for every span in scope, including spans with no tags.
    /// These override the corresponding values set by [`Self::with_all`].
    /// Declarations from matching rules take precedence.
    #[must_use]
    pub const fn with_all_resets(mut self, resets: HighlightResets) -> Self {
        self.all_resets = resets;
        self
    }

    /// Restrict this style to one appearance when used as an active layer.
    #[must_use]
    pub const fn with_theme(mut self, theme: HighlightTheme) -> Self {
        self.theme = Some(theme);
        self
    }

    /// Resolve one span without layer or appearance selection. For each tag,
    /// use the most specific rule from its definition or name and its
    /// ancestors. For identical selectors the last rule
    /// wins as a whole. Across separately matched tags the later rule wins
    /// for each property it sets.
    #[must_use]
    pub fn resolve(
        &self,
        tags: &[HighlightTag],
        scope: Option<&HighlightTag>,
    ) -> HighlightProperties {
        self.resolve_in(tags, ScopeInput::Tag(scope)).properties()
    }

    fn resolve_in(&self, tags: &[HighlightTag], input: ScopeInput<'_>) -> Declarations {
        if self.scope.is_some_and(|scope| !scope.matches(input)) {
            return Declarations::default();
        }
        let mut foreground = None;
        let mut bold = None;
        let mut italic = None;
        let mut decoration = None;
        for tag in tags {
            let Some(index) = self.matching_rule(tag) else {
                continue;
            };
            let rule = self.rules[index];
            let declarations = Declarations::new(rule.properties, rule.resets);
            keep_later(&mut foreground, index, declarations.foreground);
            keep_later(&mut bold, index, declarations.bold);
            keep_later(&mut italic, index, declarations.italic);
            keep_later(&mut decoration, index, declarations.text_decoration);
        }
        let mut style = Declarations::new(self.all, self.all_resets);
        style.apply(Declarations {
            foreground: foreground.map(|(_, value)| value),
            bold: bold.map(|(_, value)| value),
            italic: italic.map(|(_, value)| value),
            text_decoration: decoration.map(|(_, value)| value),
        });
        style
    }

    fn matching_rule(&self, tag: &HighlightTag) -> Option<usize> {
        let selector = HighlightSelector::from(tag);
        let tag_modifiers = selector.modifiers;
        let mut parents = tag.parent_keys();
        let mut key = Some(selector.key);
        while let Some(base) = key {
            let mut best = None;
            for (index, rule) in self.rules.iter().enumerate().rev() {
                for selector in rule.selectors {
                    let modifiers = selector.modifiers;
                    if selector.key != base || !modifiers.is_subset_of(tag_modifiers) {
                        continue;
                    }
                    if modifiers == tag_modifiers {
                        return Some(index);
                    }
                    if best
                        .is_none_or(|(previous, _)| modifiers.compare_specificity(previous).is_gt())
                    {
                        best = Some((modifiers, index));
                    }
                }
            }
            if let Some((_, index)) = best {
                return Some(index);
            }
            key = match base {
                TagKey::Named(name) => parent_tag(name)
                    .map(TagKey::Named)
                    .or_else(|| parents.next()),
                TagKey::Defined(_) => parents.next(),
            };
        }
        None
    }
}

fn keep_later<T>(best: &mut Option<(usize, T)>, index: usize, value: Option<T>) {
    if let Some(value) = value
        && best.as_ref().is_none_or(|(earlier, _)| index > *earlier)
    {
        *best = Some((index, value));
    }
}

// Keep absent and reset distinct until all competing rules and layers resolve.
#[derive(Default)]
struct Declarations {
    foreground: Option<Color>,
    bold: Option<bool>,
    italic: Option<bool>,
    text_decoration: Option<Option<TextDecoration>>,
}

impl Declarations {
    fn new(properties: HighlightProperties, resets: HighlightResets) -> Self {
        Self {
            foreground: properties.foreground,
            bold: (resets.normal_weight || properties.bold).then_some(!resets.normal_weight),
            italic: (resets.normal_style || properties.italic).then_some(!resets.normal_style),
            text_decoration: if resets.no_decoration {
                Some(None)
            } else {
                properties.text_decoration.map(Some)
            },
        }
    }

    fn apply(&mut self, higher: Self) {
        self.foreground = higher.foreground.or(self.foreground);
        self.bold = higher.bold.or(self.bold);
        self.italic = higher.italic.or(self.italic);
        self.text_decoration = higher.text_decoration.or(self.text_decoration);
    }

    fn properties(&self) -> HighlightProperties {
        HighlightProperties {
            foreground: self.foreground,
            bold: self.bold.unwrap_or(false),
            italic: self.italic.unwrap_or(false),
            text_decoration: self.text_decoration.flatten(),
        }
    }
}

/// An entry in extension precedence order for native syntax highlighting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightLayer<'a> {
    Active(HighlightStyle<'a>),
    Fallback(HighlightStyle<'a>),
    /// An active highlighter resolved by the host, for example One Dark.
    Resolved(HighlightProperties),
}

/// Resolve the native properties for one span in extension precedence order.
///
/// The first active layer that sets a property wins, as with separately
/// mounted equal-specificity style modules. Theme-mismatched active layers
/// are inactive. Scope-mismatched and unmatched active layers still prevent
/// fallback. With no active layers, only the first fallback is used, even
/// if it has no match. A fallback's appearance restriction is ignored.
/// Explicit resets take part in precedence; the returned properties do not
/// distinguish a winning reset from an unset property.
#[must_use]
pub fn highlighting_for(
    layers: &[HighlightLayer<'_>],
    tags: &[HighlightTag],
    scope: Option<&HighlightTag>,
    theme: HighlightTheme,
) -> HighlightProperties {
    highlighting_for_scope(layers, tags, ScopeInput::Tag(scope), theme)
}

/// Resolve layers using only the native language. `None` matches only
/// unscoped styles. Tag and WAST identity scopes are not considered. Layer
/// precedence and fallback selection are the same as in [`highlighting_for`].
#[must_use]
pub fn highlighting_for_native(
    layers: &[HighlightLayer<'_>],
    tags: &[HighlightTag],
    language: Option<NativeLanguage>,
    theme: HighlightTheme,
) -> HighlightProperties {
    highlighting_for_context(layers, tags, language.map(HighlightContext::native), theme)
}

/// Resolve layers using context from parsed syntax. Native language, WAST
/// facet, and WAST root-row scopes can match in the same stack. Tag scopes
/// are not considered. `None` matches only unscoped styles. Layer precedence
/// and fallback selection are the same as in [`highlighting_for`].
#[must_use]
pub fn highlighting_for_context(
    layers: &[HighlightLayer<'_>],
    tags: &[HighlightTag],
    context: Option<HighlightContext>,
    theme: HighlightTheme,
) -> HighlightProperties {
    highlighting_for_scope(layers, tags, ScopeInput::Parsed(context), theme)
}

fn highlighting_for_scope(
    layers: &[HighlightLayer<'_>],
    tags: &[HighlightTag],
    scope: ScopeInput<'_>,
    theme: HighlightTheme,
) -> HighlightProperties {
    let mut output = Declarations::default();
    let mut has_active = false;
    let mut fallback = None;
    for layer in layers.iter().rev() {
        let declarations = match *layer {
            HighlightLayer::Active(style) if style.theme.is_none_or(|value| value == theme) => {
                style.resolve_in(tags, scope)
            }
            HighlightLayer::Active(_) => continue,
            HighlightLayer::Fallback(style) => {
                fallback = Some(style);
                continue;
            }
            HighlightLayer::Resolved(properties) => {
                Declarations::new(properties, HighlightResets::NONE)
            }
        };
        has_active = true;
        output.apply(declarations);
    }
    if !has_active && let Some(style) = fallback {
        output = style.resolve_in(tags, scope);
    }
    output.properties()
}

// Standard tag parents from @lezer/highlight 1.2.5.
fn parent_tag(name: &str) -> Option<&'static str> {
    Some(match name {
        "lineComment" | "blockComment" | "docComment" => "comment",
        "variableName" | "typeName" | "propertyName" | "className" | "labelName" | "namespace"
        | "macroName" => "name",
        "tagName" => "typeName",
        "attributeName" => "propertyName",
        "string" | "number" | "bool" | "regexp" | "escape" | "color" | "url" => "literal",
        "docString" | "character" | "attributeValue" => "string",
        "integer" | "float" => "number",
        "self" | "null" | "atom" | "unit" | "modifier" | "operatorKeyword" | "controlKeyword"
        | "definitionKeyword" | "moduleKeyword" => "keyword",
        "derefOperator" | "arithmeticOperator" | "logicOperator" | "bitwiseOperator"
        | "compareOperator" | "updateOperator" | "definitionOperator" | "typeOperator"
        | "controlOperator" => "operator",
        "separator" | "bracket" => "punctuation",
        "angleBracket" | "squareBracket" | "paren" | "brace" => "bracket",
        "heading" | "contentSeparator" | "list" | "quote" | "emphasis" | "strong" | "link"
        | "monospace" | "strikethrough" => "content",
        "heading1" | "heading2" | "heading3" | "heading4" | "heading5" | "heading6" => "heading",
        "documentMeta" | "annotation" | "processingInstruction" => "meta",
        _ => return None,
    })
}
