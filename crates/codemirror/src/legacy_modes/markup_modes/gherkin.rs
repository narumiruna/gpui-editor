use super::{advance, eat_str, remaining};
use crate::{
    legacy_modes::{js_line_terminator, js_space},
    stream_parser::StringStream,
};

mod keywords;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    allow_background: bool,
    allow_scenario: bool,
    allow_steps: bool,
    allow_placeholders: bool,
    allow_multiline_argument: bool,
    in_keyword_line: bool,
    argument: Argument,
}

#[derive(Clone, Copy, Debug, Default)]
enum Argument {
    #[default]
    None,
    String,
    Table {
        header: bool,
    },
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            self.in_keyword_line = false;
            if let Argument::Table { header } = &mut self.argument {
                *header = false;
                if !remaining(stream)
                    .trim_start_matches(js_space)
                    .starts_with('|')
                {
                    self.argument = Argument::None;
                    self.allow_multiline_argument = false;
                }
            }
        }
        stream.eat_space();

        if self.allow_multiline_argument {
            match &self.argument {
                Argument::String => {
                    if eat_str(stream, "\"\"\"") {
                        self.argument = Argument::None;
                        self.allow_multiline_argument = false;
                    } else {
                        stream.eat_while(|ch| !js_line_terminator(ch));
                    }
                    return Some("string");
                }
                Argument::Table { header } => {
                    if stream.eat('|').is_some() {
                        stream.eat_space();
                        return Some("bracket");
                    }
                    stream.eat_while(|ch| ch != '|');
                    return Some(if *header { "header" } else { "string" });
                }
                Argument::None => {
                    if eat_str(stream, "\"\"\"") {
                        self.argument = Argument::String;
                        return Some("string");
                    }
                    if stream.eat('|').is_some() {
                        self.argument = Argument::Table { header: true };
                        return Some("bracket");
                    }
                }
            }
        }

        if stream.eat('#').is_some() {
            stream.eat_while(|ch| !js_line_terminator(ch));
            return Some("comment");
        }
        if !self.in_keyword_line {
            if let Some(after) = remaining(stream).strip_prefix('@')
                && after.starts_with(|ch| !js_space(ch))
            {
                stream.next();
                stream.eat_while(|ch| !js_space(ch));
                return Some("tag");
            }
            if keyword(stream, keywords::FEATURE, ":") {
                self.allow_scenario = true;
                self.allow_background = true;
                return self.keyword(false, false, false);
            }
            if self.allow_background && keyword(stream, keywords::BACKGROUND, ":") {
                self.allow_background = false;
                return self.keyword(true, false, false);
            }
            if self.allow_scenario && keyword(stream, keywords::OUTLINE, ":") {
                return self.keyword(true, true, false);
            }
        }
        if self.allow_scenario && keyword(stream, keywords::EXAMPLES, ":") {
            self.allow_placeholders = false;
            self.allow_steps = true;
            self.allow_background = false;
            self.allow_multiline_argument = true;
            return Some("keyword");
        }
        if !self.in_keyword_line {
            if self.allow_scenario && keyword(stream, keywords::SCENARIO, ":") {
                self.allow_background = false;
                return self.keyword(true, false, false);
            }
            if self.allow_steps && keyword(stream, keywords::STEP, "") {
                return self.keyword(true, true, true);
            }
        }

        if stream.eat('"').is_some() {
            stream.eat_while(|ch| ch != '"');
            stream.eat('"');
            return Some("string");
        }
        if self.allow_placeholders && stream.eat('<').is_some() {
            stream.eat_while(|ch| ch != '>');
            stream.eat('>');
            return Some("variable");
        }
        stream.next();
        stream.eat_while(|ch| !matches!(ch, '@' | '"' | '<' | '#'));
        None
    }

    fn keyword(
        &mut self,
        steps: bool,
        placeholders: bool,
        multiline: bool,
    ) -> Option<&'static str> {
        self.allow_steps = steps;
        self.allow_placeholders = placeholders;
        self.allow_multiline_argument = multiline;
        self.in_keyword_line = true;
        Some("keyword")
    }
}

fn keyword(stream: &mut StringStream<'_>, alternatives: &str, end: &str) -> bool {
    let text = remaining(stream);
    for candidate in alternatives.split('|') {
        if text
            .strip_prefix(candidate)
            .is_some_and(|rest| rest.starts_with(end))
        {
            advance(stream, candidate.len() + end.len());
            return true;
        }
    }
    false
}
