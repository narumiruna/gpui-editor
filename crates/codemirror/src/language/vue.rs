//! Vue template syntax from `@codemirror/lang-vue` 0.1.3.
//!
//! An HTML host with the self-closing dialect provides the tag tree and routes
//! raw bodies to the native HTML child languages. The Vue grammar replaces
//! directive attributes and text with interpolation. Expressions use the
//! JavaScript single-expression parser and its local recovery. Script and
//! style blocks keep the HTML language rules; this does not run a Vue build
//! or load custom parsers. All public positions are UTF-8 bytes. See
//! `docs/licenses/codemirror-lang-vue.txt`.

use std::ops::Range;

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{
    HighlightSpan, IndentContext, IndentOptions, LanguageData, StreamSettings,
    css::CssDocument,
    html::{self, HtmlDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage, HtmlOptions},
    javascript::{JavascriptDocument, JavascriptOptions},
    lezer::ReuseStats,
};

mod tables;
mod template;

/// Name of the upstream language provider.
pub const NAME: &str = "vue";
/// Opening tokens for automatic closing in the template host.
pub const CLOSE_BRACKETS: [&str; 2] = ["{", "\""];

/// Language data for the HTML host, with Vue's bracket settings.
pub fn language_data() -> LanguageData {
    html::language_data().with("closeBrackets", CLOSE_BRACKETS)
}

/// The context of a JavaScript expression inside the HTML host.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VueExpressionKind {
    Interpolation,
    Directive,
}

/// A JavaScript child tree. Its positions are relative to `range().start`.
#[derive(Debug)]
pub struct VueExpression {
    range: Range<usize>,
    kind: VueExpressionKind,
    document: JavascriptDocument,
}

impl VueExpression {
    /// The expression's byte range, excluding quotes and braces.
    pub fn range(&self) -> Range<usize> {
        self.range.clone()
    }

    pub fn kind(&self) -> VueExpressionKind {
        self.kind
    }

    pub fn document(&self) -> &JavascriptDocument {
        &self.document
    }
}

/// A native Vue document with its HTML host and template expressions.
#[derive(Debug)]
pub struct VueDocument {
    host: HtmlDocument,
    template: template::Template,
}

impl VueDocument {
    pub fn parse(text: Text) -> Self {
        Self::parse_with_options(text, HtmlOptions::default())
    }

    /// Parse with HTML options. Vue always enables self-closing tags.
    pub fn parse_with_options(text: Text, mut options: HtmlOptions) -> Self {
        options.self_closing_tags = true;
        Self::from_host(HtmlDocument::parse_with_options(text, options))
    }

    /// Apply byte edits using the same HTML dialect. Reuse strict host
    /// checkpoints where possible, then rebuild the Vue overlays and expressions.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        Ok(Self::from_host(self.host.reparse(changes)?))
    }

    /// Strict HTML host checkpoint reuse. This does not count mounted trees.
    pub fn reuse_stats(&self) -> ReuseStats {
        self.host.reuse_stats()
    }

    fn from_host(host: HtmlDocument) -> Self {
        let template = template::Template::parse(&host);
        Self { host, template }
    }

    pub fn text(&self) -> &Text {
        self.host.text()
    }

    /// The native HTML host. Use its mounts to inspect script and style blocks.
    pub fn host(&self) -> &HtmlDocument {
        &self.host
    }

    /// Expressions from Vue text and directives, in source order.
    pub fn expressions(&self) -> &[VueExpression] {
        &self.template.expressions
    }

    /// Highlight the host, Vue syntax, and child languages. Clip to `range`,
    /// or return `None` if the range is invalid or splits a UTF-8 character.
    pub fn highlights(
        &self,
        range: Range<usize>,
    ) -> Option<impl Iterator<Item = HighlightSpan<'_>>> {
        let host = self.host.highlights(range.clone())?;
        Some(self.template.highlights(range, host).into_iter())
    }

    /// Get a fold that starts on this line and continues past it.
    pub fn foldable(&self, line_start: usize, line_end: usize) -> Option<Range<usize>> {
        self.text().slice(line_start..line_end)?;
        self.host.foldable(line_start, line_end).or_else(|| {
            let expression = self.template.expression_at(line_end)?;
            let offset = expression.range.start;
            let range = expression
                .document
                .foldable(line_start.saturating_sub(offset), line_end - offset)?;
            Some(offset + range.start..offset + range.end)
        })
    }

    /// Compute indentation in columns at a UTF-8 byte position.
    pub fn indentation(
        &self,
        position: usize,
        settings: StreamSettings,
        options: IndentOptions<'_>,
    ) -> Option<usize> {
        let cx = IndentContext::new(self.text(), position, settings, options)?;
        if let Some(expression) = self.template.expression_at(position) {
            return javascript_indentation(
                &expression.document,
                &expression.range,
                &cx,
                settings,
                options,
                || self.host.indentation(position, settings, options),
            );
        }
        let mounts = self.host.mounts();
        let index = mounts.partition_point(|mount| mount.range().end <= position);
        if let Some(mount) = mounts
            .get(index)
            .filter(|mount| mount.range().start < position)
        {
            let range = mount.range();
            // At the start of a mount, the HTML host applies its own indent
            // rule for the enclosing tag. Keep this rule when a child has no
            // applicable indent strategy of its own.
            let outer = || self.host.indentation(range.start, settings, options);
            match mount.document() {
                HtmlEmbeddedDocument::Javascript(script) => {
                    return javascript_indentation(script, &range, &cx, settings, options, outer);
                }
                HtmlEmbeddedDocument::Css(css)
                    if mount.language() == HtmlEmbeddedLanguage::Css
                        && at_stylesheet_root(css, position - range.start) =>
                {
                    return outer();
                }
                _ => {}
            }
        }
        self.host.indentation(position, settings, options)
    }
}

fn javascript_indentation(
    script: &JavascriptDocument,
    range: &Range<usize>,
    cx: &IndentContext<'_>,
    settings: StreamSettings,
    options: IndentOptions<'_>,
    outer: impl FnOnce() -> Option<usize>,
) -> Option<usize> {
    let offset = range.start;
    let over = |position| {
        if position == 0 {
            cx.line_indent(offset, Assoc::After)
        } else {
            options
                .override_indentation
                .and_then(|get| get(offset + position))
        }
    };
    script.embedded_indentation(
        cx.position() - offset,
        settings,
        IndentOptions {
            override_indentation: Some(&over),
            simulate_break: cx
                .simulated_break()
                .filter(|point| range.contains(point))
                .map(|point| point - offset),
            simulate_double_break: options.simulate_double_break,
        },
        outer,
    )
}

fn at_stylesheet_root(css: &CssDocument, position: usize) -> bool {
    let root = css.root();
    let (mut start, mut end) = (0, root.children().len());
    while start < end {
        let mid = start + (end - start) / 2;
        let range = root.children().nth(mid).unwrap().range();
        if position <= range.start {
            end = mid;
        } else if position >= range.end {
            start = mid + 1;
        } else {
            return false;
        }
    }
    true
}
