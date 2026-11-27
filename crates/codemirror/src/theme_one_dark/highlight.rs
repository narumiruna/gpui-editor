use crate::language::{
    HighlightRule, HighlightSelector as Tag, HighlightStyle as StyleDefinition, HighlightTag,
    TagModifier::{Constant, Definition, Function, Special, Standard},
};

use super::{COLOR, Color};

/// Value of the `text-decoration` property set by One Dark.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextDecoration {
    LineThrough,
    Underline,
}

/// Text properties set by One Dark on a syntax span.
///
/// `None` and `false` mean One Dark does not set the property. They do not
/// override the span's base style.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HighlightStyle {
    pub foreground: Option<Color>,
    pub bold: bool,
    pub italic: bool,
    pub text_decoration: Option<TextDecoration>,
}

impl HighlightStyle {
    pub(crate) const EMPTY: Self = Self {
        foreground: None,
        bold: false,
        italic: false,
        text_decoration: None,
    };

    pub(crate) const fn color(foreground: Color) -> Self {
        Self {
            foreground: Some(foreground),
            ..Self::EMPTY
        }
    }

    pub(crate) fn apply(&mut self, rule: Self) {
        self.foreground = rule.foreground.or(self.foreground);
        self.bold |= rule.bold;
        self.italic |= rule.italic;
        self.text_decoration = rule.text_decoration.or(self.text_decoration);
    }
}

/// An active highlighter in a native stack for one span.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HighlightLayer {
    /// The unscoped `oneDarkHighlightStyle` export.
    OneDark,
    /// Another highlighter's properties for this span. Use an empty style if
    /// that highlighter is active but does not match the span's tags.
    Resolved(HighlightStyle),
}

/// Resolves the supported properties of one span in extension precedence order.
///
/// The first active layer that sets a property wins, as with equal-specificity
/// rules in separately mounted highlighter modules. Use the first fallback
/// highlighter's resolved style, or an empty style if there is none. Fallback
/// applies only when the active list is empty, not when a tag is unmatched.
///
/// The host selects active highlighters for the editor's appearance. Keep an
/// active highlighter in the list with an empty style if its language scope
/// excludes this span. One Dark has no appearance or language restriction.
/// These layers do not model CSS selectors, ancestor decoration, or explicit
/// resets to normal font weight, normal font style, or no decoration.
#[must_use]
pub fn layered_highlight_style(
    tags: &[HighlightTag],
    active: &[HighlightLayer],
    fallback: HighlightStyle,
) -> HighlightStyle {
    if active.is_empty() {
        return fallback;
    }
    let mut style = HighlightStyle::EMPTY;
    for layer in active.iter().rev() {
        style.apply(match *layer {
            HighlightLayer::OneDark => highlight_style(tags),
            HighlightLayer::Resolved(style) => style,
        });
    }
    style
}

/// Native rules for `oneDarkHighlightStyle`, without appearance or scope restrictions.
/// Use [`super::one_dark_highlighting`] to install them as an active highlighter.
pub const ONE_DARK_HIGHLIGHT_STYLE: StyleDefinition<'static> = StyleDefinition::define(RULES);
const EMPTY: HighlightStyle = HighlightStyle::EMPTY;

#[rustfmt::skip]
const RULES: &[HighlightRule<'static>] = &[
    HighlightRule::new(&[Tag::new("keyword")], HighlightStyle::color(COLOR.violet)),
    HighlightRule::new(&[
        Tag::new("name"), Tag::new("deleted"), Tag::new("character"), Tag::new("propertyName"), Tag::new("macroName"),
    ], HighlightStyle::color(COLOR.coral)),
    HighlightRule::new(&[
        Tag::new("labelName"), Tag::new("variableName").with_modifier(Function),
    ], HighlightStyle::color(COLOR.malibu)),
    HighlightRule::new(&[
        Tag::new("color"), Tag::new("name").with_modifier(Constant), Tag::new("name").with_modifier(Standard),
    ], HighlightStyle::color(COLOR.whiskey)),
    HighlightRule::new(&[
        Tag::new("separator"), Tag::new("name").with_modifier(Definition),
    ], HighlightStyle::color(COLOR.ivory)),
    HighlightRule::new(&[
        Tag::new("typeName"), Tag::new("className"), Tag::new("number"), Tag::new("changed"),
        Tag::new("annotation"), Tag::new("modifier"), Tag::new("self"), Tag::new("namespace"),
    ], HighlightStyle::color(COLOR.chalky)),
    HighlightRule::new(&[
        Tag::new("operator"), Tag::new("operatorKeyword"), Tag::new("url"), Tag::new("escape"),
        Tag::new("regexp"), Tag::new("link"), Tag::new("string").with_modifier(Special),
    ], HighlightStyle::color(COLOR.cyan)),
    HighlightRule::new(&[Tag::new("meta"), Tag::new("comment")], HighlightStyle::color(COLOR.stone)),
    HighlightRule::new(&[Tag::new("strong")], HighlightStyle { bold: true, ..EMPTY }),
    HighlightRule::new(&[Tag::new("emphasis")], HighlightStyle { italic: true, ..EMPTY }),
    HighlightRule::new(&[Tag::new("strikethrough")], HighlightStyle {
        text_decoration: Some(TextDecoration::LineThrough),
        ..EMPTY
    }),
    HighlightRule::new(&[Tag::new("link")], HighlightStyle {
        foreground: Some(COLOR.stone),
        text_decoration: Some(TextDecoration::Underline),
        ..EMPTY
    }),
    HighlightRule::new(&[Tag::new("heading")], HighlightStyle {
        foreground: Some(COLOR.coral),
        bold: true,
        ..EMPTY
    }),
    HighlightRule::new(&[
        Tag::new("atom"), Tag::new("bool"), Tag::new("variableName").with_modifier(Special),
    ], HighlightStyle::color(COLOR.whiskey)),
    HighlightRule::new(&[
        Tag::new("processingInstruction"), Tag::new("string"), Tag::new("inserted"),
    ], HighlightStyle::color(COLOR.sage)),
    HighlightRule::new(&[Tag::new("invalid")], HighlightStyle::color(COLOR.invalid)),
];

/// Returns the One Dark properties for the tags of one span.
///
/// Match each tag by specificity: try the base with its modifiers, then the
/// unmodified base, then its parents in the same way. If separate tags set
/// the same property, the later One Dark rule wins, independent of input
/// order. A custom definition needs a standard ancestor to match One Dark.
///
/// One Dark has no language scope restriction. The caller supplies the tags
/// that apply to this span, after any language or syntax node path matching.
/// This function does not traverse syntax trees or combine other themes.
#[must_use]
pub fn highlight_style(tags: &[HighlightTag]) -> HighlightStyle {
    ONE_DARK_HIGHLIGHT_STYLE.resolve(tags, None)
}
