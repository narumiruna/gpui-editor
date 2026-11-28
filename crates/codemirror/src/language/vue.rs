//! Vue template syntax from `@codemirror/lang-vue` 0.1.3.
//!
//! The native HTML host enables self-closing tags. Default and custom HTML
//! routes select parsers for block bodies. Outside those bodies, the Vue
//! grammar handles directives and interpolations. Expressions use the
//! JavaScript single-expression parser and its local recovery. All public
//! positions are UTF-8 bytes. See
//! `docs/licenses/codemirror-lang-vue.txt`.

use std::ops::Range;

use crate::{Assoc, ChangeSet, EditError, Text};

use super::{
    HighlightSpan, IndentContext, IndentOptions, LanguageData, StreamSettings,
    css::CssDocument,
    html::{
        self, HtmlDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage, HtmlMount,
        HtmlNestedLanguage, HtmlNesting, HtmlOptions,
    },
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
    pub fn parse_with_options(text: Text, options: HtmlOptions) -> Self {
        Self::parse_with_nested_languages(text, options, &[])
    }

    /// Parse with custom HTML body routes before the stock routes.
    pub fn parse_with_nested_languages(
        text: Text,
        options: HtmlOptions,
        nested_languages: &[HtmlNestedLanguage<'_>],
    ) -> Self {
        Self::parse_with_nesting(
            text,
            options,
            HtmlNesting {
                nested_languages,
                ..HtmlNesting::default()
            },
        )
    }

    /// Parse with custom HTML body and attribute routes before the stock routes.
    /// A body mount hides Vue template syntax. A Vue directive takes precedence
    /// over a route for the same attribute value. Always enable self-closing tags.
    pub fn parse_with_nesting(
        text: Text,
        mut options: HtmlOptions,
        nesting: HtmlNesting<'_>,
    ) -> Self {
        options.self_closing_tags = true;
        Self::from_host(HtmlDocument::parse_with_nesting(text, options, nesting))
    }

    /// Apply byte edits using the same HTML dialect and the stock routes.
    pub fn reparse(&self, changes: &ChangeSet) -> Result<Self, EditError> {
        self.reparse_with_nesting(changes, HtmlNesting::default())
    }

    /// Apply byte edits and rebuild the custom HTML body routes.
    pub fn reparse_with_nested_languages(
        &self,
        changes: &ChangeSet,
        nested_languages: &[HtmlNestedLanguage<'_>],
    ) -> Result<Self, EditError> {
        self.reparse_with_nesting(
            changes,
            HtmlNesting {
                nested_languages,
                ..HtmlNesting::default()
            },
        )
    }

    /// Apply byte edits and rebuild the mounts and Vue overlays. Supply custom
    /// routes again; the document does not retain them. Strict HTML host
    /// checkpoints can be reused before the first edit.
    pub fn reparse_with_nesting(
        &self,
        changes: &ChangeSet,
        nesting: HtmlNesting<'_>,
    ) -> Result<Self, EditError> {
        Ok(Self::from_host(
            self.host.reparse_with_nesting(changes, nesting)?,
        ))
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

    /// The base HTML document, including mounts that Vue directives replace.
    /// Use [`Self::mounts`] to inspect active routes.
    pub fn host(&self) -> &HtmlDocument {
        &self.host
    }

    /// The active HTML mounts, in source order. Vue expression mounts are
    /// available through [`Self::expressions`].
    pub fn mounts(&self) -> impl Iterator<Item = &HtmlMount> {
        self.host
            .mounts()
            .iter()
            .filter(|mount| !self.template.replaces_value(mount.range().start))
    }

    /// Expressions from Vue text and directives outside routed bodies, in source order.
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
        let host_end = self.replaced_value_start(line_end).unwrap_or(line_end);
        self.host
            .foldable(line_start, host_end)
            .filter(|fold| fold.end > line_end)
            .or_else(|| {
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
        let host_position = self.replaced_value_start(position).unwrap_or(position);
        let host_indent = || self.host.indentation(host_position, settings, options);
        if let Some(expression) = self.template.expression_at(position) {
            return javascript_indentation(
                &expression.document,
                &expression.range,
                &cx,
                settings,
                options,
                host_indent,
            );
        }
        if let Some(mount) = self
            .mount_at(position)
            .filter(|_| host_position == position)
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
        host_indent()
    }

    fn mount_at(&self, position: usize) -> Option<&HtmlMount> {
        let mounts = self.host.mounts();
        let index = mounts.partition_point(|mount| mount.range().end <= position);
        mounts
            .get(index)
            .filter(|mount| mount.range().start < position)
    }

    fn replaced_value_start(&self, position: usize) -> Option<usize> {
        let start = self.mount_at(position)?.range().start;
        self.template.replaces_value(start).then_some(start)
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
