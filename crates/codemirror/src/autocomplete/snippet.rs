use std::ops::Range;

use crate::{
    Assoc, Change, ChangeDesc, ChangeSet, EditError, EditorSelection, EditorState, MapMode,
    SelectionRange, Transaction,
};

/// A parsed template with named, numbered, or anonymous fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snippet {
    lines: Vec<SnippetLine>,
    stops: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SnippetLine {
    text: String,
    fields: Vec<(usize, Range<usize>)>,
}

#[derive(Debug)]
struct Field<'a> {
    id: usize,
    number: Option<&'a str>,
    name: String,
}

impl Field<'_> {
    fn order(&self) -> (u8, usize, &str) {
        match self.number {
            Some("") => (2, 0, ""),
            Some(number) => (0, number.len(), number),
            None => (1, self.id, ""),
        }
    }
}

impl Snippet {
    /// Parse `${name}`, `${1:default}`, `${1}`, or `${}` (also with `#`).
    /// Repeated numbers or nonempty names link fields. The text at each site
    /// is its own default. Numbered fields come first, then named or anonymous
    /// fields in template order. `${0}` is always last. Escape braces with `\`.
    pub fn parse(template: &str) -> Self {
        let mut fields: Vec<Field<'_>> = Vec::new();
        let mut lines = Vec::new();
        let template = template.replace("\r\n", "\n");
        for mut line in template.split(['\r', '\n']) {
            let mut rendered = SnippetLine {
                text: String::new(),
                fields: Vec::new(),
            };
            while let Some(ch) = line.chars().next() {
                if matches!(ch, '$' | '#')
                    && let Some(body) = line[1..].strip_prefix('{')
                    && let Some((number, raw_name, consumed)) = marker(body)
                {
                    let name = raw_name.replace("\\{", "{").replace("\\}", "}");
                    let number = number.map(|digits| digits.trim_start_matches('0'));
                    let existing = fields.iter().rfind(|field| {
                        if number.is_some() {
                            field.number == number
                        } else {
                            !name.is_empty() && field.name == name
                        }
                    });
                    let start = rendered.text.len();
                    rendered.text.push_str(&name);
                    let id = if let Some(field) = existing {
                        field.id
                    } else {
                        let field = Field {
                            id: fields.len(),
                            number,
                            name,
                        };
                        let index = fields.partition_point(|other| other.order() < field.order());
                        let id = field.id;
                        fields.insert(index, field);
                        id
                    };
                    rendered.fields.push((id, start..rendered.text.len()));
                    line = &body[consumed..];
                } else if ch == '\\' && matches!(line.as_bytes().get(1), Some(b'{' | b'}')) {
                    rendered.text.push(char::from(line.as_bytes()[1]));
                    line = &line[2..];
                } else {
                    rendered.text.push(ch);
                    line = &line[ch.len_utf8()..];
                }
            }
            lines.push(rendered);
        }
        let mut order = vec![0; fields.len()];
        for (index, field) in fields.iter().enumerate() {
            order[field.id] = index;
        }
        for line in &mut lines {
            for (id, _) in &mut line.fields {
                *id = order[*id];
            }
        }
        Self {
            lines,
            stops: fields.len(),
        }
    }

    /// Build a transaction and, for templates with more than one field, a
    /// session in the resulting document. The transaction selects every
    /// occurrence of the first field. If `range.end` is the start of the main
    /// selection, expand the replacement to its end. Discard the session if
    /// a transaction filter changes the insertion.
    ///
    /// Use the insertion line's indentation on each subsequent line, with
    /// one `indent_unit` for each leading tab. Insert line breaks as LF.
    pub fn insert(
        &self,
        state: &EditorState,
        range: Range<usize>,
        indent_unit: &str,
    ) -> Result<(Transaction, Option<SnippetSession>), EditError> {
        self.insert_with_line_ending(state, range, indent_unit, "\n")
    }

    /// Like [`Self::insert`], but use the specified LF or CRLF line ending.
    pub fn insert_with_line_ending(
        &self,
        state: &EditorState,
        mut range: Range<usize>,
        indent_unit: &str,
        line_ending: &str,
    ) -> Result<(Transaction, Option<SnippetSession>), EditError> {
        if !matches!(line_ending, "\n" | "\r\n") {
            return Err(EditError::InvalidRange);
        }
        let text = state.text();
        text.slice(range.clone()).ok_or(EditError::InvalidRange)?;
        let selected = state.selection().main().range();
        if range.end == selected.start {
            range.end = selected.end;
        }
        let line = text.line_at(range.start).ok_or(EditError::InvalidRange)?;
        let base =
            &line.text[..line.text.len() - line.text.trim_start_matches(js_whitespace).len()];
        let mut fields = vec![Vec::new(); self.stops];
        let mut output = String::new();
        for (index, line) in self.lines.iter().enumerate() {
            if index > 0 {
                output.push_str(line_ending);
            }
            let start = range
                .start
                .checked_add(output.len())
                .ok_or(EditError::InvalidRange)?;
            let (base, tabs) = if index == 0 {
                ("", 0)
            } else {
                (
                    base,
                    line.text.bytes().take_while(|&byte| byte == b'\t').count(),
                )
            };
            output.push_str(base);
            for _ in 0..tabs {
                output.push_str(indent_unit);
            }
            output.push_str(&line.text[tabs..]);
            range
                .start
                .checked_add(output.len())
                .ok_or(EditError::InvalidRange)?;
            let position = |pos: usize| {
                start + base.len() + pos.min(tabs) * indent_unit.len() + pos.saturating_sub(tabs)
            };
            for (field, site) in &line.fields {
                fields[*field].push(position(site.start)..position(site.end));
            }
        }
        let changes = ChangeSet::single(Change::new(range, output))?;
        let length = changes.desc(text.len())?.new_length();
        let mut transaction = Transaction::new(changes).with_scroll_into_view();
        if let Some(first) = fields.first() {
            transaction = transaction.with_selection(field_selection(first));
        }
        let session = (fields.len() > 1).then_some(SnippetSession {
            fields,
            active: Some(0),
            length,
        });
        Ok((transaction, session))
    }
}

