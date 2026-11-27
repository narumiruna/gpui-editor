//! Shared C-like token scanners and hooks for the bundled dialects.

use super::{
    Kind, LegacyClikeConfig, LegacyClikeHookResult, LegacyClikeWords, LegacyMode, Previous,
    Punctuation, State, StringStream, TokenControl, identifier_ascii, js_line_terminator, js_space,
    remaining,
    spec::{Spec, spec},
    word_ascii,
};
use regex::Regex;
use std::sync::LazyLock;

#[derive(Clone, Debug)]
pub(super) enum Scan {
    Quoted {
        quote: char,
        multi_line: bool,
    },
    Verbatim,
    Triple,
    RawCpp(String),
    Comment,
    NestedComment(usize),
    Directive,
    Kotlin(bool),
    Ceylon(bool),
    DartString {
        quote: char,
        triple: bool,
        raw: bool,
    },
    DartInterpolation,
    DartIdentifier,
}

impl Scan {
    pub(super) fn read(
        self,
        state: &mut State,
        stream: &mut StringStream<'_>,
    ) -> Option<&'static str> {
        match self {
            Self::Directive => directive(state, stream),
            Self::Comment => {
                let mut star = false;
                for ch in stream {
                    if ch == '/' && star {
                        return Some("comment");
                    }
                    star = ch == '*';
                }
                state.scan = Some(self);
                Some("comment")
            }
            Self::NestedComment(mut depth) => {
                state.scan = Some(Self::NestedComment(depth));
                while let Some(ch) = stream.next() {
                    if ch == '*' && stream.eat('/').is_some() {
                        if depth == 1 {
                            state.scan = None;
                            break;
                        }
                        depth -= 1;
                        state.scan = Some(Self::NestedComment(depth));
                    } else if ch == '/' && stream.eat('*').is_some() {
                        depth += 1;
                        state.scan = Some(Self::NestedComment(depth));
                    }
                }
                Some("comment")
            }
            Self::Quoted { quote, multi_line } => {
                let mut escaped = false;
                for ch in stream {
                    if ch == quote && !escaped {
                        return Some("string");
                    }
                    escaped = !escaped && ch == '\\';
                }
                if escaped || multi_line {
                    state.scan = Some(self);
                }
                Some("string")
            }
            Self::Verbatim => {
                while let Some(ch) = stream.next() {
                    if ch == '"' && stream.eat('"').is_none() {
                        return Some("string");
                    }
                }
                state.scan = Some(self);
                Some("string")
            }
            Self::Triple => {
                let mut escaped = false;
                while !stream.eol() {
                    if !escaped && stream.match_str("\"\"\"", true, false) {
                        return Some("string");
                    }
                    escaped = stream.next() == Some('\\') && !escaped;
                }
                state.scan = Some(self);
                Some("string")
            }
            Self::RawCpp(ref delimiter) => {
                if let Some(index) = remaining(stream)
                    .split(js_line_terminator)
                    .next()
                    .unwrap_or_default()
                    .match_indices(')')
                    .find_map(|(index, _)| {
                        remaining(stream)[index + 1..]
                            .strip_prefix(delimiter)
                            .filter(|tail| tail.starts_with('"'))
                            .map(|_| index)
                    })
                {
                    stream
                        .set_pos(stream.pos() + index + delimiter.len() + 2)
                        .unwrap();
                } else {
                    stream.skip_to_end();
                    state.scan = Some(self);
                }
                Some("string")
            }
            Self::Kotlin(triple) => {
                let mut escaped = false;
                let ending = if triple { "\"\"\"" } else { "\"" };
                while !stream.eol() {
                    if !escaped && stream.match_str(ending, true, false) {
                        return Some("string");
                    }
                    let next = stream.next();
                    if !escaped && next == Some('$') && stream.eat('{').is_some() {
                        stream.skip_to("}");
                    }
                    escaped = !escaped && next == Some('\\') && !triple;
                }
                if triple {
                    state.scan = Some(self);
                }
                Some("string")
            }
            Self::Ceylon(triple) => {
                let mut escaped = false;
                while !stream.eol() {
                    if !escaped {
                        if stream.eat('"').is_some()
                            && (!triple || stream.match_str("\"\"", true, false))
                        {
                            return Some("string");
                        }
                        if stream.match_str("``", true, false) {
                            state.ceylon_string = Some(triple);
                            return Some("string");
                        }
                    }
                    let next = stream.next();
                    escaped = !triple && !escaped && next == Some('\\');
                }
                state.scan = Some(self);
                Some("string")
            }
            Self::DartString { quote, triple, raw } => {
                let mut escaped = false;
                let closing = if quote == '\'' { "''" } else { "\"\"" };
                while !stream.eol() {
                    if !raw && !escaped && stream.peek() == Some('$') {
                        state.dart_strings.push(self);
                        state.scan = Some(Self::DartInterpolation);
                        return Some("string");
                    }
                    let next = stream.next();
                    if next == Some(quote)
                        && !escaped
                        && (!triple || stream.match_str(closing, true, false))
                    {
                        return Some("string");
                    }
                    escaped = !raw && !escaped && next == Some('\\');
                }
                state.scan = Some(self);
                Some("string")
            }
            Self::DartInterpolation => {
                stream.eat('$');
                if stream.eat('{').is_none() {
                    state.scan = Some(Self::DartIdentifier);
                }
                None
            }
            Self::DartIdentifier => {
                stream.eat_while(word_ascii);
                state.scan = state.dart_strings.pop();
                Some("variable")
            }
        }
    }
}

