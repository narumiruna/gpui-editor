use std::ops::Range;

/// An editable UTF-8 buffer. LF and CRLF end logical lines.
#[derive(Debug)]
pub struct Text(pub(crate) String);

/// A one-based logical line. The range and text exclude the line ending.
#[derive(Debug, Eq, PartialEq)]
pub struct Line<'a> {
    pub number: usize,
    pub range: Range<usize>,
    pub text: &'a str,
}

impl Text {
    pub fn new(source: impl Into<String>) -> Self {
        Self(source.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn slice(&self, range: Range<usize>) -> Option<&str> {
        self.0.get(range)
    }

    pub fn line(&self, number: usize) -> Option<Line<'_>> {
        let mut start = 0;
        for _ in 1..number {
            start += self.0[start..].find('\n')? + 1;
        }
        (number != 0).then(|| self.line_from(number, start))
    }

    /// The line at a byte boundary. Both bytes of CRLF belong to the preceding line.
    pub fn line_at(&self, position: usize) -> Option<Line<'_>> {
        let prefix = self.0.get(..position)?;
        let start = prefix.rfind('\n').map_or(0, |at| at + 1);
        let number = prefix.bytes().filter(|&byte| byte == b'\n').count() + 1;
        Some(self.line_from(number, start))
    }

    fn line_from(&self, number: usize, start: usize) -> Line<'_> {
        let rest = &self.0[start..];
        let text = rest
            .split_once('\n')
            .map_or(rest, |(line, _)| line.strip_suffix('\r').unwrap_or(line));
        Line {
            number,
            range: start..start + text.len(),
            text,
        }
    }
}
