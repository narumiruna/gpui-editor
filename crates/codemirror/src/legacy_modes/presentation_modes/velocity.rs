use super::{advance, remaining, word};
use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    phase: Phase,
    before_params: bool,
    in_params: bool,
    in_string: bool,
    last_builtin: bool,
}

#[derive(Clone, Copy, Debug, Default)]
enum Phase {
    #[default]
    Base,
    Quote(char),
    Comment,
    Unparsed,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        // This also runs on continuation lines in quotes, comments and raw text.
        if stream.eat_space() {
            return None;
        }
        match self.phase {
            Phase::Base => self.base(stream),
            _ => Some(self.continuation(stream)),
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let before_params = std::mem::take(&mut self.before_params);
        let ch = stream.next()?;
        match ch {
            '\'' if !self.in_string && self.in_params => {
                self.last_builtin = false;
                self.start(stream, Phase::Quote(ch))
            }
            '"' => {
                self.last_builtin = false;
                if self.in_string {
                    self.in_string = false;
                    Some("string")
                } else if self.in_params {
                    self.start(stream, Phase::Quote(ch))
                } else {
                    None
                }
            }
            '[' | ']' | '{' | '}' | '(' | ')' | ',' | ';' | '.' => {
                if ch == '(' && before_params {
                    self.in_params = true;
                } else if ch == ')' {
                    self.in_params = false;
                    self.last_builtin = true;
                }
                None
            }
            '0'..='9' => {
                self.last_builtin = false;
                stream.eat_while(|ch| word(ch) || ch == '.');
                Some("number")
            }
            '#' if stream.eat('*').is_some() => {
                self.last_builtin = false;
                self.start(stream, Phase::Comment)
            }
            '#' if eat_unparsed_open(stream) => {
                self.last_builtin = false;
                self.start(stream, Phase::Unparsed)
            }
            '#' if stream.eat('#').is_some() => {
                self.last_builtin = false;
                stream.skip_to_end();
                Some("comment")
            }
            '$' => {
                stream.eat('!');
                stream.eat_while(|ch| word(ch) || "$.-{}".contains(ch));
                if SPECIALS.contains(&stream.current()) {
                    Some("keyword")
                } else {
                    self.last_builtin = true;
                    self.before_params = true;
                    Some("builtin")
                }
            }
            ch if operator(ch) => {
                self.last_builtin = false;
                stream.eat_while(operator);
                Some("operator")
            }
            _ => {
                stream.eat_while(|ch| word(ch) || "${}@".contains(ch));
                let text = stream.current();
                if KEYWORDS.contains(&text) {
                    return Some("keyword");
                }
                if FUNCTIONS.contains(&text)
                    || stream.peek() == Some('(')
                        && macro_name(text)
                        && !FUNCTIONS.contains(&text.to_ascii_lowercase().as_str())
                {
                    self.before_params = true;
                    self.last_builtin = false;
                    return Some("keyword");
                }
                if self.in_string {
                    self.last_builtin = false;
                    return Some("string");
                }
                if self.last_builtin && stream.line()[..stream.start()].ends_with('.') {
                    return Some("builtin");
                }
                self.last_builtin = false;
                None
            }
        }
    }

    fn start(&mut self, stream: &mut StringStream<'_>, phase: Phase) -> Option<&'static str> {
        self.phase = phase;
        Some(self.continuation(stream))
    }

    fn continuation(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        let (style, done) = match self.phase {
            Phase::Quote(quote) => ("string", self.quote(stream, quote)),
            Phase::Comment => ("comment", close_comment(stream)),
            Phase::Unparsed => ("meta", close_unparsed(stream)),
            Phase::Base => unreachable!(),
        };
        if done {
            self.phase = Phase::Base;
        }
        style
    }

    fn quote(&mut self, stream: &mut StringStream<'_>, quote: char) -> bool {
        let mut escaped = false;
        while let Some(ch) = stream.next() {
            if ch == quote && !escaped {
                return true;
            }
            // In upstream, the first UTF-16 unit of this scalar used the escape.
            if ch.len_utf16() == 2 {
                escaped = false;
            }
            if quote == '"' && stream.peek() == Some('$') && !escaped {
                self.in_string = true;
                return true;
            }
            escaped = !escaped && ch == '\\';
        }
        false
    }
}

fn operator(ch: char) -> bool {
    "+-*&%=<>!?:/|".contains(ch)
}

fn macro_name(text: &str) -> bool {
    let Some(name) = text.strip_prefix('#') else {
        return false;
    };
    let name = name.strip_prefix('@').unwrap_or(name).trim_end_matches(' ');
    !name.is_empty() && name.chars().all(word)
}

fn eat_unparsed_open(stream: &mut StringStream<'_>) -> bool {
    let text = remaining(stream);
    let Some(after) = text
        .trim_start_matches(' ')
        .strip_prefix('[')
        .and_then(|tail| tail.trim_start_matches(' ').strip_prefix('['))
    else {
        return false;
    };
    advance(stream, text.len() - after.len());
    true
}

fn close_comment(stream: &mut StringStream<'_>) -> bool {
    let mut star = false;
    for ch in stream {
        if star && ch == '#' {
            return true;
        }
        star = ch == '*';
    }
    false
}

fn close_unparsed(stream: &mut StringStream<'_>) -> bool {
    let mut brackets = 0;
    for ch in stream {
        if brackets == 2 && ch == '#' {
            return true;
        }
        if ch == ']' {
            brackets = (brackets + 1).min(3);
        } else if ch != ' ' {
            brackets = 0;
        }
    }
    false
}

const KEYWORDS: &[&str] = &[
    "#end", "#else", "#break", "#stop", "#[[", "#]]", "#{end}", "#{else}", "#{break}", "#{stop}",
];
const FUNCTIONS: &[&str] = &[
    "#if",
    "#elseif",
    "#foreach",
    "#set",
    "#include",
    "#parse",
    "#macro",
    "#define",
    "#evaluate",
    "#{if}",
    "#{elseif}",
    "#{foreach}",
    "#{set}",
    "#{include}",
    "#{parse}",
    "#{macro}",
    "#{define}",
    "#{evaluate}",
];
const SPECIALS: &[&str] = &[
    "$foreach.count",
    "$foreach.hasNext",
    "$foreach.first",
    "$foreach.last",
    "$foreach.topmost",
    "$foreach.parent.count",
    "$foreach.parent.hasNext",
    "$foreach.parent.first",
    "$foreach.parent.last",
    "$foreach.parent",
    "$velocityCount",
    "$!bodyContent",
    "$bodyContent",
];
