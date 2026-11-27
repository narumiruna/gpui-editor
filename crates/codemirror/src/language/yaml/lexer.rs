// Scalar and indentation rules adapted from @lezer/yaml 1.0.4.

pub(super) struct Source<'a>(pub &'a str);

impl Source<'_> {
    pub fn byte(&self, at: usize) -> u8 {
        self.0.as_bytes().get(at).copied().unwrap_or(0)
    }

    pub fn eof(&self, at: usize) -> bool {
        at == self.0.len()
    }

    pub fn next(&self, at: usize) -> usize {
        at + self.0[at..].chars().next().map_or(0, char::len_utf8)
    }

    pub fn separated(&self, at: usize) -> bool {
        self.eof(at) || space(self.byte(at))
    }

    pub fn mark(&self, at: usize, mark: u8) -> bool {
        self.byte(at) == mark && self.separated(at + 1)
    }

    pub fn doc_mark(&self, at: usize, mark: &str) -> bool {
        (at == 0 || matches!(self.byte(at - 1), b'\r' | b'\n'))
            && self.0[at..].starts_with(mark)
            && self.separated(at + 3)
    }

    pub fn line_start(&self, at: usize) -> usize {
        self.0[..at].rfind(['\r', '\n']).map_or(0, |at| at + 1)
    }

    pub fn column(&self, at: usize) -> usize {
        self.0[self.line_start(at)..at].chars().count()
    }

    pub fn line_end(&self, at: usize) -> usize {
        self.0[at..]
            .find(['\r', '\n'])
            .map_or(self.0.len(), |end| at + end)
    }

    pub fn after_break(&self, at: usize) -> usize {
        match (self.byte(at), self.byte(at + 1)) {
            (b'\r', b'\n') => at + 2,
            (b'\r' | b'\n', _) => at + 1,
            _ => at,
        }
    }

    pub fn inline_space(&self, mut at: usize) -> usize {
        while matches!(self.byte(at), b' ' | b'\t') {
            at += 1;
        }
        at
    }

    pub fn quoted(&self, at: usize, scan: bool) -> Option<usize> {
        let quote = self.byte(at);
        let mut pos = at + 1;
        while !self.eof(pos) {
            match self.byte(pos) {
                ch if ch == quote => {
                    pos += 1;
                    if quote != b'\'' || self.byte(pos) != b'\'' {
                        return Some(pos);
                    }
                    pos += 1;
                }
                b'\\' if quote == b'"' => {
                    let escaped = self.after_break(pos + 1);
                    pos = if escaped > pos + 1 && !scan {
                        escaped
                    } else {
                        self.next(pos + 1)
                    };
                }
                b'\r' | b'\n' if scan => return None,
                _ => pos = self.next(pos),
            }
            if scan && pos - at >= 1024 {
                return None;
            }
        }
        Some(pos)
    }

    pub fn anchor(&self, mut at: usize) -> usize {
        at += 1;
        while !self.separated(at) && char_tag(self.byte(at)) != b'f' {
            at = self.next(at);
        }
        at
    }

    pub fn tag(&self, at: usize) -> usize {
        let quoted = self.byte(at + 1) == b'<';
        let mut pos = at + if quoted { 2 } else { 1 };
        loop {
            let byte = self.byte(pos);
            match byte {
                b'%' => {
                    pos += 1;
                    for _ in 0..2 {
                        if self.byte(pos).is_ascii_hexdigit() {
                            pos += 1;
                        }
                    }
                }
                33..=126 if !b"\",<>\\^`{|}".contains(&byte) || quoted && byte == b',' => {
                    pos += 1;
                }
                b'>' if quoted => return pos + 1,
                _ => return pos,
            }
        }
    }

    pub fn plain(&self, at: usize, scan: bool, flow: bool, indent: Option<usize>) -> Option<usize> {
        let first = self.byte(at);
        if !(char_tag(first) == b's'
            || matches!(first, b'?' | b':' | b'-') && safe(self.byte(at + 1), flow))
        {
            return None;
        }
        let mut end = self.next(at);
        loop {
            let mut next = end;
            let mut column = None;
            while space(self.byte(next)) {
                if matches!(self.byte(next), b'\r' | b'\n') {
                    if scan {
                        return Some(end);
                    }
                    column = Some(0);
                } else if let Some(col) = column.as_mut() {
                    *col += 1;
                }
                next += 1;
            }
            let ch = self.byte(next);
            let allowed = match ch {
                b':' => safe(self.byte(next + 1), flow),
                b'#' => self.byte(next - 1) != b' ',
                _ => safe(ch, flow),
            };
            if !allowed
                || !flow
                    && (column
                        .zip(indent)
                        .is_some_and(|(col, indent)| col <= indent)
                        || column == Some(0)
                            && (self.doc_mark(next, "---") || self.doc_mark(next, "...")))
            {
                return Some(end);
            }
            if scan && (char_tag(ch) == b'f' || next - at > 1024) {
                return None;
            }
            end = self.next(next);
        }
    }

    pub fn key_colon(&self, at: usize) -> Option<usize> {
        let mut pos = at;
        loop {
            pos = match self.byte(pos) {
                b'!' => self.inline_space(self.tag(pos)),
                b'&' => self.inline_space(self.anchor(pos)),
                _ => break,
            };
        }
        pos = match self.byte(pos) {
            b'*' => self.anchor(pos),
            b'\'' | b'"' => self.quoted(pos, true)?,
            b'[' | b'{' => self.scan_brackets(pos)?,
            b':' if self.separated(pos + 1) => pos,
            _ => self.plain(pos, true, false, None)?,
        };
        pos = self.inline_space(pos);
        self.mark(pos, b':').then_some(pos)
    }

    fn scan_brackets(&self, mut at: usize) -> Option<usize> {
        let start = at;
        let mut stack = Vec::new();
        loop {
            match self.byte(at) {
                b'[' => stack.push(b']'),
                b'{' => stack.push(b'}'),
                b']' | b'}' => {
                    if stack.pop()? != self.byte(at) {
                        return None;
                    }
                    if stack.is_empty() {
                        return Some(at + 1);
                    }
                }
                b'\'' | b'"' => {
                    at = self.quoted(at, true)?;
                    continue;
                }
                0 | b'\r' | b'\n' => return None,
                _ => {}
            }
            if at - start > 1024 {
                return None;
            }
            at = self.next(at);
        }
    }

    pub fn block_content(&self, mut at: usize, minimum: usize, explicit: Option<usize>) -> usize {
        let mut required = explicit;
        let mut end = at;
        while !self.eof(at) {
            let line_end = self.line_end(at);
            let depth = self.0.as_bytes()[at..line_end]
                .iter()
                .take_while(|&&ch| ch == b' ')
                .count();
            if self.doc_mark(at, "---") || self.doc_mark(at, "...") {
                break;
            }
            if depth < line_end - at || line_end == self.0.len() {
                let indent = *required.get_or_insert_with(|| minimum.max(depth));
                if depth < indent {
                    break;
                }
            }
            if line_end > at {
                end = line_end;
            }
            at = self.after_break(line_end);
        }
        end
    }
}

pub(super) fn space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n')
}

// Safe, indicator, and flow-indicator characters from ASCII '!' to '}'.
const CHAR_TABLE: &[u8] = b"iiisiiissisfissssssssssssisssiiissssssssssssssssssssssssssfsfssissssssssssssssssssssssssssfif";

fn char_tag(byte: u8) -> u8 {
    match byte {
        0..=32 => b'u',
        33..=125 => CHAR_TABLE[usize::from(byte - 33)],
        _ => b's',
    }
}

fn safe(byte: u8, flow: bool) -> bool {
    let tag = char_tag(byte);
    tag != b'u' && !(flow && tag == b'f')
}
