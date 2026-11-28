use std::sync::LazyLock;

use regex::Regex;

use super::{
    LegacyMode, StringStream, ascii_word, eat_str, js_space, python_factory::LegacyPythonConfig,
    remaining,
};
use crate::legacy_modes::js_line_terminator;

pub(super) const AUTOCOMPLETE: &[&str] = &[
    "as",
    "assert",
    "break",
    "class",
    "continue",
    "def",
    "del",
    "elif",
    "else",
    "except",
    "finally",
    "for",
    "from",
    "global",
    "if",
    "import",
    "lambda",
    "pass",
    "raise",
    "return",
    "try",
    "while",
    "with",
    "yield",
    "in",
    "False",
    "True",
    "abs",
    "all",
    "any",
    "bin",
    "bool",
    "bytearray",
    "callable",
    "chr",
    "classmethod",
    "compile",
    "complex",
    "delattr",
    "dict",
    "dir",
    "divmod",
    "enumerate",
    "eval",
    "filter",
    "float",
    "format",
    "frozenset",
    "getattr",
    "globals",
    "hasattr",
    "hash",
    "help",
    "hex",
    "id",
    "input",
    "int",
    "isinstance",
    "issubclass",
    "iter",
    "len",
    "list",
    "locals",
    "map",
    "max",
    "memoryview",
    "min",
    "next",
    "object",
    "oct",
    "open",
    "ord",
    "pow",
    "property",
    "range",
    "repr",
    "reversed",
    "round",
    "set",
    "setattr",
    "slice",
    "sorted",
    "staticmethod",
    "str",
    "sum",
    "super",
    "tuple",
    "type",
    "vars",
    "zip",
    "__import__",
    "NotImplemented",
    "Ellipsis",
    "__debug__",
    "exec",
    "print",
];

const COMMON_KEYWORD_COUNT: usize = 27;
const COMMON_BUILTIN_END: usize = AUTOCOMPLETE.len() - 2;
const PY2_KEYWORDS: &[&str] = &["exec", "print"];
const PY2_BUILTINS: &[&str] = &[
    "apply",
    "basestring",
    "buffer",
    "cmp",
    "coerce",
    "execfile",
    "file",
    "intern",
    "long",
    "raw_input",
    "reduce",
    "reload",
    "unichr",
    "unicode",
    "xrange",
    "None",
];
const PY3_BUILTINS: &[&str] = &["ascii", "bytes", "exec", "print"];
const PY3_KEYWORDS: &[&str] = &[
    "nonlocal",
    "None",
    "aiter",
    "anext",
    "async",
    "await",
    "breakpoint",
    "match",
    "case",
];
const CYTHON_KEYWORDS: &[&str] = &[
    "by", "cdef", "cimport", "cpdef", "ctypedef", "enum", "except", "extern", "gil", "include",
    "nogil", "property", "public", "readonly", "struct", "union", "DEF", "IF", "ELIF", "ELSE",
];

