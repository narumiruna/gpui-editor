use std::sync::Arc;

use crate::stream_parser::StringStream;

use super::{js_line_terminator, js_space};

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    phase: Phase,
    armor_type: Arc<str>,
}

#[derive(Clone, Debug, Default)]
enum Phase {
    #[default]
    Top,
    Headers,
    HeaderValue,
    Body,
    End,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        match self.phase {
            Phase::Top => {
                if let Some(armor_type) = marker(stream, "-----BEGIN ") {
                    self.phase = Phase::Headers;
                    self.armor_type = armor_type.into();
                    Some("tag")
                } else {
                    error_if_not_empty(stream)
                }
            }
            Phase::Headers => {
                if stream.sol() && stream.eat_while(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
                    if stream.eat(':').is_some() {
                        self.phase = Phase::HeaderValue;
                        return Some("atom");
                    }
                    // We read a nonempty word without a colon. This is not a header.
                    stream.skip_to_end();
                    self.phase = Phase::Body;
                    return Some("error");
                }
                let result = error_if_not_empty(stream);
                if result.is_some() {
                    self.phase = Phase::Body;
                }
                result
            }
            Phase::HeaderValue => {
                stream.skip_to_end();
                self.phase = Phase::Headers;
                Some("string")
            }
            Phase::Body => {
                if let Some(armor_type) = marker(stream, "-----END ") {
                    if armor_type == &*self.armor_type {
                        self.phase = Phase::End;
                        Some("tag")
                    } else {
                        Some("error")
                    }
                } else if stream.eat_while(|ch| ch.is_ascii_alphanumeric() || "+/=".contains(ch)) {
                    None
                } else {
                    stream.next();
                    Some("error")
                }
            }
            Phase::End => error_if_not_empty(stream),
        }
    }

    pub(super) fn blank_line(&mut self) {
        if matches!(self.phase, Phase::Headers) {
            self.phase = Phase::Body;
        }
    }
}

fn marker<'a>(stream: &mut StringStream<'a>, prefix: &str) -> Option<&'a str> {
    if !stream.sol() {
        return None;
    }
    let armor_type = stream
        .line()
        .strip_prefix(prefix)?
        .trim_end_matches(js_space)
        .strip_suffix("-----")?;
    if armor_type.contains(js_line_terminator) {
        return None;
    }
    stream.skip_to_end();
    Some(armor_type)
}

fn error_if_not_empty(stream: &mut StringStream<'_>) -> Option<&'static str> {
    stream.eat_space();
    let error = !stream.eol();
    stream.skip_to_end();
    error.then_some("error")
}
