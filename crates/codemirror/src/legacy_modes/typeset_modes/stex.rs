use std::sync::LazyLock;

use regex::Regex;

use crate::stream_parser::StringStream;

static MATH_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[0-9]+\.[0-9]*|[0-9]*\.[0-9]+|[0-9]+)").unwrap());

#[derive(Clone, Copy, Debug)]
enum Phase {
    Normal,
    Params,
    Math(Option<&'static str>),
}

#[derive(Clone, Debug)]
struct Command {
    styles: &'static [Option<&'static str>],
    bracket_count: usize,
}

impl Command {
    fn new(name: &str) -> Self {
        let styles: &[_] = match name {
            "importmodule" => &[Some("string"), Some("builtin")],
            "documentclass" => &[None, Some("atom")],
            "usepackage" | "begin" | "end" | "label" | "ref" | "eqref" | "cite" | "bibitem"
            | "Bibitem" | "RBibitem" => &[Some("atom")],
            _ => &[],
        };
        Self {
            styles,
            bracket_count: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub(in crate::legacy_modes) struct State {
    phase: Phase,
    // Each checkpoint owns its command argument indexes.
    commands: Vec<Command>,
}

impl State {
    pub(super) fn new(math: bool) -> Self {
        Self {
            phase: if math {
                Phase::Math(None)
            } else {
                Phase::Normal
            },
            commands: Vec::new(),
        }
    }

    pub(super) fn blank_line(&mut self) {
        self.phase = Phase::Normal;
        self.commands.clear();
    }

    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self.phase {
            Phase::Normal => self.normal(stream),
            Phase::Math(end) => self.math(stream, end),
            Phase::Params => {
                if stream.eat_if(|ch| matches!(ch, '{' | '[')).is_some() {
                    if let Some(command) = self.commands.last_mut() {
                        command.bracket_count += 1;
                    }
                    self.phase = Phase::Normal;
                    return Some("bracket");
                }
                if stream
                    .eat_if(|ch| matches!(ch, ' ' | '\t' | '\r'))
                    .is_some()
                {
                    return None;
                }
                self.commands.pop();
                self.phase = Phase::Normal;
                self.normal(stream)
            }
        }
    }

    fn normal(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if command(stream, |ch| {
            ch.is_ascii_alphabetic()
                || matches!(ch, '@' | '\u{c0}'..='\u{1fff}' | '\u{2060}'..='\u{10ffff}')
        }) {
            self.commands.push(Command::new(&stream.current()[1..]));
            self.phase = Phase::Params;
            return Some("tag");
        }
        if escape(stream, "$&%#{}_,;!/\\") {
            return Some("tag");
        }
        for (start, end) in [("\\[", "\\]"), ("\\(", "\\)"), ("$$", "$$"), ("$", "$")] {
            if stream.match_str(start, true, false) {
                self.phase = Phase::Math(Some(end));
                return Some("keyword");
            }
        }
        match stream.next()? {
            '%' => {
                stream.skip_to_end();
                Some("comment")
            }
            '}' | ']' if self.commands.is_empty() => Some("error"),
            '}' | ']' => {
                self.phase = Phase::Params;
                Some("bracket")
            }
            '{' | '[' => {
                self.commands.push(Command::new(""));
                Some("bracket")
            }
            ch if ch.is_ascii_digit() => {
                stream.eat_while(|ch| word(ch) || matches!(ch, '.' | '%'));
                Some("atom")
            }
            _ => {
                stream.eat_while(|ch| word(ch) || ch == '-');
                let command = self
                    .commands
                    .iter()
                    .rev()
                    .find(|command| !command.styles.is_empty())?;
                *command.styles.get(command.bracket_count.checked_sub(1)?)?
            }
        }
    }

    fn math(&mut self, stream: &mut StringStream<'_>, end: Option<&str>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        if end.is_some_and(|end| stream.match_str(end, true, false)) {
            self.phase = Phase::Normal;
            return Some("keyword");
        }
        if command(stream, |ch| ch.is_ascii_alphabetic() || ch == '@') {
            return Some("tag");
        }
        if stream.eat_while(|ch| ch.is_ascii_alphabetic()) {
            return Some("variableName.special");
        }
        if escape(stream, "$&%#{}_,;!/")
            || stream.eat_if(|ch| matches!(ch, '^' | '_' | '&')).is_some()
        {
            return Some("tag");
        }
        if stream
            .eat_if(|ch| "+-<>|=,/@!*:;'\"`~#?".contains(ch))
            .is_some()
        {
            return None;
        }
        if stream.match_regex(&MATH_NUMBER, true).is_some() {
            return Some("number");
        }
        Some(match stream.next()? {
            '{' | '}' | '[' | ']' | '(' | ')' => "bracket",
            '%' => {
                stream.skip_to_end();
                "comment"
            }
            _ => "error",
        })
    }
}

fn command(stream: &mut StringStream<'_>, predicate: impl Fn(char) -> bool) -> bool {
    if stream.peek() != Some('\\') {
        return false;
    }
    let start = stream.pos();
    stream.next();
    if stream.eat_while(predicate) {
        true
    } else {
        stream.set_pos(start).unwrap();
        false
    }
}

fn escape(stream: &mut StringStream<'_>, characters: &str) -> bool {
    if let Some(ch) = stream.line()[stream.pos()..]
        .strip_prefix('\\')
        .and_then(|tail| tail.chars().next())
        && characters.contains(ch)
    {
        stream.set_pos(stream.pos() + 1 + ch.len_utf8()).unwrap();
        return true;
    }
    false
}

fn word(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
