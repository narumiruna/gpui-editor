use std::{ops::Range, sync::Arc};

use gpui::{Bounds, Context, Rgba, Window, fill, point, rgba, size};
use gpui_editor_codemirror::{
    Assoc, EditorState, Text,
    autocomplete::brackets::{
        CloseBracketConfig, CloseBracketSyntax, close_brackets, close_brackets_enabled,
        delete_bracket_pair_with, insert_bracket_with,
    },
    language::{
        StreamSettings,
        brackets::{
            BracketConfig, BracketMatch, BracketMatchRender, BracketMatchingOptions,
            bracket_matching_options, bracket_near_in, configured_bracket_matches_in,
        },
        html::{self, HtmlDocument, HtmlEmbeddedDocument, HtmlOptions},
        javascript, jinja,
        json::{self, JsonDocument},
        markdown::MarkdownCodeParser,
        xml,
    },
    language_data::{CloseBracketData, NativeLanguage, NativeSyntax, SupportedDocument},
    theme_one_dark::ViewStyle,
};

use super::{Editor, highlight::native_language, layout::Item};
use crate::document::SectionKind;

#[cfg(test)]
mod tests;

struct JsonInFence<'a> {
    document: &'a JsonDocument,
    start: usize,
}

impl CloseBracketSyntax for JsonInFence<'_> {
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize> {
        self.document.resolve(pos.checked_sub(self.start)?, side)
    }

    fn range(&self, node: usize) -> Range<usize> {
        let range = self.document.range(node);
        self.start + range.start..self.start + range.end
    }

    fn parent(&self, node: usize) -> Option<usize> {
        self.document.parent(node)
    }

    fn first_child(&self, node: usize) -> Option<usize> {
        self.document.first_child(node)
    }
}

impl Editor {
    fn bracket_scope(&self) -> Option<(Range<usize>, Option<MarkdownCodeParser>)> {
        let active = self.active.as_ref()?;
        if !self.input_enabled || self.is_composing() {
            return None;
        }
        if let Some(fence) = self.source_fence() {
            return Some((fence.body, fence.parser));
        }
        matches!(
            active.kind,
            SectionKind::FrontMatter | SectionKind::Mermaid | SectionKind::DraftCode,
        )
        .then(|| (active.content.clone(), None))
    }

    fn source_bracket_context(
        &self,
        scope: &Range<usize>,
        parser: Option<&MarkdownCodeParser>,
    ) -> (CloseBracketConfig<'static>, Option<SupportedDocument>) {
        if parser.map(MarkdownCodeParser::language) == Some(NativeLanguage::Json) {
            return (json::close_bracket_config(), None);
        }
        let mut config = CloseBracketConfig::default();
        let document = parser.and_then(|parser| {
            let text = self.view.state().text().slice_owned(scope.clone())?;
            parser.parse(text, self.source_bracket_settings()).ok()
        });
        if matches!(parser, Some(MarkdownCodeParser::Wast(_))) {
            let state = self.view.state();
            let at = self.selection().start;
            if let Some(data) = state
                .language_data_at::<CloseBracketData>("closeBrackets", at, Assoc::Before)
                .into_iter()
                .next()
            {
                config.brackets = data.brackets.unwrap_or(config.brackets);
                config.before = data.before.unwrap_or(config.before);
                config.string_prefixes = data.string_prefixes;
            }
            if let Some(chars) = state
                .language_data_at::<&str>("wordChars", at, Assoc::Before)
                .into_iter()
                .next()
            {
                config.word_chars = chars;
            }
            return (config, document);
        }
        if let Some(document) = &document
            && let Some(position) = self.selection().start.checked_sub(scope.start)
        {
            let data = document.data_at(position, Assoc::Before);
            if let Some(data) = data
                .iter()
                .find_map(|entry| entry.get::<CloseBracketData>("closeBrackets"))
            {
                config.brackets = data.brackets.unwrap_or(config.brackets);
                config.string_prefixes = data.string_prefixes;
            }
            if let Some(chars) = data.iter().find_map(|entry| entry.get::<&str>("wordChars")) {
                config.word_chars = chars;
            }
        }
        (config, document)
    }

