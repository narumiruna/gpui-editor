use std::sync::LazyLock;

use regex::Regex;

use crate::stream_parser::StringStream;

use super::{js_line_terminator, js_space};

static PROTOCOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^HTTP/[0-9]\.[0-9]").unwrap());

#[derive(Clone, Debug, Default)]
pub(super) enum State {
    #[default]
    Start,
    StatusCode,
    StatusText,
    RequestPath,
    RequestProtocol,
    Header,
    Body,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if !matches!(self, Self::Header | Self::Body) && stream.eat_space() {
            return None;
        }
        match self {
            Self::Start => {
                if stream.match_regex(&PROTOCOL, true).is_some() {
                    *self = Self::StatusCode;
                    Some("keyword")
                } else if stream.eat_while(|ch| ch.is_ascii_uppercase())
                    && matches!(stream.peek(), Some(' ' | '\t'))
                {
                    *self = Self::RequestPath;
                    Some("keyword")
                } else {
                    self.fail_first_line(stream)
                }
            }
            Self::StatusCode => {
                let start = stream.pos();
                if stream.eat_while(|ch| ch.is_ascii_digit()) {
                    *self = Self::StatusText;
                    match stream.line()[start..stream.pos()].parse::<u16>() {
                        Ok(100..=399) => Some("atom"),
                        _ => Some("error"),
                    }
                } else {
                    self.fail_first_line(stream)
                }
            }
            Self::StatusText => {
                stream.skip_to_end();
                *self = Self::Header;
                None
            }
            Self::RequestPath => {
                stream.eat_while(|ch| !js_space(ch));
                *self = Self::RequestProtocol;
                Some("string.special")
            }
            Self::RequestProtocol => {
                if stream.match_regex(&PROTOCOL, true).is_some() && stream.eol() {
                    *self = Self::Header;
                    Some("keyword")
                } else {
                    self.fail_first_line(stream)
                }
            }
            Self::Header => {
                if stream.sol() && stream.eat_if(|ch| matches!(ch, ' ' | '\t')).is_none() {
                    stream.eat_while(|ch| ch != ':' && !js_line_terminator(ch));
                    if stream.eat(':').is_some() {
                        Some("atom")
                    } else {
                        stream.skip_to_end();
                        Some("error")
                    }
                } else {
                    stream.skip_to_end();
                    Some("string")
                }
            }
            Self::Body => {
                stream.skip_to_end();
                None
            }
        }
    }

    fn fail_first_line(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        stream.skip_to_end();
        *self = Self::Header;
        Some("error")
    }
}
