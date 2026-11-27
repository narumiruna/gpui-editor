//! `mode/css` streams and factory from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use std::{collections::HashSet, sync::LazyLock};

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod factory;
mod scanner;
mod words;

use scanner::{AtRule, Hooks, Kind, Restriction, Scanner, Style, Token, after_space};
use words::TABLES;

pub use factory::{
    LegacyCss, LegacyCssConfig, LegacyCssHookToken, LegacyCssKeywordOverrides, LegacyCssTokenHook,
    LegacyCssTokenKind, mk_css,
};
pub use words::{CSS_KEYWORDS, CssKeywords};

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Css => "css",
        LegacyMode::Scss => "scss",
        LegacyMode::Less => "less",
        LegacyMode::Gss => "gss",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    data_with_line_comment(nested(mode).then_some("//"))
}

fn data_with_line_comment(line: Option<&'static str>) -> LanguageData {
    LanguageData::default()
        .with(
            "commentTokens",
            CommentTokens {
                line,
                block: Some(("/*", "*/")),
            },
        )
        .with("indentOnInput", indent_on_input as fn(&str) -> bool)
        .with("autocomplete", CSS_KEYWORDS.all)
}

fn indent_on_input(text: &str) -> bool {
    text.trim_start_matches(js_space) == "}"
}

fn nested(mode: LegacyMode) -> bool {
    matches!(mode, LegacyMode::Scss | LegacyMode::Less)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    Top,
    Block,
    MaybeProperty,
    Property,
    PropertyBlock,
    Parens,
    Pseudo,
    DocumentTypes,
    AtBlock,
    AtComponentBlock,
    AtBlockParens,
    BeforeRestrictedBlock,
    RestrictedBlock,
    Keyframes,
    At,
    Interpolation,
}

#[derive(Clone, Copy, Debug)]
struct Frame {
    phase: Phase,
    indent: usize,
}

struct Rules<'a> {
    hooks: Hooks<'a>,
    keywords: &'a LegacyCssKeywordOverrides,
    allow_nested: bool,
    supports_at_component: bool,
    highlight_nonstandard: bool,
}

#[derive(Clone, Debug)]
pub(super) struct State {
    mode: LegacyMode,
    parser: LegacyCssState,
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self {
            mode,
            parser: LegacyCssState::new(false),
        }
    }

    pub(super) fn mode(&self) -> LegacyMode {
        self.mode
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Style {
        static NO_OVERRIDES: LazyLock<LegacyCssKeywordOverrides> =
            LazyLock::new(LegacyCssKeywordOverrides::default);
        self.parser.token(
            stream,
            Rules {
                hooks: match self.mode {
                    LegacyMode::Scss => Hooks::Scss,
                    LegacyMode::Less => Hooks::Less,
                    _ => Hooks::Css,
                },
                keywords: &NO_OVERRIDES,
                allow_nested: nested(self.mode),
                supports_at_component: self.mode == LegacyMode::Gss,
                highlight_nonstandard: true,
            },
        )
    }

    pub(super) fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        self.parser.indent(text_after, indent_unit)
    }
}

/// A checkpoint in a legacy CSS stream. Use it with the parser that created it.
#[derive(Clone, Debug)]
pub struct LegacyCssState {
    scanner: Scanner,
    phase: Phase,
    frames: Vec<Frame>,
    restriction: Restriction,
}

impl LegacyCssState {
    fn new(inline: bool) -> Self {
        let phase = if inline { Phase::Block } else { Phase::Top };
        Self {
            scanner: Scanner::default(),
            phase,
            frames: vec![Frame { phase, indent: 0 }],
            restriction: Restriction::None,
        }
    }

    fn token(&mut self, stream: &mut StringStream<'_>, rules: Rules<'_>) -> Style {
        let Token(kind, mut style) = self.scanner.scan(rules.hooks, stream)?;
        if kind != Kind::Comment {
            self.phase = self.parse(kind, stream, &mut style, &rules);
        }
        style
    }

    fn indent(&self, text_after: &str, indent_unit: usize) -> Option<usize> {
        let mut frames = self.frames.iter().rev();
        let mut frame = frames.next().unwrap();
        let mut indent = frame.indent;
        let ch = text_after.chars().next();
        if frame.phase == Phase::Property && matches!(ch, Some('}' | ')')) {
            frame = frames.next().unwrap();
        }
        if let Some(parent) = frames.next() {
            match (ch, frame.phase) {
                (
                    Some('}'),
                    Phase::Block | Phase::Top | Phase::Interpolation | Phase::RestrictedBlock,
                ) => indent = parent.indent,
                (Some(')'), Phase::Parens | Phase::AtBlockParens)
                | (Some('{'), Phase::At | Phase::AtBlock) => {
                    indent = frame.indent.saturating_sub(indent_unit);
                }
                _ => {}
            }
        }
        Some(indent)
    }

