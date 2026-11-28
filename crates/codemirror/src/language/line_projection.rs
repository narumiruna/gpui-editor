//! CodeMirror treats LF, CRLF, and lone CR as logical line breaks.

use std::{
    borrow::Cow,
    sync::{Arc, LazyLock},
};

use crate::{
    ChangeSet, ConfigurationError, EditorState, Extension, Facet, LINE_SEPARATOR, StateContext,
    Text,
};

static EDITOR_LINES: LazyLock<Facet<Option<Text>, Option<Text>>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&Option<Text>]| inputs.first().and_then(|text| (*text).clone()))
});

static DEFAULT_EDITOR_LINES: LazyLock<Facet<bool, bool>> =
    LazyLock::new(|| Facet::new(|inputs: &[&bool]| inputs.iter().any(|&&enabled| enabled)));

static EDITOR_LINE_EXTENSION: LazyLock<Extension> = LazyLock::new(|| {
    Extension::group([
        DEFAULT_EDITOR_LINES.compute_with_dependencies(
            |dependencies| dependencies.facet(&LINE_SEPARATOR.reader()),
            |state| {
                let separator = state.facet(&LINE_SEPARATOR)?;
                Ok(separator.as_ref().as_deref().is_none_or(str::is_empty))
            },
        ),
        EDITOR_LINES.compute_with_dependencies(
            |dependencies| {
                dependencies.document();
                dependencies.facet(&DEFAULT_EDITOR_LINES);
            },
            |state| {
                Ok(if uses_default_editor_lines(state)? {
                    editor_lines(state.text().as_str())
                } else {
                    None
                })
            },
        ),
    ])
});

pub(crate) fn editor_line_extension() -> Extension {
    EDITOR_LINE_EXTENSION.clone()
}

/// Whether the editor line view uses default logical breaks, even if the current
/// document has no lone CR and does not need a separate line view.
pub(crate) fn uses_default_editor_lines(
    state: &StateContext<'_>,
) -> Result<bool, ConfigurationError> {
    state.facet(&DEFAULT_EDITOR_LINES).map(|enabled| *enabled)
}

/// The preset line view for this document; the cached result survives
/// selection changes and other transactions that leave the document intact.
pub(crate) fn configured_editor_lines(
    state: &StateContext<'_>,
) -> Result<Arc<Option<Text>>, ConfigurationError> {
    state.facet(&EDITOR_LINES)
}

pub(crate) fn command_line_projection(state: &EditorState) -> Option<Text> {
    configured_editor_lines(&state.context())
        .expect("valid editor line projection")
        .as_ref()
        .clone()
}

/// Line queries need original byte offsets. Replace only the lone CR bytes.
pub(crate) fn editor_lines(source: &str) -> Option<Text> {
    let mut breaks = source
        .match_indices('\r')
        .map(|(at, _)| at)
        .filter(|&at| source.as_bytes().get(at + 1) != Some(&b'\n'))
        .peekable();
    breaks.peek()?;
    let mut output = String::with_capacity(source.len());
    let mut from = 0;
    for at in breaks {
        output.push_str(&source[from..at]);
        output.push('\n');
        from = at + 1;
    }
    output.push_str(&source[from..]);
    Some(Text::new(output))
}

pub(crate) fn splits_crlf(source: &str, at: usize) -> bool {
    at > 0 && source.as_bytes().get(at - 1..=at) == Some(b"\r\n")
}

/// Check the same-width representation used for editor line bounds.
pub(crate) fn is_editor_line_view(source: &str, projected: &Text) -> bool {
    let bytes = source.as_bytes();
    bytes.len() == projected.len()
        && bytes
            .iter()
            .zip(projected.as_str().bytes())
            .enumerate()
            .all(|(at, (&before, after))| {
                let expected = if before == b'\r' && bytes.get(at + 1) != Some(&b'\n') {
                    b'\n'
                } else {
                    before
                };
                after == expected
            })
}

/// Check that an edit at logical line bounds does not join separate breaks.
/// The caller must validate the edit ranges and avoid splitting existing CRLF.
pub(crate) fn keeps_separate_breaks(source: &str, changes: &ChangeSet) -> bool {
    let mut ends_in_cr = false;
    let mut accept = |part: &str| {
        if part.is_empty() {
            return true;
        }
        let joined = ends_in_cr && part.starts_with('\n');
        ends_in_cr = part.ends_with('\r');
        !joined
    };
    let mut from = 0;
    for (range, insert) in changes.changes() {
        if !accept(&source[from..range.start]) || !accept(insert) {
            return false;
        }
        from = range.end;
    }
    accept(&source[from..])
}

/// Parse with one LF per break. Keep the original and normalized positions of
/// each CRLF pair so syntax ranges and checkpoints still use original bytes.
pub(super) struct NormalizedInput<'a> {
    pub(super) text: Cow<'a, str>,
    pub(super) offsets: NormalizedOffsets,
}

#[derive(Debug, Default)]
pub(super) struct NormalizedOffsets(Vec<(usize, usize)>);