pub(super) fn base(
    state: &mut State,
    stream: &mut StringStream<'_>,
    control: &mut TokenControl,
    config: &LegacyClikeConfig,
) -> Option<&'static str> {
    let ch = stream.next()?;
    if let Some(mode) = state.mode {
        if let Some(style) = hook(mode, state, stream, control, ch) {
            return style;
        }
    } else if let Some((_, hook)) = config.hooks.iter().find(|(leading, _)| *leading == ch) {
        let after_leading = stream.pos();
        let result = hook(stream);
        if result == LegacyClikeHookResult::Continue || stream.pos() < after_leading {
            stream.set_pos(after_leading).unwrap();
        }
        if result != LegacyClikeHookResult::Continue {
            return result.style(None);
        }
    }
    if matches!(ch, '"' | '\'') {
        let multi_line = config.multi_line_strings
            || matches!(
                state.mode,
                Some(LegacyMode::Scala | LegacyMode::Kotlin | LegacyMode::Ceylon)
            );
        return Scan::Quoted {
            quote: ch,
            multi_line,
        }
        .read(state, stream);
    }
    let ceylon = state.mode == Some(LegacyMode::Ceylon);
    let number_start = config.number_start.as_ref().map_or_else(
        || {
            ch.is_ascii_digit()
                || if ceylon {
                    matches!(ch, '#' | '$')
                } else {
                    ch == '.'
                }
        },
        |pattern| char_matches(pattern, ch),
    );
    if number_start {
        let after_leading = stream.pos();
        stream.set_pos(stream.start()).unwrap();
        stream.match_regex(
            config.number.as_ref().unwrap_or_else(|| number(state.mode)),
            true,
        );
        if stream.pos() > stream.start() {
            return Some("number");
        }
        // An empty match (for example on a Ceylon # or $) must not stall.
        stream.set_pos(after_leading).unwrap();
    }
    let punctuation = config.is_punctuation_char.as_ref().map_or_else(
        || {
            matches!(
                ch,
                '[' | ']' | '{' | '}' | '(' | ')' | ',' | ';' | ':' | '.'
            ) || (ceylon && ch == '`')
        },
        |pattern| char_matches(pattern, ch),
    );
    if punctuation {
        control.punc = Some(Punctuation::Char(ch));
        return None;
    }
    if ch == '/' {
        if stream.eat('*').is_some() {
            return Scan::Comment.read(state, stream);
        }
        if stream.eat('/').is_some() {
            stream.skip_to_end();
            return Some("comment");
        }
    }
    let is_operator = |ch| {
        config.is_operator_char.as_ref().map_or_else(
            || operator(state.mode, ch),
            |pattern| char_matches(pattern, ch),
        )
    };
    if is_operator(ch) {
        while !remaining(stream).starts_with("//")
            && !remaining(stream).starts_with("/*")
            && stream.eat_if(is_operator).is_some()
        {}
        return Some("operator");
    }
    let id = |ch| {
        config.is_identifier_char.as_ref().map_or_else(
            || identifier(ch) || (state.mode == Some(LegacyMode::Cpp) && ch == '~'),
            |pattern| char_matches(pattern, ch),
        )
    };
    stream.eat_while(id);
    let separator = if state.is_cpp() {
        Some("::")
    } else {
        config
            .namespace_separator
            .as_deref()
            .filter(|separator| !separator.is_empty())
    };
    if let Some(separator) = separator {
        while stream.match_str(separator, true, false) {
            stream.eat_while(id);
        }
    }

    let word = stream.current();
    let spec = state.mode.map(spec);
    let has_word = |table: fn(&Spec) -> &str, custom: &LegacyClikeWords| {
        spec.map_or_else(|| custom.contains(word), |spec| contains(table(spec), word))
    };
    if has_word(|s| s.keywords, &config.keywords) {
        if has_word(|s| s.block_keywords, &config.block_keywords) {
            control.punc = Some(Punctuation::NewStatement);
        }
        control.def_keyword = has_word(|s| s.def_keywords, &config.def_keywords);
        Some("keyword")
    } else if has_word(|s| s.types, &config.types)
        || state.mode.is_some_and(|mode| dynamic_type(mode, word))
    {
        Some("type")
    } else if has_word(|s| s.builtins, &config.builtin)
        || config.is_reserved_identifier.is_some_and(|test| test(word))
        || (state.mode.is_some_and(uses_pointer_hook) && reserved_c_identifier(word))
    {
        if has_word(|s| s.block_keywords, &config.block_keywords) {
            control.punc = Some(Punctuation::NewStatement);
        }
        Some("builtin")
    } else if has_word(|s| s.atoms, &config.atoms) {
        Some("atom")
    } else {
        Some("variable")
    }
}