    fn parse(
        &mut self,
        kind: Kind,
        stream: &StringStream<'_>,
        style: &mut Style,
        rules: &Rules<'_>,
    ) -> Phase {
        use Kind::{
            Hash, Interpolation, Mark, Meta, Qualifier, Variable, VariableDefinition, Word,
        };

        // A transient phase can pass the same token to the phase on the stack.
        let mut phase = self.phase;
        let nested = rules.allow_nested;
        let words = rules.keywords;
        loop {
            match (phase, kind) {
                (Phase::Top, Mark('{')) => return self.push(Phase::Block, stream),
                (Phase::Top, Mark('}')) => return self.pop(),
                (Phase::Top, Kind::AtRule(rule)) => {
                    return match rule {
                        AtRule::Document => self.push(Phase::DocumentTypes, stream),
                        AtRule::Block => self.push(Phase::AtBlock, stream),
                        AtRule::Restricted(restriction) => {
                            self.restriction = restriction;
                            Phase::BeforeRestrictedBlock
                        }
                        AtRule::Keyframes => Phase::Keyframes,
                        AtRule::Component if rules.supports_at_component => {
                            self.push(Phase::AtComponentBlock, stream)
                        }
                        _ => self.push(Phase::At, stream),
                    };
                }
                (Phase::Top, Hash) => *style = Some("builtin"),
                (Phase::Top, Word) => *style = Some("tag"),
                (Phase::Top, VariableDefinition) => return Phase::MaybeProperty,
                (Phase::Top, Mark(':')) => return Phase::Pseudo,
                (Phase::Top, Mark('(')) if nested => return self.push(Phase::Parens, stream),

                (Phase::Block, Word) => {
                    let word = stream.current().to_lowercase();
                    if let Some(property) = rules.property_style(&word) {
                        *style = Some(property);
                        return Phase::MaybeProperty;
                    }
                    if nested {
                        *style = Some(match after_space(stream).strip_prefix(':') {
                            Some(tail) if tail.chars().next().is_none_or(js_space) => "property",
                            _ => "tag",
                        });
                        return Phase::Block;
                    }
                    *style = Some("error");
                    return Phase::MaybeProperty;
                }
                (Phase::Block, Meta) => return Phase::Block,
                (Phase::Block, Hash | Qualifier) if !nested => {
                    *style = Some("error");
                    return Phase::Block;
                }
                (Phase::Block, _) => {
                    phase = Phase::Top;
                    continue;
                }

                (Phase::MaybeProperty, Mark(':')) => return self.push(Phase::Property, stream),
                (Phase::Property, Mark(';')) => return self.pop(),
                (Phase::Property, Mark('{')) if nested => {
                    return self.push(Phase::PropertyBlock, stream);
                }
                (Phase::Property, Hash) if !hex_color(stream.current()) => *style = Some("error"),
                (Phase::Property | Phase::Parens, Word) => {
                    *style = Some(rules.value_style(&stream.current().to_lowercase()));
                }
                (Phase::Property | Phase::Parens, Mark('(')) => {
                    return self.push(Phase::Parens, stream);
                }
                (Phase::Top | Phase::Property | Phase::Parens | Phase::AtBlock, Interpolation) => {
                    return self.push(Phase::Interpolation, stream);
                }
                (Phase::PropertyBlock, Word) => {
                    *style = Some("property");
                    return Phase::MaybeProperty;
                }
                (Phase::PropertyBlock, Mark('}')) | (Phase::Parens, Mark(')')) => {
                    return self.pop();
                }

                (Phase::Pseudo, Meta) => return Phase::Pseudo,
                (Phase::Pseudo, Word) => *style = Some("variableName.constant"),
                (Phase::DocumentTypes, Word)
                    if contains(
                        &words.document_types,
                        &TABLES.document_types,
                        stream.current(),
                    ) =>
                {
                    *style = Some("tag");
                }
                (Phase::AtBlock, Word) => {
                    *style = Some(rules.at_block_style(&stream.current().to_lowercase()));
                }
                (Phase::AtBlock, Mark('(')) => return self.push(Phase::AtBlockParens, stream),
                (Phase::AtBlock | Phase::AtComponentBlock, Mark('{')) => {
                    self.pop();
                    let block = if nested { Phase::Block } else { Phase::Top };
                    return if phase == Phase::AtBlock {
                        self.push(block, stream)
                    } else {
                        self.push_at(block, stream.indentation())
                    };
                }
                (Phase::AtComponentBlock, Word) => *style = Some("error"),
                (Phase::AtBlockParens, Mark(')')) => return self.pop(),
                (Phase::AtBlockParens, Mark('{' | '}')) => {
                    self.pop();
                    phase = self.pop();
                    continue;
                }
                (Phase::DocumentTypes | Phase::AtBlockParens, _) => {
                    phase = Phase::AtBlock;
                    continue;
                }

                (Phase::BeforeRestrictedBlock, Mark('{')) => {
                    return self.push(Phase::RestrictedBlock, stream);
                }
                (Phase::BeforeRestrictedBlock, Word)
                    if self.restriction == Restriction::CounterStyle =>
                {
                    *style = Some("variable");
                    return Phase::BeforeRestrictedBlock;
                }
                (Phase::RestrictedBlock, Mark('}')) => {
                    self.restriction = Restriction::None;
                    return self.pop();
                }
                (Phase::RestrictedBlock, Word) => {
                    let word = stream.current().to_lowercase();
                    let known = match self.restriction {
                        Restriction::FontFace => {
                            contains(&words.font_properties, &TABLES.fonts, &word)
                        }
                        Restriction::CounterStyle => contains(
                            &words.counter_descriptors,
                            &TABLES.counter_descriptors,
                            &word,
                        ),
                        Restriction::None => true,
                    };
                    *style = Some(if known { "property" } else { "error" });
                    return Phase::MaybeProperty;
                }
                (Phase::Keyframes, Word) => {
                    *style = Some("variable");
                    return Phase::Keyframes;
                }
                (Phase::Keyframes, Mark('{')) => return self.push(Phase::Top, stream),
                (Phase::At, Mark(';')) | (Phase::Interpolation, Mark('}')) => return self.pop(),
                (Phase::At, Word) => *style = Some("tag"),
                (Phase::At, Hash) => *style = Some("builtin"),
                (Phase::Interpolation, Word) => *style = Some("variable"),
                (Phase::Interpolation, Variable | Mark('(' | ')')) => {}

                (Phase::Property | Phase::Parens | Phase::At, Mark('{' | '}'))
                | (Phase::AtBlock, Mark('}' | ';'))
                | (Phase::AtComponentBlock, Mark('}'))
                | (Phase::Interpolation, Mark('{' | ';')) => {
                    phase = self.pop();
                    continue;
                }
                (
                    Phase::MaybeProperty
                    | Phase::Pseudo
                    | Phase::BeforeRestrictedBlock
                    | Phase::Keyframes,
                    _,
                ) => {
                    phase = self.frame().phase;
                    continue;
                }
                (Phase::Interpolation, _) => *style = Some("error"),
                _ => {}
            }
            return self.frame().phase;
        }
    }

