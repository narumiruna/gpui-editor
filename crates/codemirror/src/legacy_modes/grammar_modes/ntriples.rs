use super::StringStream;

pub(super) fn token(stream: &mut StringStream<'_>) -> Option<&'static str> {
    // Upstream records values and advances a location state, but neither
    // affects token boundaries or styles. No state needs to survive a token.
    Some(match stream.next()? {
        '<' => {
            stream.eat_while(|ch| !matches!(ch, '#' | '>'));
            stream.eat('>');
            "variable"
        }
        '#' => {
            stream.eat_while(|ch| !matches!(ch, '>' | ' '));
            "url"
        }
        '>' => "variable",
        '_' => {
            stream.eat_while(|ch| ch != ' ');
            stream.eat(' ');
            "builtin"
        }
        '"' => {
            stream.eat_while(|ch| ch != '"');
            stream.eat('"');
            "string"
        }
        '@' => {
            stream.eat_while(|ch| ch != ' ');
            stream.eat(' ');
            "string.special"
        }
        '^' => {
            stream.next();
            stream.eat_while(|ch| ch != '>');
            stream.eat('>');
            "variable"
        }
        _ => return None,
    })
}