fn char_matches(pattern: &Regex, ch: char) -> bool {
    pattern.is_match(ch.encode_utf8(&mut [0; 4]))
}

fn hook(
    mode: LegacyMode,
    state: &mut State,
    stream: &mut StringStream<'_>,
    control: &mut TokenControl,
    ch: char,
) -> Option<Option<&'static str>> {
    match ch {
        '#' if matches!(
            mode,
            LegacyMode::C
                | LegacyMode::Cpp
                | LegacyMode::ObjectiveC
                | LegacyMode::ObjectiveCpp
                | LegacyMode::NesC
                | LegacyMode::Squirrel
                | LegacyMode::Shader
        ) && state.start_of_line =>
        {
            Some(directive(state, stream))
        }
        '*' if uses_pointer_hook(mode) && state.prev == Some(Previous::Style("type")) => {
            Some(Some("type"))
        }
        '*' if mode == LegacyMode::Kotlin => {
            Some(Some(if state.prev == Some(Previous::Punctuation('.')) {
                "variable"
            } else {
                "operator"
            }))
        }
        '0'..='9' if state.is_cpp() => {
            stream.eat_while(|ch| word_ascii(ch) || matches!(ch, '.' | '\''));
            Some(Some("number"))
        }
        'u' | 'U' | 'L' | 'R' if state.is_cpp() => cpp_string(state, stream).map(Some),
        '@' if mode == LegacyMode::CSharp && stream.eat('"').is_some() => {
            Some(Scan::Verbatim.read(state, stream))
        }
        '@' if mode == LegacyMode::Java && remaining(stream).starts_with("interface") => None,
        '@' if matches!(
            mode,
            LegacyMode::CSharp
                | LegacyMode::Java
                | LegacyMode::Scala
                | LegacyMode::Kotlin
                | LegacyMode::Ceylon
                | LegacyMode::Dart
        ) =>
        {
            stream.eat_while(|ch| identifier_ascii(ch) || (mode == LegacyMode::Dart && ch == '.'));
            Some(Some("meta"))
        }
        '"' if mode == LegacyMode::Java && remaining(stream) == "\"\"" => {
            stream.skip_to_end();
            Some(Scan::Triple.read(state, stream))
        }
        '"' if mode == LegacyMode::Scala && stream.match_str("\"\"", true, false) => {
            Some(Scan::Triple.read(state, stream))
        }
        '"' if mode == LegacyMode::Kotlin => {
            let triple = stream.match_str("\"\"", true, false);
            Some(Scan::Kotlin(triple).read(state, stream))
        }
        '"' if mode == LegacyMode::Ceylon => {
            let triple = stream.match_str("\"\"", true, false);
            Some(Scan::Ceylon(triple).read(state, stream))
        }
        '\'' if matches!(mode, LegacyMode::Scala | LegacyMode::Ceylon) => {
            static CHAR: LazyLock<Regex> = LazyLock::new(|| {
                Regex::new(concat!(
                r"^(\\[^'\x09-\x0d\x20\u{a0}\u{1680}\u{2000}-\u{200a}\u{2028}\u{2029}\u{202f}\u{205f}\u{3000}\u{feff}]+",
                r"|[^\\'\u{10000}-\u{10ffff}])'",
            )).unwrap()
            });
            if stream.match_regex(&CHAR, true).is_some() {
                Some(Some(if mode == LegacyMode::Scala {
                    "character"
                } else {
                    "string.special"
                }))
            } else {
                stream.eat_while(identifier);
                Some(Some("atom"))
            }
        }
        '=' if mode == LegacyMode::Scala
            && state.current_context().kind == Kind::Delimiter('}')
            && state.current_context().align == Some(true)
            && stream.eat('>').is_some() =>
        {
            control.reset_align = true;
            Some(Some("operator"))
        }
        '/' if matches!(
            mode,
            LegacyMode::Scala | LegacyMode::Kotlin | LegacyMode::Dart
        ) && stream.eat('*').is_some() =>
        {
            Some(Scan::NestedComment(1).read(state, stream))
        }
        '`' if state.ceylon_string.is_some() && stream.eat('`').is_some() => {
            let triple = state.ceylon_string.take().unwrap();
            Some(Scan::Ceylon(triple).read(state, stream))
        }
        '\'' | '"' if mode == LegacyMode::Dart => Some(dart_string(state, stream, ch, false)),
        'r' if mode == LegacyMode::Dart => {
            let quote = stream.eat_if(|ch| matches!(ch, '\'' | '"'))?;
            Some(dart_string(state, stream, quote, true))
        }
        '}' if mode == LegacyMode::Dart && !state.dart_strings.is_empty() => {
            state.scan = state.dart_strings.pop();
            Some(None)
        }
        _ => None,
    }
}