fn js_whitespace(ch: char) -> bool {
    ch == '\u{feff}' || (ch != '\u{85}' && ch.is_whitespace())
}

// Input starts after `${` or `#{`. The returned length includes the closing brace.
fn marker(body: &str) -> Option<(Option<&str>, &str, usize)> {
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 {
        let number = Some(&body[..digits]);
        let tail = &body[digits..];
        if tail.starts_with('}') {
            return Some((number, "", digits + 1));
        }
        if let Some(name) = tail.strip_prefix(':')
            && let Some(end) = name.find(['{', '}'])
            && name.as_bytes()[end] == b'}'
        {
            return Some((number, &name[..end], digits + end + 2));
        }
    }
    let mut bytes = body.bytes().enumerate().peekable();
    let mut escaped_end = None;
    while let Some((index, byte)) = bytes.next() {
        match byte {
            b'{' => break,
            b'}' => return Some((None, &body[..index], index + 1)),
            b'\\' => match bytes.peek() {
                Some(&(at, b'}')) => {
                    escaped_end = Some(at);
                    bytes.next();
                }
                Some((_, b'{')) => {
                    bytes.next();
                }
                _ => {}
            },
            _ => {}
        }
    }
    escaped_end.map(|end| (None, &body[..end], end + 1))
}

/// Byte ranges in the document after inserting a snippet. Each field can
/// have multiple occurrences. Select all occurrences to edit them together
/// with [`EditorState::replace_selection`]. Single-range edits are not mirrored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnippetSession {
    fields: Vec<Vec<Range<usize>>>,
    active: Option<usize>,
    length: usize,
}

impl SnippetSession {
    /// Field occurrences in tab order. Empty after the session ends.
    pub fn fields(&self) -> &[Vec<Range<usize>>] {
        &self.fields
    }

    /// A zero-based index into [`Self::fields`], or `None` after the session ends.
    pub fn active_field(&self) -> Option<usize> {
        self.active
    }

    /// Select the next field. Moving to the last field ends the session.
    pub fn next_field(&mut self) -> Option<EditorSelection> {
        let next = self.active? + 1;
        let selection = field_selection(self.fields.get(next)?);
        if next + 1 == self.fields.len() {
            self.clear();
        } else {
            self.active = Some(next);
        }
        Some(selection)
    }

    /// Select the previous field, or return `None` if already at the first.
    pub fn previous_field(&mut self) -> Option<EditorSelection> {
        let previous = self.active?.checked_sub(1)?;
        self.active = Some(previous);
        Some(field_selection(&self.fields[previous]))
    }

    pub fn clear(&mut self) {
        self.active = None;
        self.fields.clear();
    }

    /// Map ranges through an applied change description. Pass the explicit
    /// selection, if any, in the new document's coordinates. An edit that
    /// deletes a field boundary or a selection outside the active field ends
    /// the session. Return whether the session is still active.
    pub fn update(
        &mut self,
        changes: &ChangeDesc,
        selection: Option<&EditorSelection>,
    ) -> Result<bool, EditError> {
        let Some(active) = self.active else {
            return Ok(false);
        };
        if changes.length() != self.length {
            return Err(EditError::MismatchedLength);
        }
        if !changes.is_empty() {
            let map = |range: &Range<usize>| {
                let start = changes.map_pos(range.start, Assoc::Before, MapMode::TrackDel)?;
                let end = changes.map_pos(range.end, Assoc::After, MapMode::TrackDel)?;
                Some(start..end)
            };
            let fields: Option<Vec<Vec<_>>> = self
                .fields
                .iter()
                .map(|field| field.iter().map(map).collect())
                .collect();
            let Some(fields) = fields else {
                self.clear();
                return Ok(false);
            };
            self.fields = fields;
            self.length = changes.new_length();
        }
        if let Some(selection) = selection
            && !selection.ranges().iter().all(|selected| {
                let selected = selected.range();
                self.fields[active]
                    .iter()
                    .any(|field| field.start <= selected.start && field.end >= selected.end)
            })
        {
            self.clear();
            return Ok(false);
        }
        Ok(true)
    }
}

fn field_selection(sites: &[Range<usize>]) -> EditorSelection {
    let ranges = sites
        .iter()
        .map(|range| SelectionRange::new(range.start, range.end))
        .collect();
    EditorSelection::new(ranges, 0).expect("each field has at least one occurrence")
}