    pub(super) fn type_source_tag(
        &mut self,
        range: &Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(text, ">" | "/")
            || !self.input_enabled
            || self.is_composing()
            || *range != self.selection()
        {
            return false;
        }
        let Some(fence) = self.source_fence() else {
            return false;
        };
        let language = fence.language();
        let scope = fence.body;
        let state = self.view.state();
        let transaction = match language {
            Some(NativeLanguage::Html) => {
                html::auto_close_tags_in(state, scope.clone(), text, HtmlOptions::default())
            }
            Some(language @ (NativeLanguage::Angular | NativeLanguage::Vue)) => {
                let Some(input) = state.text().slice_owned(scope.clone()) else {
                    return false;
                };
                let Ok(document) = language.support().parse(input) else {
                    return false;
                };
                let Some(position) = range.start.checked_sub(scope.start) else {
                    return false;
                };
                if document.language_at(position, Assoc::Before) != Some(NativeLanguage::Html) {
                    return false;
                }
                html::auto_close_tags_in(
                    state,
                    scope.clone(),
                    text,
                    HtmlOptions {
                        self_closing_tags: true,
                        ..HtmlOptions::default()
                    },
                )
            }
            Some(
                language @ (NativeLanguage::Jinja | NativeLanguage::Liquid | NativeLanguage::Php),
            ) => language
                .support()
                .auto_close_tags_in(state, scope.clone(), text),
            Some(NativeLanguage::Xml) => xml::auto_close_tags_in(state, scope.clone(), text),
            Some(NativeLanguage::Javascript(options)) if options.jsx => {
                javascript::auto_close_tags_in(state, scope.clone(), text, options)
            }
            _ => return false,
        };
        let Ok(Some(transaction)) = transaction else {
            return false;
        };
        self.apply_source_transaction(transaction, scope, cx)
    }

    pub(super) fn type_source_percent_brace(
        &mut self,
        range: &Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if text != "%" || !self.input_enabled || self.is_composing() || *range != self.selection() {
            return false;
        }
        let Some(fence) = self.source_fence() else {
            return false;
        };
        if !matches!(
            fence.language(),
            Some(NativeLanguage::Jinja | NativeLanguage::Liquid)
        ) {
            return false;
        }
        let scope = fence.body;
        let Ok(Some(transaction)) =
            jinja::close_percent_brace_in(self.view.state(), scope.clone(), text)
        else {
            return false;
        };
        self.apply_source_transaction(transaction, scope, cx)
    }

