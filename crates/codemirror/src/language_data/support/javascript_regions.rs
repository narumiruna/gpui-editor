use std::ops::Range;

use crate::language::{
    html::{HtmlDocument, HtmlEmbeddedDocument, HtmlEmbeddedLanguage},
    javascript::JavascriptOptions,
};

use super::{NativeLanguage, NativeSyntax, SupportedDocument};

impl SupportedDocument {
    /// Return JavaScript-family regions in source order, in original UTF-8 bytes.
    /// A JavaScript, JSX, TypeScript, or TSX document returns its entire range,
    /// even when empty. Stock HTML routes and selected Markdown code parsers
    /// contribute their mounted regions. A Markdown JavaScript mount returns a
    /// separate range for each source segment.
    ///
    /// Keep original LF, CRLF, and lone CR bytes. Skip custom HTML routes,
    /// unterminated quoted attributes, and HTML mounted across Markdown source
    /// gaps. Other mixed grammars are not queried.
    /// Use the snapshot for the current editor text when supplying regions to
    /// [`crate::language::javascript::es_lint`].
    pub fn javascript_regions(&self) -> Vec<(NativeLanguage, Range<usize>)> {
        match &self.syntax {
            NativeSyntax::Javascript(_) => vec![(self.language, 0..self.text().len())],
            NativeSyntax::Html(html) => html_regions(html, 0).collect(),
            NativeSyntax::Markdown(_) => {
                let mut regions = Vec::new();
                for mount in &self.markdown_mounts {
                    let mut ranges = mount.source_ranges();
                    match (mount.language(), mount.syntax()) {
                        (language @ NativeLanguage::Javascript(_), NativeSyntax::Javascript(_)) => {
                            regions.extend(ranges.map(|range| (language, range)));
                        }
                        (NativeLanguage::Html, NativeSyntax::Html(html)) => {
                            // The HTML offsets are in concatenated input. Only a
                            // single source segment needs a simple offset shift.
                            if let Some(range) = ranges.next()
                                && ranges.len() == 0
                            {
                                regions.extend(html_regions(html, range.start));
                            }
                        }
                        _ => {}
                    }
                }
                regions
            }
            _ => Vec::new(),
        }
    }
}

fn html_regions(
    html: &HtmlDocument,
    offset: usize,
) -> impl Iterator<Item = (NativeLanguage, Range<usize>)> + '_ {
    use HtmlEmbeddedLanguage::*;

    let source = html.text().as_str().as_bytes();
    html.mounts().iter().filter_map(move |mount| {
        let HtmlEmbeddedDocument::Javascript(javascript) = mount.document() else {
            return None;
        };
        // The HTML JSON route shares JS language data only when parsed with
        // the JavaScript SingleExpression grammar, not the JSON grammar.
        let (typescript, jsx) = match (mount.language(), javascript.root().kind().name()) {
            (JavaScript, "Script") | (Json, "SingleExpression") => (false, false),
            (TypeScript, "Script") => (true, false),
            (Jsx, "Script") => (false, true),
            (Tsx, "Script") => (true, true),
            _ => return None,
        };
        let range = mount.range();
        // An unterminated value can have different end points in native and
        // upstream recovery. Do not trim or otherwise guess a range for it.
        if let Some(quote @ (b'\'' | b'"')) =
            range.start.checked_sub(1).and_then(|at| source.get(at))
            && source.get(range.end) != Some(quote)
        {
            return None;
        }
        Some((
            NativeLanguage::Javascript(JavascriptOptions { typescript, jsx }),
            offset + range.start..offset + range.end,
        ))
    })
}

#[cfg(test)]
mod tests {
    use crate::{
        Text,
        language::{
            html::{HtmlNestedLanguage, HtmlNestedParser, HtmlNesting, HtmlOptions},
            javascript::JavascriptDocument,
        },
    };

    use super::*;

    #[test]
    fn custom_routes_do_not_claim_default_javascript_identity() {
        let html = HtmlDocument::parse_with_nesting(
            Text::new("<script>ignored()</script><p onclick='included()'></p>"),
            HtmlOptions::default(),
            HtmlNesting {
                nested_languages: &[HtmlNestedLanguage {
                    tag: "script",
                    attrs: None,
                    parser: HtmlNestedParser {
                        name: "JavaScript",
                        parse: &|text| Box::new(JavascriptDocument::parse(text)),
                    },
                }],
                ..HtmlNesting::default()
            },
        );
        assert_eq!(html.mounts().len(), 2);
        assert_eq!(
            html.mounts()[0].language(),
            HtmlEmbeddedLanguage::Custom("JavaScript")
        );
        let (_, range) = html_regions(&html, 0).next().unwrap();
        assert_eq!(html.text().slice(range), Some("included()"));
        assert_eq!(html_regions(&html, 0).count(), 1);
    }
}
