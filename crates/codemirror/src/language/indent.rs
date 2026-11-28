use crate::{Assoc, Line, Text, count_column};

use super::StreamSettings;

/// Customize an indentation query without changing the document.
#[derive(Clone, Copy, Default)]
pub struct IndentOptions<'a> {
    /// Report a replacement indentation column for a line start. `None` keeps
    /// the original column. The parser sees overrides while replaying lines.
    pub override_indentation: Option<&'a dyn Fn(usize) -> Option<usize>>,
    /// Treat this byte position as a logical line break in context queries.
    pub simulate_break: Option<usize>,
    /// At the simulated break, report an empty line between the two parts.
    pub simulate_double_break: bool,
}

/// Document and line information available to a context-aware indent function.
pub struct IndentContext<'a> {
    text: &'a Text,
    position: usize,
    settings: StreamSettings,
    options: IndentOptions<'a>,
}

impl<'a> IndentContext<'a> {
    pub(super) fn new(
        text: &'a Text,
        position: usize,
        settings: StreamSettings,
        mut options: IndentOptions<'a>,
    ) -> Option<Self> {
        if !text.as_str().is_char_boundary(position) {
            return None;
        }
        options.simulate_break = options
            .simulate_break
            .filter(|&point| text.as_str().is_char_boundary(point));
        Some(Self {
            text,
            position,
            settings,
            options,
        })
    }

    pub fn text(&self) -> &Text {
        self.text
    }

    pub fn position(&self) -> usize {
        self.position
    }

    pub fn unit(&self) -> usize {
        self.settings.indent_unit
    }

    pub fn settings(&self) -> StreamSettings {
        self.settings
    }

    pub(crate) fn options(&self) -> IndentOptions<'a> {
        self.options
    }

    pub fn simulated_break(&self) -> Option<usize> {
        self.options.simulate_break
    }

    pub(super) fn at_double_break(&self) -> bool {
        self.options.simulate_double_break && self.simulated_break() == Some(self.position)
    }

    /// Get the logical line or the part of it on the selected side of a
    /// simulated break. Bias has no effect away from the break position.
    pub fn line_at(&self, position: usize, bias: Assoc) -> Option<Line<'a>> {
        let line = self.text.line_at(position)?;
        let Some(point) = self
            .simulated_break()
            .filter(|point| (line.range.start..=line.range.end).contains(point))
        else {
            return Some(line);
        };
        let range = if self.options.simulate_double_break && point == position {
            point..point
        } else if point < position || (point == position && bias == Assoc::After) {
            point..line.range.end
        } else {
            line.range.start..point
        };
        Some(Line {
            text: self.text.slice(range.clone())?,
            range,
            number: line.number,
        })
    }

    /// Get up to 100 Unicode scalars after the position on its logical line.
    pub fn text_after_pos(&self, position: usize, bias: Assoc) -> Option<&'a str> {
        let line = self.line_at(position, bias)?;
        let rest = &line.text[(position - line.range.start).min(line.text.len())..];
        let end = rest
            .char_indices()
            .nth(100)
            .map_or(rest.len(), |(end, _)| end);
        Some(&rest[..end])
    }

    /// Get the indentation column for the line, including an override if set.
    pub fn line_indent(&self, position: usize, bias: Assoc) -> Option<usize> {
        let line = self.line_at(position, bias)?;
        self.overridden(line.range.start)
            .or_else(|| self.count_indent(line.text))
    }

    /// Get the column at a position, adjusted for an indentation override.
    /// An override can put a position within the old indentation before column zero.
    pub fn column(&self, position: usize, bias: Assoc) -> Option<isize> {
        let line = self.line_at(position, bias)?;
        let signed = |value| isize::try_from(value).unwrap_or(isize::MAX);
        let column = signed(count_column(
            line.text,
            self.settings.tab_size,
            position - line.range.start,
        )?);
        Some(match self.overridden(line.range.start) {
            Some(override_column) => column
                .saturating_add(signed(override_column) - signed(self.count_indent(line.text)?)),
            None => column,
        })
    }

    fn count_indent(&self, line: &str) -> Option<usize> {
        let end = line
            .find(|ch: char| ch != '\u{feff}' && (!ch.is_whitespace() || ch == '\u{85}'))
            .unwrap_or(line.len());
        count_column(line, self.settings.tab_size, end)
    }

    pub(super) fn overridden(&self, start: usize) -> Option<usize> {
        self.options.override_indentation.and_then(|get| get(start))
    }
}

// Matches the indent callback's `^\s*(.*)` in the stream language adapter.
pub(super) fn indent_text(line: &str) -> &str {
    line.trim_start_matches(|ch: char| (ch.is_whitespace() && ch != '\u{85}') || ch == '\u{feff}')
        .split(['\r', '\n', '\u{2028}', '\u{2029}'])
        .next()
        .unwrap_or_default()
}
