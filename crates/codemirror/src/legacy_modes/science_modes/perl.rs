use std::sync::LazyLock;

use regex::Regex;

use super::{eat_text, remaining, word};
use crate::{legacy_modes::js_space, stream_parser::StringStream};

mod words;

const REGEX_STYLE: &str = "string.special";

#[derive(Clone, Debug, Default)]
pub(super) struct State(Scan);

#[derive(Clone, Debug, Default)]
enum Scan {
    #[default]
    Base,
    Delimited(Delimited),
    Pending(Delimited),
    UntilLine(String),
}

#[derive(Clone, Copy, Debug)]
struct Delimited {
    end: char,
    style: &'static str,
    modifiers: bool,
    second: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match &self.0 {
            Scan::Delimited(scan) => return Some(self.delimited(stream, *scan)),
            Scan::UntilLine(end) => {
                if stream.line() == end {
                    self.0 = Scan::Base;
                }
                stream.skip_to_end();
                return Some("string");
            }
            _ => {}
        }
        if stream.eat_space() {
            return None;
        }
        if let Scan::Pending(scan) = self.0 {
            return Some(self.delimited(stream, scan));
        }
        self.base(stream)
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        static NUMBER: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
            r"^(?:-?(?:(?:[0-9][0-9_]*)?\.[0-9]+(?:e[+-]?[0-9]+)?|[0-9]+\.[0-9]*)|0x[0-9a-fA-F_]+|0b[01_]+|[0-9][0-9_]*(?:e[+-]?[0-9]+)?)"
        ).unwrap()
        });
        if stream.match_regex(&NUMBER, true).is_some() {
            return Some("number");
        }
        if let Some(rest) = remaining(stream).strip_prefix("<<")
            && rest.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_')
        {
            eat_text(stream, "<<");
            stream.eat_while(word);
            return Some(self.until_line(stream, &stream.current()[2..]));
        }
        if stream.sol()
            && remaining(stream)
                .strip_prefix("=item")
                .is_some_and(|rest| !rest.starts_with(word))
        {
            return Some(self.until_line(stream, "=cut"));
        }

        let prefix = &stream.line()[..stream.pos()];
        let previous = prefix.chars().next_back();
        let ch = stream.next()?;
        let after_char = stream.pos();
        match ch {
            '"' | '\'' => {
                if prefix.ends_with("<<") {
                    stream.eat_while(word);
                    let name = &stream.current()[1..];
                    if !name.is_empty() && stream.eat(ch).is_some() {
                        return Some(self.until_line(stream, name));
                    }
                    stream.set_pos(after_char).unwrap();
                }
                return Some(self.open(stream, ch, "string", false));
            }
            'q' if !previous.is_some_and(word) => {
                let variant = stream
                    .peek()
                    .filter(|ch| matches!(ch, 'x' | 'q' | 'w' | 'r'));
                let rest = remaining(stream);
                let opener = rest[usize::from(variant.is_some())..].chars().next();
                if let Some(end) = opener.and_then(closing_delimiter) {
                    let style = match variant {
                        Some('x' | 'r') => REGEX_STYLE,
                        Some('w') => "bracket",
                        _ => "string",
                    };
                    // Paired quote openers in upstream never advance beyond the last code unit.
                    let target = after_char + usize::from(variant.is_some()) + 1;
                    let last_start = stream.line().char_indices().next_back().unwrap().0;
                    if matches!(opener, Some('(' | '[' | '{' | '<')) {
                        stream.set_pos(target.min(last_start)).unwrap();
                    } else {
                        stream.set_pos(target).unwrap();
                    }
                    return Some(self.open(stream, end, style, false));
                }
            }
            'm' if !previous.is_some_and(word) => {
                if let Some(end) = take_delimiter(stream) {
                    return Some(self.open(stream, end, REGEX_STYLE, false));
                }
            }
            's' | 'y' | 't' if !previous.is_some_and(|ch| word(ch) || "/>]})".contains(ch)) => {
                if (ch != 't' || stream.eat('r').is_some())
                    && let Some(end) = take_delimiter(stream)
                {
                    return Some(self.open(stream, end, REGEX_STYLE, true));
                }
            }
            '`' => return Some(self.open(stream, ch, "builtin", false)),
            '/' => {
                return Some(if prefix.trim_end_matches(js_space).ends_with('~') {
                    self.open(stream, ch, REGEX_STYLE, false)
                } else {
                    "operator"
                });
            }
            _ => {}
        }
        if ch == '$' {
            if name_or_braced(stream, |ch| ch.is_ascii_digit()) {
                return Some("builtin");
            }
            stream.set_pos(after_char).unwrap();
        }
        if "$@%".contains(ch) {
            let special = if stream.eat('^').is_some() {
                stream.eat_if(|ch| ch.is_ascii_uppercase()).is_some()
            } else {
                !previous.is_some_and(|ch| "@$%&".contains(ch))
                    && stream
                        .eat_if(|ch| "=|\\-#?@;:&`~^![]*'\"$+.,/<>()".contains(ch))
                        .is_some()
            };
            if special && words::style(stream.current()).is_some() {
                return Some("builtin");
            }
            stream.set_pos(after_char).unwrap();
        }
        if "$@%&".contains(ch) && name_or_braced(stream, |ch| word(ch) || ch == '$') {
            return Some(if words::style(stream.current()).is_some() {
                "builtin"
            } else {
                "variable"
            });
        }
        if ch == '#' && previous != Some('$') {
            stream.skip_to_end();
            return Some("comment");
        }
        if operator(ch) {
            // Start after any part of a failed braced variable, as upstream does.
            let pos = stream.pos();
            stream.eat_while(operator);
            if words::style(stream.current()).is_some() {
                return Some("operator");
            }
            stream.set_pos(pos).unwrap();
        }
        if ch == '_' && stream.start() == 0 {
            let rest = remaining(stream);
            let style = if rest.starts_with("_END__") {
                Some("comment")
            } else if rest.starts_with("_DATA__") {
                Some("builtin")
            } else if rest == "_C__" {
                Some("string")
            } else {
                None
            };
            if let Some(style) = style {
                return Some(self.open(stream, '\0', style, false));
            }
        }
        if word(ch) && previous == Some('{') {
            let end = stream.pos();
            stream.eat_while(word);
            if stream.peek() == Some('}') {
                return Some("string");
            }
            stream.set_pos(end).unwrap();
        }
        if ch.is_ascii_uppercase() {
            stream.eat_while(|ch| ch.is_ascii_uppercase() || ch == '_');
            if !stream
                .peek()
                .is_some_and(|ch| ch.is_ascii_digit() || ch.is_ascii_lowercase())
            {
                return Some(word_style(stream.current(), previous));
            }
            stream.set_pos(after_char).unwrap();
        }
        if word(ch) {
            stream.eat_while(word);
            return Some(word_style(stream.current(), previous));
        }
        None
    }

    fn open(
        &mut self,
        stream: &mut StringStream<'_>,
        end: char,
        style: &'static str,
        second: bool,
    ) -> &'static str {
        self.delimited(
            stream,
            Delimited {
                end,
                style,
                modifiers: style == REGEX_STYLE,
                second,
            },
        )
    }

    fn delimited(&mut self, stream: &mut StringStream<'_>, scan: Delimited) -> &'static str {
        let mut escaped = false;
        while let Some(ch) = stream.next() {
            if ch == scan.end && !escaped {
                if scan.second {
                    self.0 = Scan::Pending(Delimited {
                        second: false,
                        ..scan
                    });
                } else {
                    self.0 = Scan::Base;
                    if scan.modifiers {
                        stream.eat_while(|ch| "goseximacplud".contains(ch));
                    }
                }
                return scan.style;
            }
            escaped = !escaped && ch == '\\';
        }
        self.0 = Scan::Delimited(scan);
        scan.style
    }

    fn until_line(&mut self, stream: &mut StringStream<'_>, end: &str) -> &'static str {
        if stream.line() != end {
            self.0 = Scan::UntilLine(end.to_owned());
        }
        stream.skip_to_end();
        "string"
    }
}

fn word_style(word: &str, previous: Option<char>) -> &'static str {
    if previous == Some(':') {
        "meta"
    } else {
        words::style(word).unwrap_or("meta")
    }
}

fn name_or_braced(stream: &mut StringStream<'_>, ch: impl FnMut(char) -> bool + Copy) -> bool {
    stream.eat_while(ch)
        || stream.eat('{').is_some() && stream.eat_while(ch) && stream.eat('}').is_some()
}

fn closing_delimiter(ch: char) -> Option<char> {
    Some(match ch {
        '(' => ')',
        '[' => ']',
        '{' => '}',
        '<' => '>',
        '^' | '\'' | '"' | '!' | '~' | '/' => ch,
        _ => return None,
    })
}

fn take_delimiter(stream: &mut StringStream<'_>) -> Option<char> {
    let end = closing_delimiter(stream.peek()?)?;
    stream.next();
    Some(end)
}

fn operator(ch: char) -> bool {
    ":+-^*$&%@=<>!?|/~.".contains(ch)
}