    pub(super) fn type_source_bracket(
        &mut self,
        range: &Range<usize>,
        text: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if !matches!(text, "(" | ")" | "[" | "]" | "{" | "}" | "'" | "\"" | "`") {
            return false;
        }
        let Some((scope, parser)) = self.bracket_scope() else {
            return false;
        };
        let language = parser.as_ref().map(MarkdownCodeParser::language);
        if range.start < scope.start || range.end > scope.end || *range != self.selection() {
            return false;
        }
        let (mut config, document) = self.source_bracket_context(&scope, parser.as_ref());
        let document = if language == Some(NativeLanguage::Json) && text == "\"" {
            let Some(document) = parser.as_ref().and_then(|parser| {
                let body = self.view.state().text().slice_owned(scope.clone())?;
                parser.parse(body, self.source_bracket_settings()).ok()
            }) else {
                return false;
            };
            Some(document)
        } else {
            document
        };
        let json_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Json(document)) => Some(JsonInFence {
                document,
                start: scope.start,
            }),
            _ => None,
        };
        let wast_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Wast(document)) if text == "\"" => {
                Some(document.quote_syntax_in(scope.start))
            }
            _ => None,
        };
        let lezer_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::LezerGrammar(document)) if matches!(text, "'" | "\"") => {
                Some(document.quote_syntax_in(scope.start))
            }
            _ => None,
        };
        let python_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Python(document)) if matches!(text, "'" | "\"") => {
                Some(document.quote_syntax_in(scope.start))
            }
            _ => None,
        };
        let java_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Java(document)) if matches!(text, "'" | "\"") => {
                Some(document.quote_syntax_in(scope.start))
            }
            _ => None,
        };
        let rust_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Rust(document)) if matches!(text, "'" | "\"") => {
                Some(document.quote_syntax_in(scope.start))
            }
            _ => None,
        };
        let go_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Go(document)) if matches!(text, "'" | "\"" | "`") => {
                Some(document.quote_syntax_in(scope.start))
            }
            _ => None,
        };
        let javascript_syntax = if matches!(text, "'" | "\"" | "`") {
            match document.as_ref().map(SupportedDocument::syntax) {
                Some(NativeSyntax::Javascript(document)) => {
                    Some(document.quote_syntax_in(scope.start))
                }
                Some(NativeSyntax::Html(document)) => document.mounts().iter().find_map(|mount| {
                    let local = mount.range();
                    if let HtmlEmbeddedDocument::Javascript(document) = mount.document()
                        && local.contains(&(range.start - scope.start))
                    {
                        Some(document.quote_syntax_in(scope.start + local.start))
                    } else {
                        None
                    }
                }),
                _ => None,
            }
        } else {
            None
        };
        let html_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Html(document)) if matches!(text, "'" | "\"") => {
                Some(document.quote_syntax_in(scope.start))
            }
            _ => None,
        };
        let xml_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Xml(document)) if matches!(text, "'" | "\"") => {
                Some(document.quote_syntax_in(scope.start))
            }
            _ => None,
        };
        let cpp_syntax = match document.as_ref().map(SupportedDocument::syntax) {
            Some(NativeSyntax::Cpp(document)) if matches!(text, "'" | "\"") => {
                Some(document.quote_syntax_in(scope.start))
            }
            _ => None,
        };
        if let Some(syntax) = &json_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &wast_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &lezer_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &python_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &java_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &rust_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &go_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &javascript_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &html_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &xml_syntax {
            config.syntax = Some(syntax);
        } else if let Some(syntax) = &cpp_syntax {
            config.syntax = Some(syntax);
        }
        let before_scope_end = (range.is_empty() && range.end == scope.end)
            .then(|| self.source()[scope.end..].chars().next())
            .flatten()
            .map(|next| format!("{}{next}", config.before));
        let config = CloseBracketConfig {
            before: before_scope_end.as_deref().unwrap_or(config.before),
            ..config
        };
        let state = self.view.state();
        let Ok(Some(mut transaction)) = insert_bracket_with(state, text, config) else {
            return false;
        };
        if close_brackets_enabled(state) == Ok(false) {
            transaction = transaction.with_effects([close_brackets().append()]);
        }
        self.apply_source_transaction(transaction, scope, cx)
    }

    pub(super) fn delete_source_bracket_pair(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((scope, parser)) = self.bracket_scope() else {
            return false;
        };
        let selection = self.selection();
        if !selection.is_empty()
            || selection.start <= scope.start
            || selection.start >= scope.end
            || !self.source()[..selection.start].ends_with(['(', '[', '{', '\'', '"', '`'])
        {
            return false;
        }
        let (config, _) = self.source_bracket_context(&scope, parser.as_ref());
        let Ok(Some(transaction)) = delete_bracket_pair_with(self.view.state(), config) else {
            return false;
        };
        self.apply_source_transaction(transaction, scope, cx)
    }

    fn source_bracket_settings(&self) -> StreamSettings {
        StreamSettings {
            tab_size: self.view.state().tab_size(),
            ..Default::default()
        }
    }

    fn bracket_feedback_scope(&self) -> Option<(Range<usize>, Option<MarkdownCodeParser>)> {
        let active = self.active.as_ref()?;
        if !self.input_enabled || self.is_composing() || self.view.state().read_only() {
            return None;
        }
        match active.kind {
            SectionKind::DraftCode => self
                .source_fence_with_unlabeled(true)
                .map(|fence| (fence.body, fence.parser)),
            SectionKind::FrontMatter | SectionKind::Mermaid => Some((active.content.clone(), None)),
            _ => None,
        }
    }

    pub(super) fn source_bracket_matches(&self) -> Vec<BracketMatch> {
        let Some((scope, parser)) = self.bracket_feedback_scope() else {
            return Vec::new();
        };
        let language = parser.as_ref().map(MarkdownCodeParser::language);
        let state = self.view.state();
        let text = state.text();
        let options = bracket_matching_options(state).ok().flatten();
        let configured = options.as_deref().filter(|options| {
            options.brackets.is_some()
                || options.max_scan_distance.is_some()
                || options.after_cursor.is_some()
        });
        let heads = state
            .selection()
            .ranges()
            .iter()
            .filter(|range| range.is_empty() && (scope.start..=scope.end).contains(&range.head()))
            .map(|range| range.head());
        let Some(language) = language.filter(|&language| supports_bracket_syntax(language)) else {
            if options.is_some() {
                return configured_bracket_matches_in(state, scope, |_, _| true)
                    .unwrap_or_default();
            }
            return heads
                .filter_map(|head| {
                    bracket_near_in(
                        text,
                        scope.clone(),
                        head,
                        BracketConfig::default(),
                        |_, _| true,
                    )
                })
                .collect();
        };
        let parser = parser.expect("known source parser");
        let syntax_config = configured.map(|options| options.config()).or_else(|| {
            matches!(
                &parser,
                MarkdownCodeParser::Wast(_)
                    | MarkdownCodeParser::Json(_)
                    | MarkdownCodeParser::Lezer(_)
                    | MarkdownCodeParser::Rust(_)
            )
            .then(BracketConfig::default)
        });
        let settings = self.source_bracket_settings();
        let current = self.current_source_document(&scope, &parser, settings);
        // Layout normally prepares this syntax. Otherwise parse the body once, on demand.
        let mut parsed = None;
        let mut match_body = |head| {
            let document = current.or_else(|| {
                parsed
                    .get_or_insert_with(|| {
                        parser
                            .parse(text.slice_owned(scope.clone())?, settings)
                            .ok()
                    })
                    .as_ref()
            })?;
            grammar_match_at(document, head, syntax_config)
        };
        heads
            .filter_map(|head| {
                if syntax_config.is_some()
                    || matches!(language, NativeLanguage::Html | NativeLanguage::Xml)
                {
                    bracket_near_body(&scope, head, &mut match_body)
                } else {
                    let candidates = if matches!(language, NativeLanguage::Javascript(_)) {
                        "()[]{}$<>/"
                    } else {
                        "()[]{}"
                    };
                    grammar_bracket_near(text, &scope, head, candidates, &mut match_body)
                }
            })
            .collect()
    }

    pub(super) fn source_bracket_marks(&self) -> Vec<BracketPaintMark> {
        let hits = self.source_bracket_matches();
        if hits.is_empty() {
            return Vec::new();
        }
        let Some((scope, _)) = self.bracket_feedback_scope() else {
            return Vec::new();
        };
        let state = self.view.state();
        let options = bracket_matching_options(state).ok().flatten();
        let render = options
            .as_ref()
            .and_then(|options| options.render_match.as_ref());
        bracket_paint_marks(state, scope, hits, render)
    }

    pub(super) fn standalone_bracket_options(&self) -> Option<Arc<BracketMatchingOptions>> {
        let state = self.view.state();
        if !self.input_enabled || self.is_composing() || state.read_only() {
            return None;
        }
        bracket_matching_options(state).ok().flatten()
    }

    pub(super) fn has_standalone_bracket_syntax(&self) -> bool {
        self.path
            .as_deref()
            .and_then(native_language)
            .is_some_and(supports_bracket_syntax)
    }

    pub(super) fn standalone_bracket_marks(&self) -> Vec<BracketPaintMark> {
        let Some(options) = self.standalone_bracket_options() else {
            return Vec::new();
        };
        let state = self.view.state();
        let scope = 0..state.text().len();
        let hits = if self.has_standalone_bracket_syntax() {
            // Do not use plain matching while the syntax for this snapshot is pending.
            let Some(document) = self.current_native_document() else {
                return Vec::new();
            };
            let config = options.config();
            state
                .selection()
                .ranges()
                .iter()
                .filter(|range| range.is_empty())
                .filter_map(|range| grammar_match_at(document, range.head(), Some(config)))
                .collect()
        } else {
            configured_bracket_matches_in(state, scope.clone(), |_, _| true).unwrap_or_default()
        };
        bracket_paint_marks(state, scope, hits, options.render_match.as_ref())
    }

    #[cfg(test)]
    pub(super) fn source_bracket_match(&self) -> Option<BracketMatch> {
        self.source_bracket_matches().into_iter().next()
    }
}