fn dart_string(
    state: &mut State,
    stream: &mut StringStream<'_>,
    quote: char,
    raw: bool,
) -> Option<&'static str> {
    let triple = if stream.eat(quote).is_some() {
        if stream.eat(quote).is_some() {
            true
        } else {
            return Some("string");
        }
    } else {
        false
    };
    Scan::DartString { quote, triple, raw }.read(state, stream)
}

fn directive(state: &mut State, stream: &mut StringStream<'_>) -> Option<&'static str> {
    if !state.start_of_line {
        return None;
    }
    while !stream.eol() {
        let tail = remaining(stream);
        if tail == "\\" {
            stream.next();
            state.scan = Some(Scan::Directive);
            break;
        }
        if tail.starts_with("//") || tail.starts_with("/*") {
            break;
        }
        stream.next();
    }
    Some("meta")
}

fn cpp_string(state: &mut State, stream: &mut StringStream<'_>) -> Option<&'static str> {
    stream.back_up(1).unwrap();
    if ["u8R", "uR", "UR", "LR", "R"]
        .into_iter()
        .any(|prefix| stream.match_str(prefix, true, false))
    {
        let tail = remaining(stream).strip_prefix('"')?;
        let (delimiter, _) = tail.split_once('(')?;
        if delimiter.encode_utf16().count() > 16
            || delimiter.contains(|ch| js_space(ch) || matches!(ch, '\\' | ')'))
        {
            return None;
        }
        stream.set_pos(stream.pos() + delimiter.len() + 2).unwrap();
        return Scan::RawCpp(delimiter.to_owned()).read(state, stream);
    }
    if ["u8", "u", "U", "L"]
        .into_iter()
        .any(|prefix| stream.match_str(prefix, true, false))
    {
        return stream
            .peek()
            .filter(|ch| matches!(ch, '"' | '\''))
            .map(|_| "string");
    }
    stream.next();
    None
}

fn identifier(ch: char) -> bool {
    identifier_ascii(ch) || ch >= '\u{a1}'
}

fn operator(mode: Option<LegacyMode>, ch: char) -> bool {
    matches!(
        ch,
        '+' | '-' | '*' | '&' | '%' | '=' | '<' | '>' | '!' | '?' | '|' | '/'
    ) || match mode {
        Some(LegacyMode::Scala) => matches!(ch, '#' | ':' | '@'),
        Some(LegacyMode::Ceylon) => matches!(ch, '^' | '~' | ':'),
        _ => false,
    }
}

fn contains(words: &str, word: &str) -> bool {
    // Empty lists and the Objective-C empty completion cannot match a token.
    !word.is_empty() && words.split(' ').any(|candidate| candidate == word)
}

fn dynamic_type(mode: LegacyMode, word: &str) -> bool {
    if mode == LegacyMode::Ceylon {
        return word.chars().next().is_some_and(|ch| {
            ch.len_utf16() == 1 && ch.to_uppercase().eq([ch]) && !ch.to_lowercase().eq([ch])
        });
    }
    (matches!(
        mode,
        LegacyMode::C
            | LegacyMode::Cpp
            | LegacyMode::NesC
            | LegacyMode::Squirrel
            | LegacyMode::ObjectiveC
            | LegacyMode::ObjectiveCpp
    ) && (matches!(
        word,
        "int"
            | "long"
            | "char"
            | "short"
            | "double"
            | "float"
            | "unsigned"
            | "signed"
            | "void"
            | "bool"
    ) || word
        .strip_suffix("_t")
        .and_then(|prefix| prefix.chars().next_back())
        .is_some_and(|ch| !js_line_terminator(ch))))
        || (matches!(mode, LegacyMode::ObjectiveC | LegacyMode::ObjectiveCpp)
            && matches!(
                word,
                "SEL" | "instancetype" | "id" | "Class" | "Protocol" | "BOOL"
            ))
}

fn uses_pointer_hook(mode: LegacyMode) -> bool {
    matches!(
        mode,
        LegacyMode::C | LegacyMode::Cpp | LegacyMode::ObjectiveC | LegacyMode::ObjectiveCpp
    )
}

fn reserved_c_identifier(word: &str) -> bool {
    word.strip_prefix('_')
        .and_then(|tail| tail.chars().next())
        .is_some_and(|ch| ch == '_' || (ch.len_utf16() == 1 && !ch.to_lowercase().eq([ch])))
}

fn number(mode: Option<LegacyMode>) -> &'static Regex {
    static DEFAULT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?i-u:^(?:0x[a-f0-9]+|0b[01]+|(?:[0-9]+\.?[0-9]*|\.[0-9]+)(?:e[-+]?[0-9]+)?)(?:u|ll?|l|f)?)"
    ).unwrap()
    });
    static JAVA: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?i-u:^(?:0x[a-f0-9_]+|0b[01_]+|(?:[0-9_]+\.?[0-9]*|\.[0-9]+)(?:e[-+]?[0-9_]+)?)(?:u|ll?|l|f)?)"
    ).unwrap()
    });
    static KOTLIN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?i-u:^(?:0x[a-f0-9_]+|0b[01_]+|(?:[0-9_]+(?:\.[0-9]+)?|\.[0-9]+)(?:e[-+]?[0-9_]+)?)(?:ul?|l|f)?)"
    ).unwrap()
    });
    static CEYLON: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
        r"(?i-u:^(?:#[0-9a-f_]+|\$[01_]+|[0-9_]+[kMGTPmunpf]?|[0-9_]+\.[0-9_]+(?:e[-+]?[0-9]+|[kMGTPmunpf]|)|))"
    ).unwrap()
    });
    static DART: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i-u:^(?:0x[a-f0-9_]+|(?:[0-9_]+\.?[0-9_]*|\.[0-9_]+)(?:e[-+]?[0-9_]+)?))")
            .unwrap()
    });
    match mode {
        Some(LegacyMode::Java) => &JAVA,
        Some(LegacyMode::Kotlin) => &KOTLIN,
        Some(LegacyMode::Ceylon) => &CEYLON,
        Some(LegacyMode::Dart) => &DART,
        _ => &DEFAULT,
    }
}
