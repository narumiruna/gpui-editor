//! Python, Cython, Ruby, Groovy, and PowerShell streams from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the upstream MIT notice.

use super::{LegacyMode, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod groovy;
mod powershell;
mod python;
pub(super) mod python_factory;
mod ruby;

pub(super) fn name(mode: LegacyMode) -> Option<&'static str> {
    Some(match mode {
        LegacyMode::Python | LegacyMode::Cython => "python",
        LegacyMode::Ruby => "ruby",
        LegacyMode::Groovy => "groovy",
        LegacyMode::PowerShell => "powershell",
        _ => unreachable!(),
    })
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    let comments = match mode {
        LegacyMode::Python | LegacyMode::Cython | LegacyMode::Ruby => CommentTokens {
            line: Some("#"),
            block: None,
        },
        LegacyMode::Groovy => CommentTokens {
            line: Some("//"),
            block: Some(("/*", "*/")),
        },
        LegacyMode::PowerShell => CommentTokens {
            line: Some("#"),
            block: Some(("<#", "#>")),
        },
        _ => unreachable!(),
    };
    let data = LanguageData::default().with("commentTokens", comments);
    let brackets = &["(", "[", "{", "'", "\"", "'''", "\"\"\""][..];
    match mode {
        LegacyMode::Python | LegacyMode::Cython => data
            .with("autocomplete", python::AUTOCOMPLETE)
            .with("closeBrackets", brackets)
            .with("indentOnInput", python_indent_input as fn(&str) -> bool),
        LegacyMode::Ruby => data
            .with("autocomplete", ruby::KEYWORDS)
            .with("indentOnInput", ruby_indent_input as fn(&str) -> bool),
        LegacyMode::Groovy => data
            .with("closeBrackets", brackets)
            .with("indentOnInput", groovy_indent_input as fn(&str) -> bool),
        LegacyMode::PowerShell => data,
        _ => unreachable!(),
    }
}

fn python_indent_input(text: &str) -> bool {
    matches!(
        text.trim_start_matches(js_space),
        "}" | "]" | ")" | "else:" | "elif " | "except " | "finally:"
    )
}

fn ruby_indent_input(text: &str) -> bool {
    matches!(
        text.trim_start_matches(js_space),
        "end" | "rescue" | "elsif" | "else" | "}"
    )
}

fn groovy_indent_input(text: &str) -> bool {
    matches!(text.trim_start_matches(js_space), "{" | "}")
}

#[derive(Clone, Debug)]
pub(super) struct State(Scan);

#[derive(Clone, Debug)]
enum Scan {
    Python(python::State),
    Ruby(ruby::State),
    Groovy(groovy::State),
    PowerShell(powershell::State),
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self(match mode {
            LegacyMode::Python | LegacyMode::Cython => Scan::Python(python::State::new(mode)),
            LegacyMode::Ruby => Scan::Ruby(ruby::State::new(indent_unit)),
            LegacyMode::Groovy => Scan::Groovy(groovy::State::new(indent_unit)),
            LegacyMode::PowerShell => Scan::PowerShell(powershell::State::default()),
            _ => unreachable!(),
        })
    }

    pub(super) fn mode(&self) -> LegacyMode {
        match &self.0 {
            Scan::Python(state) => state.mode,
            Scan::Ruby(_) => LegacyMode::Ruby,
            Scan::Groovy(_) => LegacyMode::Groovy,
            Scan::PowerShell(_) => LegacyMode::PowerShell,
        }
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &mut self.0 {
            Scan::Python(state) => state.token(stream),
            Scan::Ruby(state) => state.token(stream),
            Scan::Groovy(state) => state.token(stream),
            Scan::PowerShell(state) => state.token(stream),
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match &self.0 {
            Scan::Python(state) => state.indent(text_after, unit),
            Scan::Ruby(state) => state.indent(text_after, unit),
            Scan::Groovy(state) => state.indent(text_after, unit),
            Scan::PowerShell(_) => None,
        }
    }
}

fn remaining<'line>(stream: &StringStream<'line>) -> &'line str {
    &stream.line()[stream.pos()..]
}

fn ascii_word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn word_end(text: &str) -> bool {
    !text.starts_with(ascii_word)
}

fn eat_str(stream: &mut StringStream<'_>, text: &str) -> bool {
    stream.match_str(text, true, false)
}
