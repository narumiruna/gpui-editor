//! `defaultHighlightStyle` from `@codemirror/language` 6.13.1 (MIT).

use super::TagModifier::{Definition, Local, Special};
use super::resolve::{
    Color, HighlightProperties, HighlightRule, HighlightSelector as Tag, HighlightStyle,
    TextDecoration,
};

/// The CodeMirror default style. Both upstream setup presets use it as a fallback.
pub const DEFAULT_HIGHLIGHT_STYLE: HighlightStyle<'static> = HighlightStyle::define(RULES);

const EMPTY: HighlightProperties = HighlightProperties::EMPTY;

const fn rgb(color: u32) -> HighlightProperties {
    HighlightProperties::color(Color::from_rgba32((color << 8) | 0xff))
}

#[rustfmt::skip]
const RULES: &[HighlightRule<'static>] = &[
    HighlightRule::new(&[Tag::new("meta")], rgb(0x404740)),
    HighlightRule::new(&[Tag::new("link")], HighlightProperties {
        text_decoration: Some(TextDecoration::Underline),
        ..EMPTY
    }),
    HighlightRule::new(&[Tag::new("heading")], HighlightProperties {
        bold: true,
        text_decoration: Some(TextDecoration::Underline),
        ..EMPTY
    }),
    HighlightRule::new(&[Tag::new("emphasis")], HighlightProperties { italic: true, ..EMPTY }),
    HighlightRule::new(&[Tag::new("strong")], HighlightProperties { bold: true, ..EMPTY }),
    HighlightRule::new(&[Tag::new("strikethrough")], HighlightProperties {
        text_decoration: Some(TextDecoration::LineThrough),
        ..EMPTY
    }),
    HighlightRule::new(&[Tag::new("keyword")], rgb(0x770088)),
    HighlightRule::new(&[
        Tag::new("atom"), Tag::new("bool"), Tag::new("url"), Tag::new("contentSeparator"), Tag::new("labelName"),
    ], rgb(0x221199)),
    HighlightRule::new(&[Tag::new("literal"), Tag::new("inserted")], rgb(0x116644)),
    HighlightRule::new(&[Tag::new("string"), Tag::new("deleted")], rgb(0xaa1111)),
    HighlightRule::new(&[
        Tag::new("regexp"), Tag::new("escape"), Tag::new("string").with_modifier(Special),
    ], rgb(0xee4400)),
    HighlightRule::new(&[Tag::new("variableName").with_modifier(Definition)], rgb(0x0000ff)),
    HighlightRule::new(&[Tag::new("variableName").with_modifier(Local)], rgb(0x3300aa)),
    HighlightRule::new(&[Tag::new("typeName"), Tag::new("namespace")], rgb(0x008855)),
    HighlightRule::new(&[Tag::new("className")], rgb(0x116677)),
    HighlightRule::new(&[Tag::new("variableName").with_modifier(Special), Tag::new("macroName")], rgb(0x225566)),
    HighlightRule::new(&[Tag::new("propertyName").with_modifier(Definition)], rgb(0x0000cc)),
    HighlightRule::new(&[Tag::new("comment")], rgb(0x994400)),
    HighlightRule::new(&[Tag::new("invalid")], rgb(0xff0000)),
];