fn bracket_paint_marks(
    state: &EditorState,
    scope: Range<usize>,
    hits: Vec<BracketMatch>,
    render: Option<&BracketMatchRender>,
) -> Vec<BracketPaintMark> {
    if let Some(render) = render {
        let marks: Vec<_> = hits.iter().flat_map(|hit| render(hit, state)).collect();
        if marks.iter().any(|mark| {
            mark.from < scope.start
                || mark.to > scope.end
                || mark.from >= mark.to
                || state.text().slice(mark.from..mark.to).is_none()
        }) {
            return Vec::new();
        }
        return marks
            .into_iter()
            .filter_map(|mark| {
                let mut matching = false;
                let mut nonmatching = false;
                for class in mark.value.class().split_ascii_whitespace() {
                    matching |= class == "cm-matchingBracket";
                    nonmatching |= class == "cm-nonmatchingBracket";
                }
                (matching || nonmatching).then_some(BracketPaintMark {
                    range: mark.from..mark.to,
                    matched: !nonmatching,
                })
            })
            .collect();
    }
    hits.into_iter()
        .flat_map(|hit| {
            std::iter::once(hit.start)
                .chain(hit.end)
                .map(move |range| BracketPaintMark {
                    range,
                    matched: hit.matched,
                })
        })
        .collect()
}

