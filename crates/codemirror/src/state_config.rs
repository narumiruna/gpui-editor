use std::{
    collections::HashMap,
    num::NonZeroUsize,
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Change, ChangeSet, ChangeSpec, ConfigurationError, EditError, EditorSelection, EditorState,
    Extension, Facet, Precedence, Text, change::ResolvedSpec, extension::InitialConfiguration,
};

/// Whether to retain multiple selection ranges. With explicit inputs, any
/// `true` enables them, even if a `false` has higher precedence. Unlike
/// CodeMirror, the native default with no inputs is `true`.
/// Updates use the start state's policy, even when they reconfigure this facet.
/// This native facet accepts computed inputs, but [`EditorState::create`] and
/// [`EditorState::create_from_text`] require static inputs for the initial policy.
pub static ALLOW_MULTIPLE_SELECTIONS: LazyLock<Facet<bool, bool>> =
    LazyLock::new(|| Facet::new(|values: &[&bool]| values.is_empty() || values.contains(&&true)));

/// The first input sets the separator for explicit string conversion. An
/// absent or empty value selects the default input split (CRLF, LF, or CR) and
/// LF output. It does not change raw text or raw change sets. This native facet
/// accepts computed inputs, but [`EditorState::create`] requires static inputs
/// to parse its initial string.
pub static LINE_SEPARATOR: LazyLock<Facet<String, Option<String>>> =
    LazyLock::new(|| Facet::new(|values: &[&String]| values.first().map(|value| (*value).clone())));

/// Whether commands that check read-only state should refuse edits. The first
/// input wins. This does not block programmatic transactions.
pub static READ_ONLY: LazyLock<Facet<bool, bool>> =
    LazyLock::new(|| Facet::new(|values: &[&bool]| values.first().is_some_and(|value| **value)));

/// The first tab size input wins. The default is four columns.
pub static TAB_SIZE: LazyLock<Facet<NonZeroUsize, NonZeroUsize>> = LazyLock::new(|| {
    Facet::new(|values: &[&NonZeroUsize]| {
        values
            .first()
            .map_or(NonZeroUsize::new(4).unwrap(), |value| **value)
    })
});

// Collaboration uses replacement geometry even when the inserted bytes
// equal the removed bytes. Other native editors keep their no-op policy.
pub(crate) static PRESERVE_REPLACEMENTS: LazyLock<Facet<(), bool>> =
    LazyLock::new(|| Facet::new(|inputs: &[&()]| !inputs.is_empty()));

pub type PhraseMap = HashMap<String, String>;

/// Ordered translations. The first map containing a phrase wins.
pub static PHRASES: LazyLock<Facet<Arc<PhraseMap>, Vec<Arc<PhraseMap>>>> = LazyLock::new(|| {
    Facet::new(|maps: &[&Arc<PhraseMap>]| maps.iter().map(|map| Arc::clone(map)).collect())
});

fn parse_lines(source: &str, separator: &str) -> String {
    if separator.is_empty() {
        return source.replace("\r\n", "\n").replace('\r', "\n");
    }
    let mut lines = source.split(separator);
    let mut text = String::with_capacity(source.len());
    text.push_str(lines.next().unwrap_or_default());
    for line in lines {
        // Keep a trailing CR in the line content rather than using it in the break.
        if text.ends_with('\r') {
            text.push('\r');
        }
        text.push('\n');
        text.push_str(line);
    }
    text
}

impl EditorState {
    /// Create a state by parsing `source` with the configured [`LINE_SEPARATOR`].
    /// The first input in precedence order wins. With no input or an empty input,
    /// CRLF, LF, and CR all become LF. Native `Text` treats any remaining LF as
    /// a line break, even if it did not match the separator.
    ///
    /// Selection positions are UTF-8 byte offsets in the parsed text. With no
    /// selection, the cursor starts at zero. All endpoints are checked before
    /// [`ALLOW_MULTIPLE_SELECTIONS`] can reduce the selection to its main range.
    /// Unlike CodeMirror, the native default permits multiple ranges. Set
    /// `ALLOW_MULTIPLE_SELECTIONS.of(false)` to use CodeMirror's default policy.
    ///
    /// Fields and computed facets initialize against the final text and selection.
    ///
    /// # Errors
    /// Returns [`EditError::InvalidSelection`] if an endpoint is out of bounds
    /// or does not lie on a UTF-8 boundary. Returns [`EditError::Configuration`]
    /// for an invalid configuration, including
    /// [`ConfigurationError::DynamicInitialPolicy`] for a computed input to
    /// either initial policy.
    pub fn create(
        source: impl AsRef<str>,
        selection: Option<EditorSelection>,
        extension: Extension,
    ) -> Result<Self, EditError> {
        let configuration = InitialConfiguration::new(extension)?;
        let separator = configuration.static_facet(&LINE_SEPARATOR)?;
        let text = Text::new(parse_lines(
            source.as_ref(),
            separator.as_deref().unwrap_or(""),
        ));
        Self::finish_create(text, selection, configuration)
    }

