//! Concrete streams and the native `clike` factory from `@codemirror/legacy-modes` 6.5.5.
//! See `docs/licenses/codemirror-legacy-modes.txt` for the MIT notice.
//!
//! These are lexical modes, not syntax parsers. Byte ranges and columns follow
//! the Rust scalar stream. Ceylon keeps pending interpolation in document state
//! and advances when its number pattern would otherwise match no characters.

use std::sync::{LazyLock, OnceLock};

use regex::Regex;

use super::{LegacyMode, js_line_terminator, js_space};
use crate::{commands::CommentTokens, language::LanguageData, stream_parser::StringStream};

mod factory;
mod scanner;
mod spec;

pub use factory::{
    LegacyClike, LegacyClikeConfig, LegacyClikeHook, LegacyClikeHookResult, LegacyClikeState,
    LegacyClikeTokenHook, LegacyClikeWords, clike,
};
use scanner::{Scan, base};
use spec::spec;

static DEFAULT_CONFIG: LegacyClikeConfig = LegacyClikeConfig::DEFAULT;

pub(super) fn name(mode: LegacyMode) -> &'static str {
    spec(mode).name
}

pub(super) fn data(mode: LegacyMode) -> LanguageData {
    static COMPLETIONS: [OnceLock<Vec<&str>>; LegacyMode::ALL.len()] =
        [const { OnceLock::new() }; LegacyMode::ALL.len()];
    let spec = spec(mode);
    let words = COMPLETIONS[mode as usize].get_or_init(|| {
        [spec.keywords, spec.types, spec.builtins, spec.atoms]
            .into_iter()
            .filter(|words| !words.is_empty())
            .flat_map(|words| words.split(' '))
            .collect()
    });
    let indent_switch = !matches!(
        mode,
        LegacyMode::Scala | LegacyMode::Shader | LegacyMode::Ceylon
    );
    let data = common_data()
        .with("indentOnInput", indent_trigger(indent_switch))
        .with("autocomplete", words.as_slice());
    if matches!(
        mode,
        LegacyMode::Scala | LegacyMode::Kotlin | LegacyMode::Ceylon
    ) {
        data.with("closeBrackets", &["(", "[", "{", "'", "\"", "\"\"\""][..])
    } else {
        data
    }
}

fn common_data() -> LanguageData {
    LanguageData::default().with(
        "commentTokens",
        CommentTokens {
            line: Some("//"),
            block: Some(("/*", "*/")),
        },
    )
}

fn indent_trigger(indent_switch: bool) -> fn(&str) -> bool {
    if indent_switch {
        |line| {
            let line = line.trim_start_matches(js_space);
            matches!(line, "{" | "{}" | "}" | "default:")
                || line
                    .strip_prefix("case ")
                    .is_some_and(|tail| tail.ends_with(':') && !tail.contains(js_line_terminator))
        }
    } else {
        |line| matches!(line.trim_start_matches(js_space), "{" | "}")
    }
}

