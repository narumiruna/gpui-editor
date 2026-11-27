use super::{eat_str, word};
use crate::stream_parser::StringStream;

pub(super) fn token(stream: &mut StringStream<'_>) -> Option<&'static str> {
    if stream.eat_space() {
        return None;
    }
    let sol = stream.sol();
    let ch = stream.next()?;

    if ch == '\\' {
        if ["fB", "fR", "fI", "u", "d", "%", "&"]
            .iter()
            .any(|text| eat_str(stream, text))
        {
            return Some("string");
        }
        if eat_str(stream, "m[") {
            stream.skip_to("]");
            stream.next();
        } else if eat_str(stream, "s+") || eat_str(stream, "s-") {
            stream.eat_while(|ch| ch.is_ascii_digit() || ch == '-');
        } else if eat_str(stream, "(") || eat_str(stream, "*(") {
            stream.eat_while(word_or_hyphen);
        }
        return Some("string");
    }
    if sol && matches!(ch, '.' | '\'') && stream.eat('\\').is_some() && stream.eat('"').is_some() {
        stream.skip_to_end();
        return Some("comment");
    }
    if sol && ch == '.' {
        if ["B ", "I ", "R "].iter().any(|text| eat_str(stream, text)) {
            return Some("attribute");
        }
        if ["TH ", "SH ", "SS ", "HP "]
            .iter()
            .any(|text| eat_str(stream, text))
        {
            stream.skip_to_end();
            return Some("quote");
        }
        // Each test consumes one letter, even when the next test fails.
        if pair(stream, |ch| ch.is_ascii_uppercase()) || pair(stream, |ch| ch.is_ascii_lowercase())
        {
            return Some("attribute");
        }
    }
    stream.eat_while(word_or_hyphen);
    None
}

fn pair(stream: &mut StringStream<'_>, matches: impl Fn(char) -> bool) -> bool {
    stream.eat_if(&matches).is_some() && stream.eat_if(matches).is_some()
}

fn word_or_hyphen(ch: char) -> bool {
    word(ch) || ch == '-'
}