    /// Create a state from persistent text. This keeps the original bytes and
    /// storage and does not use [`LINE_SEPARATOR`] to parse the document.
    /// A computed separator is allowed for later string conversions.
    ///
    /// This checks the initial selection and applies the static selection policy
    /// as in [`Self::create`]. Fields initialize against this text and selection.
    ///
    /// # Errors
    /// Returns the same errors as [`Self::create`], except that a computed
    /// separator is not an initial policy error.
    pub fn create_from_text(
        text: Text,
        selection: Option<EditorSelection>,
        extension: Extension,
    ) -> Result<Self, EditError> {
        Self::finish_create(text, selection, InitialConfiguration::new(extension)?)
    }

    fn finish_create(
        text: Text,
        selection: Option<EditorSelection>,
        configuration: InitialConfiguration,
    ) -> Result<Self, EditError> {
        let selection = selection.unwrap_or_else(|| EditorSelection::single(0, 0));
        selection.check(|position| text.is_char_boundary(position))?;
        let selection = if !*configuration.static_facet(&ALLOW_MULTIPLE_SELECTIONS)?
            && selection.ranges().len() > 1
        {
            selection.as_single()
        } else {
            selection
        };
        let extensions = configuration.finish(&text, &selection)?;
        Ok(Self {
            text,
            selection,
            extensions,
            wast_publication: None,
        })
    }

    /// Parse the initial string and install its separator with highest
    /// precedence. The separator given here wins over inputs in `extension`.
    /// Initialization callbacks see the parsed text. Use [`Self::with_extensions`]
    /// to install a policy but keep the initial bytes unchanged.
    pub fn with_line_separator(
        source: impl AsRef<str>,
        separator: impl Into<String>,
        extension: Extension,
    ) -> Result<Self, ConfigurationError> {
        let separator = separator.into();
        let text = parse_lines(source.as_ref(), &separator);
        let policy = LINE_SEPARATOR
            .of(separator)
            .with_precedence(Precedence::Highest);
        Self::with_extensions(text, Extension::group([policy, extension]))
    }

    /// Return whether transactions retain multiple selections.
    pub fn allows_multiple_selections(&self) -> bool {
        *self
            .context()
            .facet(&ALLOW_MULTIPLE_SELECTIONS)
            .expect("valid multiple selection facet")
    }

    fn line_separator(&self) -> Arc<Option<String>> {
        self.context()
            .facet(&LINE_SEPARATOR)
            .expect("valid line separator facet")
    }

    /// Return the current output separator. The default is LF.
    pub fn line_break(&self) -> String {
        self.line_separator()
            .as_deref()
            .filter(|separator| !separator.is_empty())
            .unwrap_or("\n")
            .to_owned()
    }

    /// Parse a string with the current separator. Use LF for parsed line breaks,
    /// or CRLF if needed to keep a CR at the end of a line's content. LF bytes
    /// that do not match the separator still make native logical lines.
    /// Use [`Text::new`] to keep the original bytes instead.
    pub fn to_text(&self, source: &str) -> Text {
        Text::new(parse_lines(
            source,
            self.line_separator().as_deref().unwrap_or(""),
        ))
    }