    fn frame(&self) -> Frame {
        *self.frames.last().unwrap()
    }

    fn push(&mut self, phase: Phase, stream: &StringStream<'_>) -> Phase {
        self.push_at(
            phase,
            stream.indentation().saturating_add(stream.indent_unit()),
        )
    }

    fn push_at(&mut self, phase: Phase, indent: usize) -> Phase {
        self.frames.push(Frame { phase, indent });
        phase
    }

    fn pop(&mut self) -> Phase {
        if self.frames.len() > 1 {
            self.frames.pop();
        }
        self.frame().phase
    }
}

fn hex_color(text: &str) -> bool {
    text.strip_prefix('#').is_some_and(|hex| {
        matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn contains(keys: &Option<HashSet<String>>, defaults: &HashSet<String>, word: &str) -> bool {
    keys.as_ref().unwrap_or(defaults).contains(word)
}

impl Rules<'_> {
    fn property_style(&self, word: &str) -> Style {
        if contains(&self.keywords.property_keywords, &TABLES.properties, word) {
            Some("property")
        } else if contains(
            &self.keywords.nonstandard_property_keywords,
            &TABLES.nonstandard_properties,
            word,
        ) {
            Some(if self.highlight_nonstandard {
                "string.special"
            } else {
                "property"
            })
        } else {
            None
        }
    }

    fn value_style(&self, word: &str) -> &'static str {
        if contains(&self.keywords.value_keywords, &TABLES.values, word) {
            "atom"
        } else if contains(&self.keywords.color_keywords, &TABLES.colors, word) {
            "keyword"
        } else {
            "variable"
        }
    }

    fn at_block_style(&self, word: &str) -> &'static str {
        if matches!(word, "only" | "not" | "and" | "or") {
            "keyword"
        } else if contains(&self.keywords.media_types, &TABLES.media_types, word) {
            "attribute"
        } else if contains(&self.keywords.media_features, &TABLES.media_features, word) {
            "property"
        } else if contains(
            &self.keywords.media_value_keywords,
            &TABLES.media_values,
            word,
        ) {
            "keyword"
        } else if let Some(style) = self.property_style(word) {
            style
        } else {
            match self.value_style(word) {
                "variable" => "error",
                style => style,
            }
        }
    }
}
