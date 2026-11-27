use std::{borrow::Cow, ops::Range, sync::Arc};

use gpui::Context;
use gpui_editor_codemirror::{
    Change, ChangeSet, EditorSelection, EditorState, TAB_SIZE, Text, Transaction,
    language::{
        StreamSettings,
        markdown::{MarkdownCodeParser, configured_markdown_parser},
    },
    language_data::{LANGUAGES, LanguageDescription, NativeLanguage, SupportedDocument},
};

use super::Editor;
use crate::document::SectionKind;

#[cfg(test)]
mod tests;

pub(super) struct SourceFence {
    pub parser: Option<MarkdownCodeParser>,
    pub stock_json: bool,
    pub body: Range<usize>,
}

impl SourceFence {
    pub(super) fn language(&self) -> Option<NativeLanguage> {
        self.parser.as_ref().map(MarkdownCodeParser::language)
    }
}

pub(super) struct SourceSyntax {
    revision: u64,
    origin: usize,
    parser: MarkdownCodeParser,
    settings: StreamSettings,
    document: SupportedDocument,
}

impl Editor {
    /// Editable body of the active code fence, excluding its header and the
    /// line break before its closing fence.
    pub(super) fn source_fence(&self) -> Option<SourceFence> {
        self.source_fence_with_unlabeled(false)
    }

    pub(super) fn source_fence_with_unlabeled(
        &self,
        include_unlabeled: bool,
    ) -> Option<SourceFence> {
        let active = self
            .active
            .as_ref()
            .filter(|section| section.kind == SectionKind::DraftCode)?;
        let source = self.source().get(active.content.clone())?;
        let (header, body) = source.split_once('\n')?;
        let start = active.content.start + header.len() + 1;
        let header = header.trim();
        let fence = header
            .as_bytes()
            .first()
            .copied()
            .filter(|&c| matches!(c, b'`' | b'~'))?;
        let ticks = header.bytes().take_while(|&c| c == fence).count();
        if ticks < 3 {
            return None;
        }
        let info = header[ticks..].trim_start_matches([' ', '\t']);
        let parser = fence_parser_in(self.view.state(), info);
        if !include_unlabeled && info.is_empty() && parser.is_none() {
            return None;
        }
        let stock_json = parser.as_ref().map(MarkdownCodeParser::language)
            == Some(NativeLanguage::Json)
            && (configured_markdown_parser(self.view.state()).is_some()
                || info
                    .split_whitespace()
                    .next()
                    .is_some_and(|name| name.eq_ignore_ascii_case("json")));
        let tail = body.trim_end_matches(['\r', '\n']);
        let mut end = active.content.end;
        if let Some((body, close)) = tail.rsplit_once('\n') {
            if closing_fence(close, fence, ticks) {
                end = start + body.strip_suffix('\r').unwrap_or(body).len();
            }
        } else if closing_fence(tail, fence, ticks) {
            // There is not an editable body line before the closing fence.
            return None;
        }
        Some(SourceFence {
            parser,
            stock_json,
            body: start..end,
        })
    }

    pub(super) fn word_scope(&self) -> Option<(Range<usize>, MarkdownCodeParser)> {
        let fence = self.source_fence()?;
        if !(fence.body.start..=fence.body.end).contains(&self.cursor()) {
            return None;
        }
        Some((fence.body, fence.parser?))
    }

