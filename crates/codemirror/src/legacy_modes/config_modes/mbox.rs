use super::{super::js_line_terminator, StringStream};

pub(super) const HEADERS: &[&str] = &[
    "From",
    "Sender",
    "Reply-To",
    "To",
    "Cc",
    "Bcc",
    "Message-ID",
    "In-Reply-To",
    "References",
    "Resent-From",
    "Resent-Sender",
    "Resent-To",
    "Resent-Cc",
    "Resent-Bcc",
    "Resent-Message-ID",
    "Return-Path",
    "Received",
    "Date",
    "Subject",
    "Comments",
    "Keywords",
    "Resent-Date",
];

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum State {
    #[default]
    Body,
    Separator,
    Subject,
    TextHeader,
    AddressHeader,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.sol() {
            return self.start_line(stream);
        }
        match self {
            Self::Body => {
                stream.skip_to_end();
                None
            }
            Self::Separator => Some(separator_token(stream)),
            Self::Subject | Self::TextHeader => {
                stream.skip_to_end();
                Some(if matches!(self, Self::Subject) {
                    "header"
                } else {
                    "string"
                })
            }
            Self::AddressHeader => Some(address_token(stream)),
        }
    }

    fn start_line(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if matches!(self, Self::Subject | Self::TextHeader | Self::AddressHeader)
            && stream.eat_if(|ch| matches!(ch, ' ' | '\t')).is_some()
        {
            return None;
        }
        if stream.match_str("From ", true, false) {
            *self = Self::Separator;
            return Some("atom");
        }
        for &name in HEADERS {
            if stream
                .line()
                .strip_prefix(name)
                .is_some_and(|tail| tail.starts_with(": "))
            {
                *self = match name {
                    "Subject" => Self::Subject,
                    "Date" | "Comments" | "Keywords" | "Resent-Date" => Self::TextHeader,
                    _ => Self::AddressHeader,
                };
                stream.set_pos(name.len() + 2).unwrap();
                return Some("atom");
            }
        }
        // Custom headers are only recognized inside an established header block.
        if !matches!(self, Self::Body)
            && let Some(colon) = stream.line().find(':').filter(|&pos| pos > 0)
        {
            *self = Self::AddressHeader;
            stream.set_pos(colon + 1).unwrap();
            return Some("atom");
        }
        *self = Self::Body;
        stream.skip_to_end();
        None
    }
}

fn separator_token(stream: &mut StringStream<'_>) -> &'static str {
    // The address regex treats only literal spaces as separators, not tabs.
    let mut offset = 0;
    for part in stream.line()[stream.pos()..].split(' ') {
        if part
            .match_indices('@')
            .any(|(pos, _)| pos > 0 && pos + 1 < part.len())
        {
            if offset == 0 {
                stream.set_pos(stream.pos() + part.len()).unwrap();
                return "link";
            }
            stream.set_pos(stream.pos() + offset).unwrap();
            return "atom";
        }
        // The lookahead can see an address after a space, but its dot cannot
        // cross a JavaScript line terminator on its way to that address.
        if part.contains(js_line_terminator) {
            break;
        }
        offset += part.len() + 1;
    }
    stream.skip_to_end();
    "atom"
}

fn address_token(stream: &mut StringStream<'_>) -> &'static str {
    let before_break = stream.line()[stream.pos()..]
        .split(js_line_terminator)
        .next()
        .unwrap();
    if let Some(open) = before_break.find('<')
        && let Some(close) = before_break[open + 1..].find('>')
    {
        if open == 0 {
            stream.set_pos(stream.pos() + close + 2).unwrap();
            return "string link";
        }
        stream.set_pos(stream.pos() + open).unwrap();
        return "string";
    }
    stream.skip_to_end();
    "string"
}