#[derive(Clone, Debug)]
pub(super) struct State {
    mode: Option<LegacyMode>,
    scan: Option<Scan>,
    context: Vec<Context>,
    top_indent_unit: usize,
    indented: usize,
    start_of_line: bool,
    prev: Option<Previous>,
    type_at_eol: bool,
    ceylon_string: Option<bool>,
    dart_strings: Vec<Scan>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Previous {
    Style(&'static str),
    Punctuation(char),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Top,
    Statement,
    Delimiter(char),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Info {
    Switch,
    Namespace,
    Other,
}

#[derive(Clone, Copy, Debug)]
struct Context {
    kind: Kind,
    info: Info,
    skip_indent: bool,
    align: Option<bool>,
    column: usize,
    indented: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Punctuation {
    Char(char),
    NewStatement,
}

#[derive(Default)]
struct TokenControl {
    punc: Option<Punctuation>,
    def_keyword: bool,
    reset_align: bool,
}

impl State {
    pub(super) fn new(mode: LegacyMode, indent_unit: usize) -> Self {
        Self::for_mode(Some(mode), indent_unit)
    }

    fn for_mode(mode: Option<LegacyMode>, indent_unit: usize) -> Self {
        Self {
            mode,
            scan: None,
            context: vec![Context {
                kind: Kind::Top,
                info: Info::Other,
                skip_indent: false,
                align: Some(false),
                column: 0,
                indented: 0,
            }],
            top_indent_unit: indent_unit,
            indented: 0,
            start_of_line: true,
            prev: None,
            type_at_eol: false,
            ceylon_string: None,
            dart_strings: Vec::new(),
        }
    }

    pub(super) fn mode(&self) -> LegacyMode {
        self.mode.expect("bundled mode")
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        self.token_with(stream, &DEFAULT_CONFIG)
    }

    fn token_with(
        &mut self,
        stream: &mut StringStream<'_>,
        config: &LegacyClikeConfig,
    ) -> Option<&'static str> {
        let index = self.context.len() - 1;
        if stream.sol() {
            self.context[index].align.get_or_insert(false);
            self.indented = stream.indentation();
            self.start_of_line = true;
        }
        if stream.eat_space() {
            self.maybe_eol(stream, config);
            return None;
        }
        let mut control = TokenControl::default();
        let mut style = if let Some(scan) = self.scan.take() {
            scan.read(self, stream)
        } else {
            base(self, stream, &mut control, config)
        };
        if matches!(style, Some("comment" | "meta")) {
            return style;
        }
        if control.reset_align {
            self.context[index].align = None;
        } else {
            self.context[index].align.get_or_insert(true);
        }

        let kind = self.context[index].kind;
        let punc = control.punc;
        match punc {
            Some(Punctuation::Char(';' | ':')) => self.pop_statements(),
            Some(Punctuation::Char(',')) if comma_ends_statement(stream) => self.pop_statements(),
            Some(Punctuation::Char('{')) => self.push(stream, Kind::Delimiter('}'), config),
            Some(Punctuation::Char('[')) => self.push(stream, Kind::Delimiter(']'), config),
            Some(Punctuation::Char('(')) => self.push(stream, Kind::Delimiter(')'), config),
            Some(Punctuation::Char('}')) => {
                self.pop_statements();
                if self.current_context().kind == Kind::Delimiter('}') {
                    self.pop();
                }
                self.pop_statements();
            }
            Some(Punctuation::Char(ch)) if kind == Kind::Delimiter(ch) => self.pop(),
            _ if config.indent_statements
                && !matches!(self.mode, Some(LegacyMode::Scala | LegacyMode::Kotlin))
                && (matches!(kind, Kind::Top | Kind::Delimiter('}'))
                    || (kind == Kind::Statement && punc == Some(Punctuation::NewStatement))) =>
            {
                self.push(stream, Kind::Statement, config);
            }
            _ => {}
        }

        if style == Some("variable")
            && (self.prev == Some(Previous::Style("def"))
                || (self.type_first_definitions(config)
                    && self.type_before(stream, stream.start())
                    && self.is_top_scope()
                    && remaining(stream)
                        .trim_start_matches(js_space)
                        .starts_with('(')))
        {
            style = Some("def");
        }
        style = self.style_hook(stream, style);
        if let Some(hook) = config.token_hook {
            style = hook(stream, style).style(style);
        }
        if style == Some("def") && (!config.style_defs || self.mode == Some(LegacyMode::Ceylon)) {
            style = Some("variable");
        }
        self.start_of_line = false;
        self.prev = if control.def_keyword {
            Some(Previous::Style("def"))
        } else if let Some(style) = style {
            Some(Previous::Style(style))
        } else if let Some(Punctuation::Char(ch)) = punc {
            Some(Previous::Punctuation(ch))
        } else {
            None
        };
        self.maybe_eol(stream, config);
        style
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        self.indent_with(text_after, unit, &DEFAULT_CONFIG)
    }

    fn indent_with(
        &self,
        text_after: &str,
        unit: usize,
        config: &LegacyClikeConfig,
    ) -> Option<usize> {
        if self.scan.is_some() || (self.type_at_eol && self.is_top_scope()) {
            return None;
        }
        let first = text_after.chars().next();
        let mut index = self.context.len() - 1;
        let closing = first.is_some_and(|ch| self.context[index].kind == Kind::Delimiter(ch));
        if self.context[index].kind == Kind::Statement && first == Some('}') {
            index -= 1;
        }
        while self.context[index].skip_indent {
            index -= 1;
        }
        if config.allman_indentation && matches!(first, Some('{' | '(')) {
            while !matches!(self.context[index].kind, Kind::Top | Kind::Delimiter('}')) {
                index -= 1;
            }
            return Some(self.context[index].indented);
        }
        let context = self.context[index];
        if self.mode == Some(LegacyMode::Kotlin) {
            let after_close = matches!(self.prev, Some(Previous::Punctuation('}' | ')')));
            if after_close && text_after.is_empty() {
                return Some(self.indented);
            }
            if (self.prev == Some(Previous::Style("operator"))
                && text_after != "}"
                && self.current_context().kind != Kind::Delimiter('}'))
                || ((after_close || self.prev == Some(Previous::Style("variable")))
                    && first == Some('.'))
            {
                return Some(self.add_indent(context, unit.saturating_mul(2)));
            }
            if context.align == Some(true) && context.kind == Kind::Delimiter('}') {
                return Some(
                    context
                        .indented
                        .saturating_add(if closing { 0 } else { unit }),
                );
            }
        }
        if context.kind == Kind::Top {
            return Some(self.add_indent(context, unit));
        }
        let statement_unit = if config.statement_indent_unit == 0 {
            unit
        } else {
            config.statement_indent_unit
        };
        let width = if context.kind == Kind::Statement {
            context.indented.saturating_add(if first == Some('{') {
                0
            } else {
                statement_unit
            })
        } else if context.align == Some(true)
            && (!config.dont_align_calls || context.kind != Kind::Delimiter(')'))
        {
            context.column.saturating_add(usize::from(!closing))
        } else if context.kind == Kind::Delimiter(')') && !closing {
            context.indented.saturating_add(statement_unit)
        } else if !closing {
            let switch_block = self.context[index - 1].info == Info::Switch;
            let extra_switch = switch_block && !starts_case_or_default(text_after);
            context
                .indented
                .saturating_add(unit.saturating_mul(1 + usize::from(extra_switch)))
        } else {
            context.indented
        };
        Some(width)
    }

    fn add_indent(&self, context: Context, width: usize) -> usize {
        if context.kind == Kind::Top {
            // The upstream root starts one indent unit below column zero.
            width.saturating_sub(self.top_indent_unit)
        } else {
            context.indented.saturating_add(width)
        }
    }

    fn current_context(&self) -> &Context {
        self.context.last().unwrap()
    }

    fn push(&mut self, stream: &StringStream<'_>, kind: Kind, config: &LegacyClikeConfig) {
        let parent = self.current_context();
        let mut skip_indent = false;
        let (indented, info) = if kind == Kind::Statement {
            let word = stream.current();
            skip_indent = (self.is_cpp() && word == "template")
                || (self.is_objective()
                    && word.starts_with('@')
                    && !word.contains(js_line_terminator))
                || config
                    .dont_indent_statements
                    .as_ref()
                    .is_some_and(|pattern| pattern.is_match(word));
            let info = match word {
                "switch" => Info::Switch,
                "namespace" => Info::Namespace,
                _ => Info::Other,
            };
            (self.indented, info)
        } else if parent.kind == Kind::Statement {
            (parent.indented, Info::Other)
        } else {
            (self.indented, Info::Other)
        };
        self.context.push(Context {
            kind,
            info,
            skip_indent,
            align: None,
            column: stream.column(),
            indented,
        });
    }

    fn pop(&mut self) {
        let context = self.context.pop().unwrap();
        if let Kind::Delimiter(_) = context.kind {
            self.indented = context.indented;
        }
    }

    fn pop_statements(&mut self) {
        while self.current_context().kind == Kind::Statement {
            self.pop();
        }
    }

    fn is_top_scope(&self) -> bool {
        !self
            .context
            .windows(2)
            .any(|pair| pair[1].kind == Kind::Delimiter('}') && pair[0].info != Info::Namespace)
    }

    fn type_before(&self, stream: &StringStream<'_>, pos: usize) -> bool {
        let prefix = &stream.line()[..pos];
        let mut tail = prefix.trim_end_matches(js_space).chars().rev();
        let punctuation = match tail.next() {
            Some('*' | ']') => tail.next().is_some_and(|ch| !js_space(ch)),
            Some('>') => {
                tail.next().is_some_and(|ch| !matches!(ch, '-' | ' '))
                    && tail.next().is_some_and(|ch| !js_space(ch))
            }
            _ => false,
        };
        matches!(self.prev, Some(Previous::Style("variable" | "type")))
            || punctuation
            || prefix.ends_with('*')
            || (self.type_at_eol && stream.column() == stream.indentation())
    }

    fn maybe_eol(&mut self, stream: &StringStream<'_>, config: &LegacyClikeConfig) {
        if self.type_first_definitions(config) && stream.eol() && self.is_top_scope() {
            self.type_at_eol = self.type_before(stream, stream.pos());
        }
    }

    fn type_first_definitions(&self, config: &LegacyClikeConfig) -> bool {
        config.type_first_definitions
            || matches!(
                self.mode,
                Some(
                    LegacyMode::C
                        | LegacyMode::Cpp
                        | LegacyMode::ObjectiveC
                        | LegacyMode::ObjectiveCpp
                        | LegacyMode::CSharp
                        | LegacyMode::Java
                        | LegacyMode::Squirrel
                        | LegacyMode::Ceylon
                )
            )
    }

    fn is_cpp(&self) -> bool {
        matches!(self.mode, Some(LegacyMode::Cpp | LegacyMode::ObjectiveCpp))
    }

    fn is_objective(&self) -> bool {
        matches!(
            self.mode,
            Some(LegacyMode::ObjectiveC | LegacyMode::ObjectiveCpp)
        )
    }

    fn style_hook(
        &self,
        stream: &StringStream<'_>,
        style: Option<&'static str>,
    ) -> Option<&'static str> {
        if self.mode == Some(LegacyMode::Ceylon)
            && self.prev == Some(Previous::Punctuation('.'))
            && matches!(style, Some("variable" | "type"))
        {
            return Some("variableName.special");
        }
        if style == Some("variable") {
            if self.mode == Some(LegacyMode::Dart) {
                let word = stream.current().trim_start_matches(['_', '$']);
                if word.starts_with(|ch: char| ch.is_ascii_uppercase())
                    && word.chars().all(identifier_ascii)
                {
                    return Some("type");
                }
            } else if self.is_cpp()
                && stream.peek() == Some('(')
                && matches!(self.prev, None | Some(Previous::Punctuation(';' | '}')))
            {
                static CONSTRUCTOR: LazyLock<Regex> =
                    LazyLock::new(|| Regex::new(r"([a-zA-Z0-9_]+)::~?([a-zA-Z0-9_]+)$").unwrap());
                if CONSTRUCTOR
                    .captures(stream.current())
                    .is_some_and(|m| m[1] == m[2])
                {
                    return Some("def");
                }
            }
        }
        style
    }
}

fn remaining<'line>(stream: &StringStream<'line>) -> &'line str {
    &stream.line()[stream.pos()..]
}

fn word_ascii(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn identifier_ascii(ch: char) -> bool {
    word_ascii(ch) || ch == '$'
}

fn starts_case_or_default(line: &str) -> bool {
    ["case", "default"].into_iter().any(|prefix| {
        line.strip_prefix(prefix)
            .is_some_and(|tail| !tail.starts_with(word_ascii))
    })
}

fn comma_ends_statement(stream: &StringStream<'_>) -> bool {
    let tail = remaining(stream).trim_start_matches(js_space);
    tail.is_empty()
        || tail
            .strip_prefix("//")
            .is_some_and(|tail| !tail.contains(js_line_terminator))
}