static NUMBERS: LazyLock<[Regex; 7]> = LazyLock::new(|| {
    [
        r"(?i-u:^[0-9_]*\.[0-9]+(e[+-]?[0-9]+)?)",
        r"^[0-9_]+\.[0-9]*",
        r"^\.[0-9]+",
        r"(?i-u:^0x[0-9a-f_]+)",
        r"(?i-u:^0b[01_]+)",
        r"(?i-u:^0o[0-7_]+)",
        r"^[1-9][0-9_]*(e[+-]?[0-9_]+)?",
    ]
    .map(|pattern| Regex::new(pattern).unwrap())
});
static DEFAULT_CONFIG: LazyLock<LegacyPythonConfig> = LazyLock::new(LegacyPythonConfig::default);
static PY2_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i-u:^(?:[rubf]|ur|br)?(?:'{3}|"{3}|['"]))"#).unwrap());
static PY3_PREFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i-u:^(?:[rbuf]|br|rb|fr|rf)?(?:'{3}|"{3}|['"]))"#).unwrap());
static OPERATOR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[-+*/%&|^]=?|[<>=]+|//=?|\*\*=?|!=|[~!@]|\.\.\.)").unwrap());

#[derive(Clone, Debug)]
pub(super) struct State {
    pub(super) mode: LegacyMode,
    frames: Vec<Frame>,
    scopes: Vec<Scope>,
    indent: usize,
    last: Option<&'static str>,
    lambda: bool,
    dedent: bool,
    beginning: bool,
    error_token: bool,
}

#[derive(Clone, Copy, Debug)]
enum Frame {
    String { quote: &'static str, format: bool },
    Expression(usize),
}

#[derive(Clone, Copy, Debug)]
struct Scope {
    offset: usize,
    closing: Option<char>,
    align: Option<usize>,
}

impl State {
    pub(super) fn new(mode: LegacyMode) -> Self {
        Self {
            mode,
            frames: Vec::new(),
            scopes: vec![Scope {
                offset: 0,
                closing: None,
                align: None,
            }],
            indent: 0,
            last: None,
            lambda: false,
            dedent: false,
            beginning: false,
            error_token: false,
        }
    }

    fn top(&self) -> Scope {
        *self.scopes.last().unwrap()
    }

    fn push_scope(&mut self, stream: &StringStream<'_>) {
        self.scopes.push(Scope {
            offset: self.top().offset.saturating_add(stream.indent_unit()),
            closing: None,
            align: None,
        });
    }

    fn dedent_to(&mut self, offset: usize) -> bool {
        while self.scopes.len() > 1 && self.top().offset > offset {
            if self.top().closing.is_some() {
                return true;
            }
            self.scopes.pop();
        }
        self.top().offset != offset
    }

    fn base(
        &mut self,
        stream: &mut StringStream<'_>,
        config: &LegacyPythonConfig,
    ) -> Option<&'static str> {
        if stream.sol() && self.last != Some("\\") {
            let indent = stream.indentation();
            self.indent = indent;
            if self.top().closing.is_none() {
                let offset = self.top().offset;
                if stream.eat_space() {
                    if indent > offset {
                        self.push_scope(stream);
                    } else if indent < offset
                        && self.dedent_to(indent)
                        && stream.peek() != Some('#')
                    {
                        self.error_token = true;
                    }
                    return None;
                }
                let style = self.base_inner(stream, config, false);
                return if offset > 0 && self.dedent_to(indent) {
                    Some(add_error(style))
                } else {
                    style
                };
            }
        }
        self.base_inner(stream, config, false)
    }

    fn base_inner(
        &mut self,
        stream: &mut StringStream<'_>,
        config: &LegacyPythonConfig,
        format: bool,
    ) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        if !format && stream.peek() == Some('#') {
            stream.eat_while(|ch| !js_line_terminator(ch));
            return Some("comment");
        }
        if stream
            .peek()
            .is_some_and(|ch| ch.is_ascii_digit() || ch == '.')
            && number(stream)
        {
            return Some("number");
        }
        let (prefix, keywords, version_builtins) = if config.python3() {
            (&*PY3_PREFIX, PY3_KEYWORDS, PY3_BUILTINS)
        } else {
            (&*PY2_PREFIX, PY2_KEYWORDS, PY2_BUILTINS)
        };
        if let Some(matched) = stream.match_regex(prefix, true) {
            let opener = matched.get(0).unwrap().as_str();
            let quote = match opener
                .trim_start_matches(|ch: char| "rubf".contains(ch.to_ascii_lowercase()))
            {
                "'" => "'",
                "\"" => "\"",
                "'''" => "'''",
                _ => "\"\"\"",
            };
            let format = opener.contains(['f', 'F']);
            self.frames.push(Frame::String { quote, format });
            return self.string(stream, config, quote, format);
        }
        if [
            config.single_operators.as_ref(),
            config.double_operators.as_ref(),
            config.double_delimiters.as_ref(),
            config.triple_delimiters.as_ref(),
            Some(config.operators.as_ref().unwrap_or(&OPERATOR)),
        ]
        .into_iter()
        .flatten()
        .any(|pattern| matches_nonempty(stream, pattern, true))
        {
            return Some("operator");
        }
        let punctuation = match config
            .delimiters
            .as_ref()
            .or(config.single_delimiters.as_ref())
        {
            Some(pattern) => matches_nonempty(stream, pattern, true),
            None => stream.eat_if(|ch| "()[]{}@,:`=;.\\".contains(ch)).is_some(),
        };
        if punctuation {
            return Some("punctuation");
        }
        if self.last == Some(".") && identifier(stream, config, true) {
            return Some("property");
        }
        if eat_word(stream, &AUTOCOMPLETE[..COMMON_KEYWORD_COUNT])
            || (self.mode == LegacyMode::Cython && eat_word(stream, CYTHON_KEYWORDS))
            || eat_word(stream, &config.extra_keywords)
            || eat_word(stream, keywords)
            || eat_word(stream, &["and", "or", "not", "is"])
        {
            return Some("keyword");
        }
        if eat_word(
            stream,
            &AUTOCOMPLETE[COMMON_KEYWORD_COUNT..COMMON_BUILTIN_END],
        ) || eat_word(stream, &config.extra_builtins)
            || eat_word(stream, version_builtins)
        {
            return Some("builtin");
        }
        if eat_word(stream, &["self", "cls"]) {
            return Some("self");
        }
        if identifier(stream, config, true) {
            return Some(if matches!(self.last, Some("def" | "class")) {
                "def"
            } else {
                "variable"
            });
        }
        stream.next();
        (!format).then_some("error")
    }

    fn string(
        &mut self,
        stream: &mut StringStream<'_>,
        config: &LegacyPythonConfig,
        quote: &'static str,
        format: bool,
    ) -> Option<&'static str> {
        let single_line = quote.len() == 1;
        while !stream.eol() {
            stream.eat_while(|ch| {
                !(matches!(ch, '\'' | '"' | '\\') || format && matches!(ch, '{' | '}'))
            });
            if stream.eat('\\').is_some() {
                stream.next();
                if single_line && stream.eol() {
                    return Some("string");
                }
            } else if eat_str(stream, quote) {
                self.frames.pop();
                return Some("string");
            } else if format && (eat_str(stream, "{{") || eat_str(stream, "}}")) {
                return Some("string");
            } else if format && stream.peek() == Some('{') {
                self.frames.push(Frame::Expression(0));
                return if stream.current().is_empty() {
                    self.expression(stream, config, 0)
                } else {
                    Some("string")
                };
            } else if format && stream.eat('}').is_some() {
                return Some("error");
            } else {
                stream.eat_if(|ch| matches!(ch, '\'' | '"'));
            }
        }
        if single_line {
            if config.single_line_string_errors {
                return Some("error");
            }
            self.frames.pop();
        }
        Some("string")
    }

    fn expression(
        &mut self,
        stream: &mut StringStream<'_>,
        config: &LegacyPythonConfig,
        depth: usize,
    ) -> Option<&'static str> {
        let style = self.base_inner(stream, config, true);
        if style == Some("punctuation") {
            match stream.current() {
                "{" => *self.frames.last_mut().unwrap() = Frame::Expression(depth + 1),
                "}" if depth <= 1 => {
                    self.frames.pop();
                }
                "}" => *self.frames.last_mut().unwrap() = Frame::Expression(depth - 1),
                _ => (),
            }
        }
        style
    }

    fn lex(
        &mut self,
        stream: &mut StringStream<'_>,
        config: &LegacyPythonConfig,
    ) -> Option<&'static str> {
        if stream.sol() {
            self.beginning = true;
            self.dedent = false;
        }
        let mut style = match self.frames.last().copied() {
            None => self.base(stream, config),
            Some(Frame::String { quote, format }) => self.string(stream, config, quote, format),
            Some(Frame::Expression(depth)) => self.expression(stream, config, depth),
        };
        let current = stream.current();
        if self.beginning && current == "@" {
            return Some(if identifier(stream, config, false) {
                "meta"
            } else if config.python3() {
                "operator"
            } else {
                "error"
            });
        }
        if !current.chars().all(js_space) {
            self.beginning = false;
        }
        if matches!(style, Some("variable" | "builtin")) && self.last == Some("meta") {
            style = Some("meta");
        }
        match current {
            "pass" | "return" => self.dedent = true,
            "lambda" => self.lambda = true,
            ":" if !self.lambda
                && self.top().closing.is_none()
                && comment_or_end(remaining(stream).trim_start_matches(js_space)) =>
            {
                self.push_scope(stream);
            }
            _ => (),
        }
        if !style.is_some_and(|s| s.contains("string") || s.contains("comment")) {
            let closing = match current {
                "[" => Some(']'),
                "(" => Some(')'),
                "{" => Some('}'),
                _ => None,
            };
            let hanging = config.hanging_indent(stream.indent_unit());
            if closing.is_some() {
                let next =
                    remaining(stream).trim_start_matches(|ch| js_space(ch) || "[({".contains(ch));
                self.scopes.push(Scope {
                    offset: self.indent.saturating_add(hanging),
                    closing,
                    align: (!comment_or_end(next)).then(|| stream.column().saturating_add(1)),
                });
            } else if matches!(current, "]" | ")" | "}") {
                if self.top().closing == current.chars().next() {
                    self.indent = self.scopes.pop().unwrap().offset.saturating_sub(hanging);
                } else {
                    return Some("error");
                }
            }
        }
        if self.dedent && stream.eol() && self.top().closing.is_none() && self.scopes.len() > 1 {
            self.scopes.pop();
        }
        style
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        self.token_with_config(stream, &DEFAULT_CONFIG)
    }

    pub(super) fn token_with_config(
        &mut self,
        stream: &mut StringStream<'_>,
        config: &LegacyPythonConfig,
    ) -> Option<&'static str> {
        let add_err = std::mem::take(&mut self.error_token);
        let style = self.lex(stream, config);
        if let Some(style) = style.filter(|&style| style != "comment") {
            self.last = match (style, stream.current()) {
                ("def", _) | ("keyword" | "punctuation", "def") => Some("def"),
                ("keyword" | "punctuation", "class") => Some("class"),
                ("keyword" | "punctuation", ".") => Some("."),
                ("keyword" | "punctuation", "\\") => Some("\\"),
                ("meta", _) => Some("meta"),
                _ => None,
            };
        }
        if stream.eol() {
            self.lambda = false;
        }
        if add_err {
            Some("error")
        } else {
            style.filter(|&style| style != "punctuation")
        }
    }

    pub(super) fn indent(&self, text_after: &str, unit: usize) -> Option<usize> {
        match self.frames.last() {
            Some(Frame::String { .. }) => return None,
            Some(Frame::Expression(_)) => return Some(0),
            None => (),
        }
        let scope = self.top();
        let closing = if let Some(close) = scope.closing {
            text_after.starts_with(close)
        } else {
            !self.dedent
                && ["else:", "elif ", "except ", "finally:"]
                    .iter()
                    .any(|prefix| text_after.starts_with(prefix))
        };
        Some(if let Some(align) = scope.align {
            align.saturating_sub(usize::from(closing))
        } else {
            scope.offset.saturating_sub(if closing { unit } else { 0 })
        })
    }
}

