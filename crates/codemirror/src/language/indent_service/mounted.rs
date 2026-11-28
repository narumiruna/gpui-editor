use std::{ops::Range, sync::Arc};

use crate::{
    EditorState, Text,
    language_data::{NativeSyntax, SupportedDocument, WastLanguage},
};

use super::{
    IndentContext, IndentOptions, Provider, SERVICES, StreamSettings, indent_with_services,
};
use crate::language::{
    line_projection::splits_crlf,
    markdown::{MarkdownCodeParser, MarkdownMount, markdown_snapshot},
};

/// Indentation for a current, contiguous configured Markdown code mount.
/// Commands use body-local byte offsets. Services and node strategies see the
/// parent state and host byte offsets. This query does not grant edit access.
pub struct MarkdownCodeIndent<'a> {
    state: &'a EditorState,
    mount: &'a MarkdownMount,
    body: Range<usize>,
    lines: &'a Text,
    services: Arc<Vec<Provider>>,
}

/// Indentation for a contiguous WAST mount.
pub type MarkdownWastIndent<'a> = MarkdownCodeIndent<'a>;

impl<'a> MarkdownCodeIndent<'a> {
    /// Create the query for a configured WAST parser.
    pub fn new(
        state: &'a EditorState,
        snapshot: &'a SupportedDocument,
        body: Range<usize>,
        parser: &WastLanguage,
    ) -> Option<Self> {
        Self::for_parser(
            state,
            snapshot,
            body,
            &MarkdownCodeParser::Wast(parser.clone()),
        )
    }

    /// Check the installed snapshot, exact body bounds, and parser identity.
    /// A quoted mount with gaps, a stale snapshot, or a different parser does
    /// not qualify, even if its text or grammar name is the same.
    pub fn for_parser(
        state: &'a EditorState,
        snapshot: &'a SupportedDocument,
        body: Range<usize>,
        parser: &MarkdownCodeParser,
    ) -> Option<Self> {
        if matches!(parser, MarkdownCodeParser::Native(_))
            || !std::ptr::eq(markdown_snapshot(state)?.as_ref(), snapshot)
        {
            return None;
        }
        let NativeSyntax::Markdown(doc) = snapshot.syntax() else {
            return None;
        };
        Some(Self {
            state,
            mount: snapshot.markdown_code_body(&body, parser)?,
            body,
            lines: doc.projected_lines().unwrap_or_else(|| state.text()),
            services: state.context().facet(&SERVICES).ok()?,
        })
    }

    /// Body-local syntax for line projection and delimiter gap queries.
    pub fn syntax(&self) -> &'a NativeSyntax {
        self.mount.syntax()
    }

    /// Translate the query, simulated break, and line overrides from body
    /// offsets. An installed service can supply or suppress indentation even
    /// at a body endpoint where the code mount has no active strategy.
    pub fn at(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        let to_host = |at| {
            let at = self.body.start.checked_add(at)?;
            (at <= self.body.end && !splits_crlf(self.state.text().as_str(), at)).then_some(at)
        };
        let position = to_host(position)?;
        let overrides = |host: usize| {
            let local = host.checked_sub(self.body.start)?;
            if host > self.body.end {
                return None;
            }
            options.override_indentation?(local)
        };
        let options = IndentOptions {
            simulate_break: options.simulate_break.and_then(to_host),
            override_indentation: Some(&overrides),
            ..options
        };
        let context = IndentContext::new(self.lines, position, settings, options)?;
        indent_with_services(self.state, &self.services, &context, || {
            if position == self.body.start || position == self.body.end {
                return None;
            }
            self.mount.code_indentation(self.state, &context)
        })
    }
}