    pub(super) fn state_with_source_words(&self, use_word_scope: bool) -> Cow<'_, EditorState> {
        let source = self.view.state();
        let Some((scope, parser)) = use_word_scope.then(|| self.word_scope()).flatten() else {
            return Cow::Borrowed(source);
        };
        if matches!(parser, MarkdownCodeParser::Wast(_)) {
            // The installed Markdown mount already reads positional WAST facets.
            return Cow::Borrowed(source);
        }
        let Some(body) = source.text().slice_owned(scope.clone()) else {
            return Cow::Borrowed(source);
        };
        let Ok(document) = parser.parse(body, StreamSettings::default()) else {
            return Cow::Borrowed(source);
        };
        let document = Arc::new(document);
        let Some(extension) = document.language_data_extension_in(source.text(), scope) else {
            return Cow::Borrowed(source);
        };
        let mut state = source.clone();
        // Attach the language data to the operation snapshot, not to edit history.
        if state
            .update(
                Transaction::default()
                    .with_effects([extension.append()])
                    .without_filtering(),
            )
            .is_ok()
        {
            Cow::Owned(state)
        } else {
            Cow::Borrowed(source)
        }
    }
    pub(super) fn current_source_document(
        &self,
        scope: &Range<usize>,
        parser: &MarkdownCodeParser,
        settings: StreamSettings,
    ) -> Option<&SupportedDocument> {
        let cache = self.source_syntax.as_ref()?;
        (cache.revision == self.view.revision()
            && cache.origin == scope.start
            && Some(scope.end) == scope.start.checked_add(cache.document.text().len())
            && &cache.parser == parser
            && cache.settings == settings)
            .then_some(&cache.document)
    }

    pub(super) fn source_document(
        &mut self,
        scope: &Range<usize>,
        parser: &MarkdownCodeParser,
        settings: StreamSettings,
    ) -> Option<&SupportedDocument> {
        if self
            .current_source_document(scope, parser, settings)
            .is_some()
        {
            return self.source_syntax.as_ref().map(|cache| &cache.document);
        }
        let revision = self.view.revision();
        let text = self.view.state().text().slice_owned(scope.clone())?;
        if let Some(mut cache) = self.source_syntax.take().filter(|cache| {
            cache.origin == scope.start && &cache.parser == parser && cache.settings == settings
        }) {
            let previous = cache.document.text();
            let updated = previous == &text || {
                let changes = source_edit(previous.as_str(), text.as_str());
                cache.document.apply_changes(&changes).is_ok()
            };
            if updated {
                cache.revision = revision;
                return Some(&self.source_syntax.insert(cache).document);
            }
        }
        let document = parser.parse(text, settings).ok()?;
        Some(
            &self
                .source_syntax
                .insert(SourceSyntax {
                    revision,
                    origin: scope.start,
                    parser: parser.clone(),
                    settings,
                    document,
                })
                .document,
        )
    }

    pub(super) fn state_in_source(&self, scope: &Range<usize>) -> Option<EditorState> {
        self.state_in_source_with_lines(scope, None)
    }

    pub(super) fn state_in_source_with_lines(
        &self,
        scope: &Range<usize>,
        lines: Option<Text>,
    ) -> Option<EditorState> {
        let selection = self.view.state().selection();
        let local = translate_selection(selection, |pos| {
            (scope.start <= pos && pos <= scope.end).then(|| pos - scope.start)
        })?;
        let text = lines.or_else(|| self.view.state().text().slice_owned(scope.clone()))?;
        if text.len() != scope.len() {
            return None;
        }
        EditorState::create_from_text(text, Some(local), TAB_SIZE.of(self.view.state().tab_size()))
            .ok()
    }

    pub(super) fn apply_local_source_edit(
        &mut self,
        local: Transaction,
        scope: Range<usize>,
        cx: &mut Context<Self>,
    ) -> bool {
        if !local.effects().is_empty() {
            return false;
        }
        let Ok(description) = local.changes().desc(scope.len()) else {
            return false;
        };
        let changes = local.changes().changes().map(|(range, text)| {
            Change::new(scope.start + range.start..scope.start + range.end, text)
        });
        let Ok(changes) = ChangeSet::new(changes.collect()) else {
            return false;
        };
        let mut transaction = Transaction::new(changes).with_scroll_into_view();
        if let Some(selection) = local.selection() {
            let Some(selection) = translate_selection(selection, |pos| {
                (pos <= description.new_length())
                    .then(|| scope.start.checked_add(pos))
                    .flatten()
            }) else {
                return false;
            };
            transaction = transaction.with_selection(selection);
        }
        if let Some(event) = local.user_event() {
            transaction = transaction.with_user_event(event);
        }
        self.apply_source_transaction(transaction, scope, cx)
    }
}

fn source_edit(before: &str, after: &str) -> ChangeSet {
    let prefix = before
        .chars()
        .zip(after.chars())
        .take_while(|(old, new)| old == new)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    let suffix = before[prefix..]
        .chars()
        .rev()
        .zip(after[prefix..].chars().rev())
        .take_while(|(old, new)| old == new)
        .map(|(ch, _)| ch.len_utf8())
        .sum::<usize>();
    ChangeSet::single(Change::new(
        prefix..before.len() - suffix,
        &after[prefix..after.len() - suffix],
    ))
    .expect("the common prefix and suffix contain full characters")
}

pub(super) fn translate_selection(
    selection: &EditorSelection,
    translate: impl Fn(usize) -> Option<usize>,
) -> Option<EditorSelection> {
    let ranges = selection
        .ranges()
        .iter()
        .map(|range| {
            let anchor = translate(range.anchor())?;
            let head = translate(range.head())?;
            Some(if range.undirectional() {
                EditorSelection::undirectional_range(anchor, head)
            } else {
                EditorSelection::range(
                    anchor,
                    head,
                    range.goal_column(),
                    range.bidi_level(),
                    range.assoc(),
                )
            })
        })
        .collect::<Option<Vec<_>>>()?;
    EditorSelection::new(ranges, selection.main_index()).ok()
}

pub(super) fn fence_parser_in(state: &EditorState, info: &str) -> Option<MarkdownCodeParser> {
    if let Some(config) = configured_markdown_parser(state) {
        return config.code_parser(info);
    }
    fence_language(info.split_whitespace().next()?).map(MarkdownCodeParser::Native)
}

pub(super) fn fence_language(name: &str) -> Option<NativeLanguage> {
    if name.eq_ignore_ascii_case("lezer") {
        return Some(NativeLanguage::LezerGrammar);
    }
    if name.eq_ignore_ascii_case("angular") {
        return Some(NativeLanguage::Angular);
    }
    LanguageDescription::match_name(LANGUAGES, name, false)
        .or_else(|| {
            let extension = name.to_ascii_lowercase();
            LANGUAGES
                .iter()
                .find(|entry| entry.extensions.contains(&extension.as_str()))
        })
        .and_then(LanguageDescription::built_in)
}

fn closing_fence(text: &str, fence: u8, ticks: usize) -> bool {
    let line = text.trim();
    line.len() >= ticks && line.bytes().all(|byte| byte == fence)
}
