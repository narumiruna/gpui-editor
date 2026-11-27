use std::{
    ops::Range as ByteRange,
    sync::{Arc, LazyLock},
};

use crate::{
    ConfigurationError, EditError, EditorState, Extension, Facet, Range, RangeSet, StateField,
    language::line_projection::{
        configured_editor_lines, editor_line_extension, uses_default_editor_lines,
    },
    language_data::{NativeSyntax, attached_language_snapshot},
    view::MarkDecoration,
};

use super::{BracketConfig, BracketMatch, bracket_near_with_units};

/// Native renderer for a single match in the current immutable editor state.
/// The host draws the resulting marks. The ranges must use document bytes.
pub type BracketMatchRender =
    Arc<dyn Fn(&BracketMatch, &EditorState) -> Vec<Range<MarkDecoration>> + Send + Sync>;

/// Optional inputs to [`bracket_matching`]. Explicit values for the same
/// option must agree. Callbacks agree only when they share an `Arc`.
#[derive(Clone, Default)]
pub struct BracketMatchingOptions {
    pub after_cursor: Option<bool>,
    pub brackets: Option<Arc<str>>,
    /// Maximum scan distance in UTF-16 units at logical chunk boundaries.
    /// Zero means the stock limit.
    pub max_scan_distance: Option<usize>,
    pub render_match: Option<BracketMatchRender>,
}

impl BracketMatchingOptions {
    /// Options for the match itself, without the renderer.
    pub fn config(&self) -> BracketConfig<'_> {
        BracketConfig {
            brackets: self.brackets.as_deref().unwrap_or(""),
            after_cursor: self.after_cursor.unwrap_or(true),
            max_scan_distance: self.max_scan_distance.unwrap_or(0),
        }
    }
}

type CombinedOptions = Result<BracketMatchingOptions, ConfigurationError>;
static OPTIONS: LazyLock<Facet<BracketMatchingOptions, CombinedOptions>> = LazyLock::new(|| {
    Facet::new(|inputs: &[&BracketMatchingOptions]| {
        let mut options = BracketMatchingOptions::default();
        for input in inputs {
            combine(
                &mut options.after_cursor,
                &input.after_cursor,
                PartialEq::eq,
            )?;
            combine(&mut options.brackets, &input.brackets, PartialEq::eq)?;
            combine(
                &mut options.max_scan_distance,
                &input.max_scan_distance,
                PartialEq::eq,
            )?;
            combine(&mut options.render_match, &input.render_match, Arc::ptr_eq)?;
        }
        Ok(options)
    })
});
static INSTALLED: LazyLock<StateField<BracketMatchingOptions>> = LazyLock::new(|| {
    StateField::new(
        |state| state.facet(&OPTIONS)?.as_ref().clone(),
        |_, update| update.state().facet(&OPTIONS)?.as_ref().clone(),
    )
});

fn combine<T: Clone>(
    target: &mut Option<T>,
    value: &Option<T>,
    equal: impl FnOnce(&T, &T) -> bool,
) -> Result<(), ConfigurationError> {
    if let Some(value) = value {
        if target
            .as_ref()
            .is_some_and(|previous| !equal(previous, value))
        {
            return Err(ConfigurationError::InvalidFieldData);
        }
        *target = Some(value.clone());
    }
    Ok(())
}

/// Install bracket matching for headless view hosts. Keep custom marks, scan
/// policy and matching separate from drawing, focus and IME lifecycle.
pub fn bracket_matching(options: BracketMatchingOptions) -> Extension {
    Extension::group([
        OPTIONS.of(options),
        INSTALLED.extension(),
        editor_line_extension(),
    ])
}

/// Return the combined options when bracket matching is installed.
pub fn bracket_matching_options(
    state: &EditorState,
) -> Result<Option<Arc<BracketMatchingOptions>>, ConfigurationError> {
    state.context().field(&INSTALLED)
}

/// Compute one result per qualifying caret in the installed configuration.
/// `same_syntax` compares the raw byte offsets of two bracket tokens.
pub fn configured_bracket_matches(
    state: &EditorState,
    same_syntax: impl Fn(usize, usize) -> bool,
) -> Result<Vec<BracketMatch>, EditError> {
    configured_bracket_matches_in(state, 0..state.text().len(), same_syntax)
}

/// Like [`configured_bracket_matches`], but skip carets outside the scope and
/// do not scan outside it. The scope and results use original document bytes.
pub fn configured_bracket_matches_in(
    state: &EditorState,
    scope: ByteRange<usize>,
    same_syntax: impl Fn(usize, usize) -> bool,
) -> Result<Vec<BracketMatch>, EditError> {
    if !state.text().is_valid_range(scope.clone()) {
        return Err(EditError::InvalidRange);
    }
    let Some(options) = bracket_matching_options(state)? else {
        return Ok(Vec::new());
    };
    matches(state, &options, scope, same_syntax)
}

