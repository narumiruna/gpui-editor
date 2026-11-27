use std::{collections::HashSet, sync::LazyLock};

use super::{StringStream, js_space, word};

static APPS: LazyLock<HashSet<&str>> = LazyLock::new(|| {
    include_str!("asterisk_apps.txt")
        .split_ascii_whitespace()
        .collect()
});

#[derive(Clone, Copy, Debug, Default)]
enum Phase {
    #[default]
    Base,
    Assignment,
    Extension,
    Priority,
    Application,
}

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    phase: Phase,
    block_comment: bool,
    same: bool,
    include: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }
        match self.phase {
            Phase::Base => self.base(stream),
            Phase::Assignment => {
                stream.eat_while(|ch| !js_space(ch));
                if matches!(stream.current(), "=" | "=>") {
                    self.phase = Phase::Extension;
                    Some("strong")
                } else {
                    // An error leaves the directive flags in effect.
                    self.phase = Phase::Base;
                    stream.skip_to_end();
                    Some("error")
                }
            }
            Phase::Extension => {
                self.phase = Phase::Priority;
                stream.eat_while(|ch| ch != ',');
                if std::mem::take(&mut self.include) {
                    stream.skip_to_end();
                    self.phase = Phase::Base;
                }
                if std::mem::take(&mut self.same) {
                    self.phase = Phase::Application;
                }
                Some("tag")
            }
            Phase::Priority => {
                self.phase = Phase::Application;
                stream.next();
                stream.eat_while(|ch| ch != ',');
                Some("number")
            }
            Phase::Application => {
                stream.eat_while(|ch| ch == ',');
                if stream.current() == "," {
                    return None;
                }
                self.phase = Phase::Base;
                stream.eat_while(word);
                let name = stream.current();
                let known = if name.bytes().any(|ch| ch.is_ascii_uppercase()) {
                    APPS.contains(name.to_ascii_lowercase().as_str())
                } else {
                    APPS.contains(name)
                };
                known.then_some("def")
            }
        }
    }

    fn base(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let ch = stream.next()?;
        if self.block_comment {
            if (ch == '-' && stream.match_str("-;", true, false))
                || (stream.skip_to("--;") && stream.match_str("--;", true, false))
            {
                self.block_comment = false;
            } else {
                stream.skip_to_end();
            }
            return Some("comment");
        }
        match ch {
            ';' => {
                if stream.match_str("--", true, false) && stream.peek() != Some('-') {
                    self.block_comment = true;
                } else {
                    stream.skip_to_end();
                }
                return Some("comment");
            }
            '[' => {
                stream.skip_to("]");
                stream.eat(']');
                return Some("header");
            }
            '"' => {
                // The upstream quote token stops before the next quote.
                stream.skip_to("\"");
                return Some("string");
            }
            '\'' => {
                stream.skip_to("'");
                return Some("string.special");
            }
            '#' => {
                stream.eat_while(word);
                if matches!(stream.current(), "#include" | "#exec") {
                    stream.skip_to_end();
                    return Some("strong");
                }
            }
            '$' if stream.peek() == Some('{') => {
                stream.skip_to("}");
                stream.eat('}');
                return Some("variableName.special");
            }
            _ => {}
        }
        stream.eat_while(word);
        match stream.current() {
            "same" => self.same = true,
            "include" | "ignorepat" | "switch" => self.include = true,
            "exten" => {}
            _ => return None,
        }
        self.phase = Phase::Assignment;
        Some("atom")
    }
}
