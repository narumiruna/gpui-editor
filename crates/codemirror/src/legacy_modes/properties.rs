use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    position: Position,
    next_multiline: bool,
    after_section: bool,
}

#[derive(Clone, Debug, Default)]
enum Position {
    #[default]
    Key,
    Value,
    Comment,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let line_start = stream.sol() || self.after_section;
        self.after_section = false;
        if line_start {
            if self.next_multiline {
                self.next_multiline = false;
            } else {
                self.position = Position::Key;
            }
        }
        if stream.eol() && !self.next_multiline {
            self.position = Position::Key;
        }
        if line_start {
            stream.eat_space();
        }
        match stream.next() {
            Some('#' | '!' | ';') if line_start => {
                self.position = Position::Comment;
                stream.skip_to_end();
            }
            Some('[') if line_start => {
                self.after_section = true;
                stream.skip_to("]");
                stream.eat(']');
                return Some("header");
            }
            Some('=' | ':') => {
                self.position = Position::Value;
                return None;
            }
            Some('\\') if matches!(self.position, Position::Value) && stream.eol() => {
                self.next_multiline = true;
            }
            _ => {}
        }
        Some(match self.position {
            Position::Key => "def",
            Position::Value => "quote",
            Position::Comment => "comment",
        })
    }
}