impl<'a> NormalizedInput<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        let mut breaks = source.match_indices('\r').peekable();
        let mut crlf = Vec::new();
        if breaks.peek().is_none() {
            return Self {
                text: Cow::Borrowed(source),
                offsets: NormalizedOffsets::default(),
            };
        }
        let mut text = String::with_capacity(source.len());
        let mut from = 0;
        for (at, _) in breaks {
            text.push_str(&source[from..at]);
            let pair = source.as_bytes().get(at + 1) == Some(&b'\n');
            if pair {
                crlf.push((at, text.len()));
            }
            text.push('\n');
            from = at + 1 + usize::from(pair);
        }
        text.push_str(&source[from..]);
        Self {
            text: Cow::Owned(text),
            offsets: NormalizedOffsets(crlf),
        }
    }

    pub(super) fn original_byte(&self, at: usize) -> usize {
        self.offsets.original_byte(at)
    }

    pub(super) fn normalized_byte(&self, at: usize) -> usize {
        self.offsets.normalized_byte(at)
    }
}

impl NormalizedOffsets {
    pub(super) fn original_byte(&self, at: usize) -> usize {
        at + self.0.partition_point(|&(_, lf)| lf < at)
    }

    pub(super) fn normalized_byte(&self, at: usize) -> usize {
        at - self.0.partition_point(|&(cr, _)| cr < at)
    }
}

/// Changes in the difference between original bytes and default CodeMirror
/// UTF-16 positions. Store only the ends of characters and line endings that
/// change the difference. A snapshot builds this index only for a predicate.
#[derive(Debug)]
pub(in crate::language) struct EditorOffsets(Box<[(usize, usize)]>);

impl EditorOffsets {
    pub(in crate::language) fn new(source: &str) -> Self {
        let mut offsets = Vec::new();
        let mut delta = 0;
        for (at, ch) in source.char_indices() {
            let width = ch.len_utf8();
            let crlf = usize::from(ch == '\r' && source.as_bytes().get(at + 1) == Some(&b'\n'));
            let removed = width - ch.len_utf16() + crlf;
            if removed > 0 {
                delta += removed;
                let end = at + width + crlf;
                offsets.push((end, end - delta));
            }
        }
        Self(offsets.into_boxed_slice())
    }

    pub(in crate::language) fn editor(&self, byte: usize) -> usize {
        let i = self.0.partition_point(|&(end, _)| end <= byte);
        byte - self.delta(i)
    }

    pub(in crate::language) fn source(&self, editor: usize) -> usize {
        let i = self.0.partition_point(|&(_, end)| end <= editor);
        // Between two surrogates this yields a byte inside the scalar. Only
        // compare that byte to node bounds; never use it to slice the text.
        editor + self.delta(i)
    }

    fn delta(&self, i: usize) -> usize {
        i.checked_sub(1).map_or(0, |i| {
            let (byte, editor) = self.0[i];
            byte - editor
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_projection_is_reused_until_document_change() {
        use crate::{Change, EditorSelection, Transaction};
        let mut state = EditorState::with_extensions("a\rb", editor_line_extension()).unwrap();
        let initial = configured_editor_lines(&state.context()).unwrap();
        assert_eq!(initial.as_ref().as_ref().unwrap().as_str(), "a\nb");
        state
            .update(Transaction::default().with_selection(EditorSelection::single(2, 2)))
            .unwrap();
        let selected = configured_editor_lines(&state.context()).unwrap();
        assert!(Arc::ptr_eq(&initial, &selected));
        state
            .update(Transaction::new(
                ChangeSet::single(Change::new(2..2, "\n")).unwrap(),
            ))
            .unwrap();
        assert_eq!(state.text().as_str(), "a\r\nb");
        let edited = configured_editor_lines(&state.context()).unwrap();
        assert!(edited.is_none());
        assert!(!Arc::ptr_eq(&initial, &edited));
    }

    #[test]
    fn an_explicit_separator_disables_default_line_projection() {
        use crate::{Compartment, Transaction};
        let slot = Compartment::new();
        let mut state = EditorState::with_extensions(
            "a\rb",
            Extension::group([
                editor_line_extension(),
                slot.of(LINE_SEPARATOR.of("\n".into())),
            ]),
        )
        .unwrap();
        assert!(command_line_projection(&state).is_none());
        state
            .update(
                Transaction::default()
                    .with_effects([slot.reconfigure(LINE_SEPARATOR.of(String::new()))]),
            )
            .unwrap();
        assert_eq!(command_line_projection(&state).unwrap().line_count(), 2);
        assert_eq!(state.text().as_str(), "a\rb");
    }

    #[test]
    fn projections_preserve_all_logical_character_boundaries() {
        for raw in ["", "漢", "\r", "\r\r\n", "\r\n\r", "😀\r漢\r\n\r\n\r"] {
            let normalized = NormalizedInput::new(raw);
            let editor = editor_lines(raw).unwrap_or_else(|| Text::new(raw));
            assert_eq!(editor.len(), raw.len());
            assert_eq!(
                normalized.text,
                raw.replace("\r\n", "\n").replace('\r', "\n")
            );
            for byte in 0..=raw.len() {
                if !raw.is_char_boundary(byte)
                    || (byte > 0 && raw.as_bytes().get(byte - 1..byte + 1) == Some(b"\r\n"))
                {
                    continue;
                }
                let normalized_byte = normalized.normalized_byte(byte);
                assert!(normalized.text.is_char_boundary(normalized_byte));
                assert_eq!(normalized.original_byte(normalized_byte), byte);
            }
        }
    }
}