fn supports_bracket_syntax(language: NativeLanguage) -> bool {
    matches!(
        language,
        NativeLanguage::Wast
            | NativeLanguage::LezerGrammar
            | NativeLanguage::Json
            | NativeLanguage::Java
            | NativeLanguage::Rust
            | NativeLanguage::Go
            | NativeLanguage::Python
            | NativeLanguage::Javascript(_)
            | NativeLanguage::Css
            | NativeLanguage::Cpp
            | NativeLanguage::Html
            | NativeLanguage::Xml
    )
}

fn grammar_match_at(
    document: &SupportedDocument,
    head: usize,
    config: Option<BracketConfig<'_>>,
) -> Option<BracketMatch> {
    match document.syntax() {
        NativeSyntax::Wast(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        NativeSyntax::LezerGrammar(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        NativeSyntax::Json(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        NativeSyntax::Java(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        NativeSyntax::Rust(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        NativeSyntax::Go(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        NativeSyntax::Python(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        NativeSyntax::Javascript(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        NativeSyntax::Css(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        NativeSyntax::Cpp(doc) => doc.bracket_near_with_config(head, config.unwrap_or_default()),
        NativeSyntax::Html(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => html_match_at(doc, head),
        },
        NativeSyntax::Xml(doc) => match config {
            Some(config) => doc.bracket_near_with_config(head, config),
            None => doc.bracket_near(head),
        },
        _ => None,
    }
}

fn html_match_at(document: &HtmlDocument, head: usize) -> Option<BracketMatch> {
    if let Some(mount) = document.mounts().iter().find(|mount| {
        let range = mount.range();
        !range.is_empty() && (range.start..=range.end).contains(&head)
    }) {
        let range = mount.range();
        let mounted = || match mount.document() {
            HtmlEmbeddedDocument::Javascript(script) => {
                bracket_near_body(&range, head, |local| script.bracket_near(local))
            }
            HtmlEmbeddedDocument::Css(style) => {
                bracket_near_body(&range, head, |local| style.bracket_near(local))
            }
            HtmlEmbeddedDocument::Custom(_) => bracket_near_in(
                document.text(),
                range.clone(),
                head,
                BracketConfig::default(),
                |_, _| true,
            ),
        };
        // At a mount boundary, check the side just before the caret first.
        if head == range.start {
            document.bracket_near(head).or_else(mounted)
        } else if head == range.end {
            mounted().or_else(|| document.bracket_near(head))
        } else {
            mounted()
        }
    } else {
        document.bracket_near(head)
    }
}

fn grammar_bracket_near(
    text: &Text,
    scope: &Range<usize>,
    head: usize,
    candidates: &str,
    match_body: impl FnOnce(usize) -> Option<BracketMatch>,
) -> Option<BracketMatch> {
    bracket_near_body(scope, head, |local| {
        let before = (head > scope.start)
            .then(|| text.slice(head - 1..head))
            .flatten();
        let after = (head < scope.end)
            .then(|| text.slice(head..head + 1))
            .flatten();
        [before, after]
            .into_iter()
            .flatten()
            .any(|ch| candidates.contains(ch))
            .then(|| match_body(local))?
    })
}

fn bracket_near_body(
    scope: &Range<usize>,
    head: usize,
    match_body: impl FnOnce(usize) -> Option<BracketMatch>,
) -> Option<BracketMatch> {
    if !(scope.start..=scope.end).contains(&head) {
        return None;
    }
    let hit = match_body(head - scope.start)?;
    let to_source = |range: Range<usize>| scope.start + range.start..scope.start + range.end;
    Some(BracketMatch {
        start: to_source(hit.start),
        end: hit.end.map(to_source),
        matched: hit.matched,
    })
}

pub(super) struct BracketPaintMark {
    range: Range<usize>,
    matched: bool,
}

impl BracketPaintMark {
    pub(super) fn overlaps(&self, range: &Range<usize>) -> bool {
        self.range.start < range.end && range.start < self.range.end
    }
}

pub(super) fn paint_source_brackets(
    marks: &[BracketPaintMark],
    item: &Item,
    row: Bounds<gpui::Pixels>,
    style: Option<&ViewStyle>,
    window: &mut Window,
    #[cfg(test)] submitted: &mut Vec<gpui::PaintQuad>,
) {
    for mark in marks {
        let color: Rgba = if let Some(style) = style {
            rgba(if mark.matched {
                style.matching_bracket.rgba32()
            } else {
                style.nonmatching_bracket.rgba32()
            })
        } else if mark.matched {
            rgba(0x328c_8252)
        } else {
            rgba(0xbb55_5544)
        };
        for x in item.x_spans(mark.range.clone()) {
            let quad = fill(
                Bounds::new(
                    row.origin + point(x.start, gpui::px(0.)),
                    size(x.end - x.start, row.size.height),
                ),
                color,
            );
            #[cfg(test)]
            let recorded = quad.clone();
            window.paint_quad(quad);
            #[cfg(test)]
            submitted.push(recorded);
        }
    }
}
