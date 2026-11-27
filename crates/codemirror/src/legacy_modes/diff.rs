use crate::stream_parser::StringStream;

pub(super) fn token(stream: &mut StringStream<'_>) -> Option<&'static str> {
    let line = stream.line();
    let end = line.trim_end_matches([' ', '\t']).len();
    let prefix = line.as_bytes().first();
    if !stream.sol() || end == 0 {
        stream.skip_to_end();
        Some(match prefix {
            Some(b'+') => "error inserted",
            Some(b'-') => "error deleted",
            Some(b'@') => "error meta",
            _ => "error",
        })
    } else {
        stream.set_pos(end).expect("trim ends at a UTF-8 boundary");
        match prefix {
            Some(b'+') => Some("inserted"),
            Some(b'-') => Some("deleted"),
            Some(b'@') => Some("meta"),
            _ => None,
        }
    }
}
