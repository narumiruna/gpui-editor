use super::{super::js_space, word};
use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    quote: Option<char>,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        if stream.eat_space() {
            return None;
        }

        let pos = stream.pos();
        let ch = stream.next()?;
        if ch == '$' {
            return Some(variable(stream));
        }
        if self.quote.is_some() {
            stream.set_pos(pos).unwrap();
            return Some(self.string(stream));
        }

        // The upstream command pattern starts after the first character.
        let tail = &stream.line()[stream.pos()..];
        let name = tail.trim_start_matches(js_space);
        if let Some(end) = name.find(|ch| !word(ch))
            && end > 0
            && (name[end..].starts_with('(') || name[end..].starts_with(" ("))
        {
            let space = usize::from(name[end..].starts_with(' '));
            stream
                .set_pos(stream.pos() + tail.len() - name.len() + end + space)
                .unwrap();
            return Some("def");
        }

        match ch {
            '#' => {
                stream.skip_to_end();
                Some("comment")
            }
            '\'' | '"' => {
                self.quote = Some(ch);
                Some(self.string(stream))
            }
            '(' | ')' => Some("bracket"),
            ch if ch.is_ascii_digit() => Some("number"),
            _ => {
                stream.eat_while(|ch| word(ch) || ch == '-');
                None
            }
        }
    }

    fn string(&mut self, stream: &mut StringStream<'_>) -> &'static str {
        let quote = self.quote.unwrap();
        let mut previous = None;
        while let Some(ch) = stream.next() {
            if ch == quote {
                self.quote = None;
                break;
            }
            if ch == '$' && previous != Some('\\') && quote == '"' {
                stream.back_up(1).unwrap();
                break;
            }
            previous = Some(ch);
        }
        "string"
    }
}

fn variable(stream: &mut StringStream<'_>) -> &'static str {
    let start = stream.pos();
    stream.eat('{');
    if stream.eat_while(word) {
        stream.eat('}');
        "variableName.special"
    } else {
        stream.set_pos(start).unwrap();
        "variable"
    }
}