fn comment_or_end(text: &str) -> bool {
    text.is_empty() || text.starts_with('#')
}

fn add_error(style: Option<&'static str>) -> &'static str {
    match style {
        None => "null error",
        Some("builtin") => "builtin error",
        Some("comment") => "comment error",
        Some("def") => "def error",
        Some("error") => "error error",
        Some("keyword") => "keyword error",
        Some("number") => "number error",
        Some("operator") => "operator error",
        Some("property") => "property error",
        Some("punctuation") => "punctuation error",
        Some("self") => "self error",
        Some("string") => "string error",
        Some("variable") => "variable error",
        _ => unreachable!(),
    }
}

fn number(stream: &mut StringStream<'_>) -> bool {
    let mut float = false;
    for regex in &NUMBERS[..3] {
        float |= stream.match_regex(regex, true).is_some();
    }
    if float {
        stream.eat_if(|ch| ch.eq_ignore_ascii_case(&'j'));
        return true;
    }
    let mut integer = false;
    for (i, regex) in NUMBERS[3..].iter().enumerate() {
        if stream.match_regex(regex, true).is_some() {
            integer = true;
            if i == 3 {
                stream.eat_if(|ch| ch.eq_ignore_ascii_case(&'j'));
            }
        }
    }
    if remaining(stream).strip_prefix('0').is_some_and(|next| {
        !next.starts_with(|ch: char| ch.is_ascii_digit() || ch.eq_ignore_ascii_case(&'x'))
    }) {
        stream.next();
        integer = true;
    }
    if integer {
        stream.eat_if(|ch| ch.eq_ignore_ascii_case(&'l'));
    }
    integer
}

fn eat_word(stream: &mut StringStream<'_>, words: &[impl AsRef<str>]) -> bool {
    words.iter().any(|word| {
        let word = word.as_ref();
        !word.is_empty()
            && remaining(stream)
                .strip_prefix(word)
                .is_some_and(|after| word.ends_with(ascii_word) != after.starts_with(ascii_word))
            && eat_str(stream, word)
    })
}

fn matches_nonempty(stream: &mut StringStream<'_>, pattern: &Regex, consume: bool) -> bool {
    stream
        .match_regex(pattern, consume)
        .is_some_and(|matched| !matched[0].is_empty())
}

fn identifier(stream: &mut StringStream<'_>, config: &LegacyPythonConfig, consume: bool) -> bool {
    if let Some(pattern) = &config.identifiers {
        return matches_nonempty(stream, pattern, consume);
    }
    let unicode = |ch| config.python3() && ch >= '\u{00A1}';
    if !stream
        .peek()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_' || unicode(ch))
    {
        return false;
    }
    if consume {
        stream.next();
        stream.eat_while(|ch| ascii_word(ch) || unicode(ch));
    }
    true
}
