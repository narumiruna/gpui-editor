//! Grammars from `@codemirror/legacy-modes` 6.5.5, compiled by the factory.

use std::sync::LazyLock;

use super::{SimpleMode, SimpleModeOptions, SimpleRule, SimpleToken, simple_mode};
use crate::{commands::CommentTokens, language::LanguageData, legacy_modes::LegacyMode};

mod factor;
mod nsis;
mod wast;

pub(in crate::legacy_modes) fn bundled(mode: LegacyMode) -> &'static SimpleMode {
    static FACTOR: LazyLock<SimpleMode> =
        LazyLock::new(|| build(LegacyMode::Factor, factor::states()));
    static NSIS: LazyLock<SimpleMode> = LazyLock::new(|| build(LegacyMode::Nsis, nsis::states()));
    static WAST: LazyLock<SimpleMode> = LazyLock::new(|| build(LegacyMode::Wast, wast::states()));
    match mode {
        LegacyMode::Factor => &FACTOR,
        LegacyMode::Nsis => &NSIS,
        LegacyMode::Wast => &WAST,
        _ => unreachable!("not a bundled simple mode"),
    }
}

pub(in crate::legacy_modes) fn data(mode: LegacyMode) -> LanguageData {
    let (name, dont_indent) = metadata(mode);
    let mut data = LanguageData::default().with("name", name);
    if !dont_indent.is_empty() {
        data = data.with("dontIndentStates", dont_indent);
    }
    match mode {
        LegacyMode::Factor => data.with(
            "commentTokens",
            CommentTokens {
                line: Some("!"),
                block: None,
            },
        ),
        LegacyMode::Nsis => data
            .with(
                "commentTokens",
                CommentTokens {
                    line: Some("#"),
                    block: Some(("/*", "*/")),
                },
            )
            .with("indentOnInput", nsis::indent_on_input as fn(&str) -> bool),
        LegacyMode::Wast => data,
        _ => unreachable!("not a bundled simple mode"),
    }
}

fn metadata(mode: LegacyMode) -> (&'static str, &'static [&'static str]) {
    match mode {
        LegacyMode::Factor => (
            "factor",
            &["start", "vocabulary", "string", "string3", "stack"],
        ),
        LegacyMode::Nsis => ("nsis", &[]),
        LegacyMode::Wast => ("wast", &["comment"]),
        _ => unreachable!("not a bundled simple mode"),
    }
}

fn build(
    mode: LegacyMode,
    states: impl IntoIterator<Item = (&'static str, Vec<SimpleRule>)>,
) -> SimpleMode {
    let (name, dont_indent) = metadata(mode);
    simple_mode(
        states,
        SimpleModeOptions {
            name: Some(name.into()),
            dont_indent_states: dont_indent.iter().map(|&name| name.into()).collect(),
            language_data: data(mode),
            ..SimpleModeOptions::default()
        },
    )
    .unwrap_or_else(|error| panic!("{name} grammar: {error}"))
}

fn styled(regex: &str, style: &str) -> SimpleRule {
    SimpleRule {
        token: SimpleToken::Style(style.into()),
        ..SimpleRule::new(regex)
    }
}

fn groups(regex: &str, styles: &[Option<&str>]) -> SimpleRule {
    SimpleRule {
        token: SimpleToken::Groups(
            styles
                .iter()
                .map(|style| style.map(str::to_owned))
                .collect(),
        ),
        ..SimpleRule::new(regex)
    }
}

fn next(rule: SimpleRule, state: &str) -> SimpleRule {
    SimpleRule {
        next: Some(state.into()),
        ..rule
    }
}
