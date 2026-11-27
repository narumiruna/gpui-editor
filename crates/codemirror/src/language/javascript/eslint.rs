use std::{ops::Range, sync::Arc};

use serde_json::{Value, json};

use crate::{
    Change, ChangeSet, EditError, EditorState, Transaction,
    language::line_projection::{NormalizedInput, splits_crlf},
    language_data::NativeLanguage,
    lint::{Action, Diagnostic, LintSource, Severity},
};

/// One host finding. Lines and UTF-16 columns start at one in the input to the
/// verifier. An absent end line or an end column of one gives a point.
#[derive(Clone, Debug)]
pub struct EslintFinding {
    pub line: usize,
    pub column: usize,
    pub end_line: Option<usize>,
    pub end_column: Option<usize>,
    pub severity: u8,
    pub message: String,
    pub rule_id: Option<String>,
    pub fix: Option<EslintFix>,
}

/// One replacement in zero-based UTF-16 units relative to the verifier input.
#[derive(Clone, Debug)]
pub struct EslintFix {
    pub range: Range<usize>,
    pub text: String,
}

type Regions = dyn Fn(&EditorState) -> Vec<(NativeLanguage, Range<usize>)> + Send + Sync;

/// Adapt trusted, pure host callbacks to a native lint source. This function
/// does not start ESLint or execute the input. The host must supply the verifier
/// and current parser regions as nonoverlapping byte ranges in source order.
/// Only regions marked [`NativeLanguage::Javascript`] run the verifier. This
/// includes all four dialects. Use separate ranges for disjoint overlay parts.
///
/// The verifier sees LF for each CR, LF, or CRLF break. Fix text is used as
/// supplied. Invalid finding ranges are skipped; an invalid fix does not remove
/// its finding. A fix follows the current diagnostic start if the captured text
/// still fits in a JavaScript region and has not changed. Apply the returned
/// transaction through the normal filters and any host-specific edit checks.
///
/// A supplied JSON configuration passes through unchanged. With no
/// configuration, `recommended_rules` supplies rule IDs once at construction.
/// The default enables these rules as errors and uses a 2019 module parser.
pub fn es_lint(
    regions: impl Fn(&EditorState) -> Vec<(NativeLanguage, Range<usize>)> + Send + Sync + 'static,
    verify: impl Fn(&str, &Value) -> Vec<EslintFinding> + Send + Sync + 'static,
    config: Option<Value>,
    recommended_rules: impl FnOnce() -> Vec<String>,
) -> LintSource {
    let config = config.unwrap_or_else(|| {
        let rules: serde_json::Map<_, _> = recommended_rules()
            .into_iter()
            .map(|id| (id, Value::from(2)))
            .collect();
        json!({
            "parserOptions": {"ecmaVersion": 2019, "sourceType": "module"},
            "env": {"browser": true, "node": true, "es6": true, "es2015": true,
                    "es2017": true, "es2020": true},
            "rules": rules,
        })
    });
    let regions: Arc<Regions> = Arc::new(regions);
    LintSource::new(move |state| {
        let source = state.text().as_str();
        let mut diagnostics = Vec::new();
        for (language, scope) in regions(state) {
            if !matches!(language, NativeLanguage::Javascript(_)) {
                continue;
            }
            let Some(raw) = source_slice(source, scope.clone()) else {
                continue;
            };
            let input = NormalizedInput::new(raw);
            let findings = verify(&input.text, &config);
            if findings.is_empty() {
                continue;
            }
            let coordinates = Coordinates::new(input);
            diagnostics.extend(
                findings.into_iter().filter_map(|finding| {
                    diagnostic(finding, &coordinates, source, &scope, &regions)
                }),
            );
        }
        diagnostics
    })
}

fn diagnostic(
    finding: EslintFinding,
    coordinates: &Coordinates<'_>,
    source: &str,
    scope: &Range<usize>,
    regions: &Arc<Regions>,
) -> Option<Diagnostic> {
    let from = coordinates.position(finding.line, finding.column)?;
    let to = match (finding.end_line, finding.end_column) {
        (None, _) | (_, Some(1)) => from,
        (Some(line), Some(column)) => coordinates.position(line, column)?,
        (Some(_), None) => return None,
    };
    let range = coordinates.range(from..to, scope.start)?;
    let severity = if finding.severity == 1 {
        Severity::Warning
    } else {
        Severity::Error
    };
    let mut diagnostic = Diagnostic::new(range, severity, finding.message);
    diagnostic.source = Some(match finding.rule_id.filter(|id| !id.is_empty()) {
        Some(id) => format!("eslint:{id}"),
        None => "eslint".into(),
    });
    if let Some(fix) = finding.fix
        && let Some(range) = coordinates.range(fix.range, scope.start)
    {
        diagnostic.actions.push(fix_action(
            source,
            scope,
            &diagnostic.range,
            range,
            fix.text,
            Arc::clone(regions),
        ));
    }
    Some(diagnostic)
}

