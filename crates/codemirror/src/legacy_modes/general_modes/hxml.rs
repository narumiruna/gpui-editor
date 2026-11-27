use super::StringStream;

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct State {
    in_string: bool,
}

impl State {
    pub(super) fn token(&mut self, stream: &mut StringStream<'_>) -> Option<&'static str> {
        let ch = stream.peek()?;
        if ch == '#' {
            stream.skip_to_end();
            return Some("comment");
        }
        if stream.sol() && ch == '-' {
            stream.next();
            let long = stream.eat('-').is_some();
            let define = stream.eat('D').is_some();
            stream.eat_while(|ch| ch.is_ascii_alphabetic());
            return Some(if define {
                "keyword c"
            } else if long {
                "keyword a"
            } else {
                "variable-2"
            });
        }
        if !self.in_string && ch == '\'' {
            self.in_string = true;
            stream.next();
        }
        if self.in_string {
            if stream.skip_to("'") {
                stream.next();
                self.in_string = false;
            } else {
                stream.skip_to_end();
            }
            return Some("string");
        }
        stream.next();
        None
    }
}