    /// Build changes from byte ranges and strings. Parse each insertion with
    /// the start state's separator and validate each range against its text.
    /// Each range refers to the start document. An out-of-order or overlapping
    /// range starts a new group that maps over all preceding groups.
    /// Transactions that take a prebuilt [`ChangeSet`] do not parse it again.
    /// At an edit boundary, adjacent CR and LF bytes become one line ending.
    pub fn changes<S: AsRef<str>>(
        &self,
        changes: impl IntoIterator<Item = (Range<usize>, S)>,
    ) -> Result<ChangeSet, EditError> {
        let separator = self.line_separator();
        let separator = separator.as_deref().unwrap_or("");
        let changes = changes
            .into_iter()
            .map(|(range, text)| self.resolve_change(range, parse_lines(text.as_ref(), separator)))
            .collect::<Result<Vec<_>, _>>()?;
        ChangeSet::from_specs(changes, self.text().len())
    }

    /// Build changes from strings, existing text, sets, and nested groups.
    /// Strings use the state's separator. A prebuilt set ends the current
    /// group and is rebased as a unit against the preceding changes.
    /// Reject invalid ranges without changing the state.
    pub fn changes_from_specs(&self, specs: &[ChangeSpec<'_>]) -> Result<ChangeSet, EditError> {
        let separator = self.line_separator();
        let separator = separator.as_deref().unwrap_or("");
        let mut resolved = Vec::new();
        let mut pending = vec![specs.iter()];
        while let Some(group) = pending.last_mut() {
            match group.next() {
                Some(ChangeSpec::String(range, insert)) => {
                    resolved
                        .push(self.resolve_change(range.clone(), parse_lines(insert, separator))?);
                }
                Some(ChangeSpec::Text(range, insert)) => {
                    resolved.push(self.resolve_change(range.clone(), insert.as_str().into())?);
                }
                Some(ChangeSpec::Set(set)) => {
                    set.check(self.text())?;
                    resolved.push(ResolvedSpec::Set(set));
                }
                Some(ChangeSpec::Group(children)) => pending.push(children.iter()),
                None => {
                    pending.pop();
                }
            }
        }
        ChangeSet::from_specs(resolved, self.text().len())
    }

    fn resolve_change<'a>(
        &self,
        range: Range<usize>,
        insert: String,
    ) -> Result<ResolvedSpec<'a>, EditError> {
        if !self.text().is_valid_range(range.clone()) {
            return Err(EditError::InvalidRange);
        }
        Ok(ResolvedSpec::Edit(Change::new(range, insert)))
    }

    /// Copy a checked byte range using the current separator for complete line
    /// breaks. A partial CRLF at an endpoint is copied as is.
    pub fn slice_doc(&self, range: Range<usize>) -> Option<String> {
        self.text()
            .slice_with_line_separator(range, &self.line_break())
    }

    /// Translate a phrase and replace its placeholders. `$` means the first
    /// insertion, `$2` the second, and `$$` a dollar sign. If there are no
    /// insertions, leave all markers unchanged.
    pub fn phrase(&self, phrase: &str, insert: &[&str]) -> String {
        let maps = self.context().facet(&PHRASES).expect("typed phrase maps");
        let pattern = maps
            .iter()
            .find_map(|map| map.get(phrase))
            .map_or(phrase, String::as_str);
        if insert.is_empty() || !pattern.contains('$') {
            return pattern.to_owned();
        }

        let mut result = String::with_capacity(pattern.len());
        let mut rest = pattern;
        while let Some(at) = rest.find('$') {
            result.push_str(&rest[..at]);
            let marker = &rest[at..];
            let tail = &marker[1..];
            if let Some(after) = tail.strip_prefix('$') {
                result.push('$');
                rest = after;
                continue;
            }
            let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
            let place = if digits == 0 {
                Some(1)
            } else {
                tail[..digits].parse::<usize>().ok()
            };
            let replacement = place
                .and_then(|number| number.checked_sub(1))
                .and_then(|index| insert.get(index).copied());
            result.push_str(replacement.unwrap_or(&marker[..1 + digits]));
            rest = &tail[digits..];
        }
        result.push_str(rest);
        result
    }

    pub fn read_only(&self) -> bool {
        *self
            .context()
            .facet(&READ_ONLY)
            .expect("valid read-only facet")
    }

    pub fn tab_size(&self) -> NonZeroUsize {
        *self
            .context()
            .facet(&TAB_SIZE)
            .expect("valid tab size facet")
    }
}