fn fix_action(
    source: &str,
    scope: &Range<usize>,
    diagnostic: &Range<usize>,
    fix: Range<usize>,
    insert: String,
    regions: Arc<Regions>,
) -> Action {
    let mut guard = diagnostic.start.min(fix.start)..diagnostic.end.max(fix.end);
    if guard.is_empty() {
        // A point insertion has no old text. Check the adjacent characters too.
        if let Some(ch) = source[scope.start..guard.start].chars().next_back() {
            let start = guard.start - ch.len_utf8();
            guard.start = start - usize::from(splits_crlf(source, start));
        }
        if let Some(ch) = source[guard.end..scope.end].chars().next() {
            let end = guard.end + ch.len_utf8();
            guard.end = end + usize::from(splits_crlf(source, end));
        }
    }
    let expected = source[guard.start..guard.end].to_owned();
    let anchor = diagnostic.start - guard.start;
    let replace = fix.start - guard.start..fix.end - guard.start;
    Action::new("fix", move |state, current| {
        let invalid = EditError::InvalidRange;
        let source = state.text().as_str();
        let from = current.start.checked_sub(anchor).ok_or(invalid)?;
        let to = from.checked_add(expected.len()).ok_or(invalid)?;
        // The same bytes between the anchor and the fix preserve the UTF-16
        // distance, including any projected CRLF. Do not guess at stale edits.
        if state.read_only()
            || current.end > to
            || source_slice(source, from..to) != Some(expected.as_str())
            || !regions(state).into_iter().any(|(language, scope)| {
                matches!(language, NativeLanguage::Javascript(_))
                    && scope.start <= from
                    && to <= scope.end
                    && source_slice(source, scope).is_some()
            })
        {
            return Err(invalid);
        }
        let range = from + replace.start..from + replace.end;
        let changes = ChangeSet::single(Change::new(range, &insert))?;
        Ok(Transaction::new(changes).with_scroll_into_view())
    })
}

fn source_slice(source: &str, range: Range<usize>) -> Option<&str> {
    let slice = source.get(range.clone())?;
    (!splits_crlf(source, range.start) && !splits_crlf(source, range.end)).then_some(slice)
}

struct Coordinates<'a> {
    input: NormalizedInput<'a>,
    lines: Vec<usize>,
    // End positions in UTF-16 and normalized bytes, and whether a surrogate
    // pair precedes the endpoint. ASCII runs need no entries.
    wide: Vec<(usize, usize, bool)>,
    units: usize,
}

impl<'a> Coordinates<'a> {
    fn new(input: NormalizedInput<'a>) -> Self {
        let (mut lines, mut wide, mut units) = (vec![0], Vec::new(), 0);
        for (at, ch) in input.text.char_indices() {
            let width = ch.len_utf16();
            units += width;
            if ch == '\n' {
                lines.push(units);
            } else if !ch.is_ascii() {
                wide.push((units, at + ch.len_utf8(), width == 2));
            }
        }
        Self {
            input,
            lines,
            wide,
            units,
        }
    }

    fn position(&self, line: usize, column: usize) -> Option<usize> {
        let &start = self.lines.get(line.checked_sub(1)?)?;
        let end = self.lines.get(line).map_or(self.units, |next| next - 1);
        let at = start.checked_add(column.checked_sub(1)?)?;
        (at <= end).then_some(at)
    }

    fn range(&self, units: Range<usize>, start: usize) -> Option<Range<usize>> {
        if units.start > units.end {
            return None;
        }
        Some(start + self.byte(units.start)?..start + self.byte(units.end)?)
    }

    fn byte(&self, at: usize) -> Option<usize> {
        if at > self.units {
            return None;
        }
        let index = self.wide.partition_point(|&(end, _, _)| end <= at);
        if self
            .wide
            .get(index)
            .is_some_and(|&(end, _, pair)| pair && end - 1 == at)
        {
            return None;
        }
        let (units, bytes, _) = index.checked_sub(1).map_or((0, 0, false), |i| self.wide[i]);
        Some(self.input.original_byte(bytes + (at - units)))
    }
}
