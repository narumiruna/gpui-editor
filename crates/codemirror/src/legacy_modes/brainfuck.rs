use crate::stream_parser::StringStream;

#[derive(Clone, Debug, Default)]
pub(super) struct State {
    comment_line: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        // Upstream consumes whitespace before checking for a line start.
        if stream.eat_space() {
            return None;
        }
        if stream.sol() {
            self.comment_line = false;
        }
        let style = match stream.next()? {
            '[' | ']' => "bracket",
            '+' | '-' => "keyword",
            '<' | '>' => "atom",
            '.' | ',' => "def",
            _ => {
                self.comment_line = true;
                "comment"
            }
        };
        if self.comment_line {
            self.comment_line = !stream.eol();
            Some("comment")
        } else {
            Some(style)
        }
    }
}