fn matches(
    state: &EditorState,
    options: &BracketMatchingOptions,
    scope: ByteRange<usize>,
    same_syntax: impl Fn(usize, usize) -> bool,
) -> Result<Vec<BracketMatch>, EditError> {
    let context = state.context();
    let projection = configured_editor_lines(&context)?;
    let text = projection.as_ref().as_ref().unwrap_or(context.text());
    let logical = uses_default_editor_lines(&context)?;
    let config = options.config();
    Ok(state
        .selection()
        .ranges()
        .iter()
        .filter(|range| range.is_empty() && (scope.start..=scope.end).contains(&range.head()))
        .filter_map(|range| {
            bracket_near_with_units(
                text,
                scope.clone(),
                range.head(),
                config,
                &same_syntax,
                |chunk| {
                    if logical && chunk == "\r\n" {
                        1
                    } else {
                        chunk.encode_utf16().count()
                    }
                },
            )
        })
        .collect())
}

pub(crate) fn state_bracket_matches(state: &EditorState) -> Result<Vec<BracketMatch>, EditError> {
    let Some(options) = bracket_matching_options(state)? else {
        return Ok(Vec::new());
    };
    state_matches(state, &options)
}

fn state_matches(
    state: &EditorState,
    options: &BracketMatchingOptions,
) -> Result<Vec<BracketMatch>, EditError> {
    let attached = attached_language_snapshot(state);
    let syntax = attached.as_deref().map(|doc| doc.syntax());
    let config = options.config();
    match syntax {
        Some(NativeSyntax::Go(doc)) => Ok(match_carets(state, |head| {
            doc.bracket_near_with_config(head, config)
        })),
        Some(NativeSyntax::Json(doc)) => Ok(match_carets(state, |head| {
            doc.bracket_near_with_config(head, config)
        })),
        Some(NativeSyntax::LezerGrammar(doc)) => Ok(match_carets(state, |head| {
            doc.bracket_near_with_config(head, config)
        })),
        Some(NativeSyntax::Wast(doc)) => Ok(match_carets(state, |head| {
            doc.bracket_near_with_config(head, config)
        })),
        _ => matches(state, options, 0..state.text().len(), |_, _| true),
    }
}

fn match_carets(
    state: &EditorState,
    near: impl Fn(usize) -> Option<BracketMatch>,
) -> Vec<BracketMatch> {
    state
        .selection()
        .ranges()
        .iter()
        .filter(|range| range.is_empty())
        .filter_map(|range| near(range.head()))
        .collect()
}

/// Build stock matching or nonmatching marks, or call the configured renderer
/// once per match. Use the supplied plain scan policy even with attached
/// syntax. Reject invalid callback ranges. Does not change the state.
pub fn bracket_match_decorations(
    state: &EditorState,
    same_syntax: impl Fn(usize, usize) -> bool,
) -> Result<RangeSet<MarkDecoration>, EditError> {
    let Some(options) = bracket_matching_options(state)? else {
        return Ok(RangeSet::empty());
    };
    let found = matches(state, &options, 0..state.text().len(), same_syntax)?;
    render_matches(state, &options, found)
}

pub(crate) fn state_bracket_match_decorations(
    state: &EditorState,
) -> Result<RangeSet<MarkDecoration>, EditError> {
    let Some(options) = bracket_matching_options(state)? else {
        return Ok(RangeSet::empty());
    };
    let found = state_matches(state, &options)?;
    render_matches(state, &options, found)
}

fn render_matches(
    state: &EditorState,
    options: &BracketMatchingOptions,
    hits: Vec<BracketMatch>,
) -> Result<RangeSet<MarkDecoration>, EditError> {
    let mut marks = Vec::new();
    for found in hits {
        if let Some(render) = &options.render_match {
            marks.extend(render(&found, state));
        } else {
            let label = if found.matched {
                "cm-matchingBracket"
            } else {
                "cm-nonmatchingBracket"
            };
            let mark = MarkDecoration::new(label);
            marks.push(mark.clone().range(found.start)?);
            if let Some(end) = found.end {
                marks.push(mark.range(end)?);
            }
        }
    }
    if marks
        .iter()
        .any(|mark| mark.from >= mark.to || !state.text().is_valid_range(mark.from..mark.to))
    {
        return Err(EditError::InvalidRange);
    }
    RangeSet::of(marks, true).map_err(|_| EditError::InvalidRange)
}
