use std::sync::{Arc, LazyLock};

use crate::{
    Assoc, EditorState, Extension, MapMode, Text,
    language::{
        LanguageData,
        css::CssNodeKind,
        html::{
            HtmlDocument, HtmlEmbeddedDocument, HtmlNodeKind, close_tags_after,
            html_input_is_active, html_schema_completion_source,
        },
    },
    view::input::input_completion_handler,
};

use super::{
    LrDataFacet, NativeLanguage, NativeSyntax, ScriptCompletionSource, SupportedDocument,
    attached_language_snapshot,
    input::{close_projected_tags_after, close_switched_end_tags},
    jinja_liquid_install, markdown_install, metadata,
    projection::HtmlProjection,
    route::{self, HtmlDataSite},
    script_install, stylesheet_install,
};

static HTML_DATA: LazyLock<LrDataFacet> =
    LazyLock::new(|| LrDataFacet::new(Some(metadata(NativeLanguage::Html))));

pub(super) fn extension() -> Extension {
    static BUNDLE: LazyLock<Extension> = LazyLock::new(|| {
        Extension::group([
            NativeLanguage::Html.support().tracked_syntax_extension(),
            support(),
        ])
    });
    BUNDLE.clone()
}

/// Each stock HTML instance owns its schema and child support inputs, but
/// installs no additional parser or language data provider.
pub(super) fn support() -> Extension {
    let schema: ScriptCompletionSource = Arc::new(|state, at, explicit| {
        let active = attached_language_snapshot(state)?;
        let syntax = active.syntax();
        match route::html_data_site(&active, at, Assoc::Before)? {
            HtmlDataSite::Host {
                document,
                query,
                projection,
            } => {
                let result =
                    html_schema_completion_source(document, query.at, explicit, &|name| {
                        projection
                            .as_ref()
                            .is_none_or(|map| map.source(name).is_some())
                    })?;
                active.project_completion(&query, result)
            }
            HtmlDataSite::Template => {
                let doc = route::html_host(syntax)?;
                // The left node in a directive or binding has no HTML
                // attribute name or value, even at the end of the attribute.
                let attribute =
                    std::iter::successors(Some(route::html_left_node(doc, at)), |node| {
                        node.parent()
                    })
                    .find(|node| node.kind() == HtmlNodeKind::Attribute);
                if attribute.is_some_and(|node| {
                    let source = &doc.text().as_str()[node.range()];
                    match syntax {
                        NativeSyntax::Vue(_) => route::vue_attribute(source),
                        NativeSyntax::Angular(_) => source.starts_with(['*', '#', '(', '[']),
                        _ => false,
                    }
                }) {
                    return None;
                }
                html_schema_completion_source(doc, at, explicit, &|_| true)
            }
            HtmlDataSite::Mount(mount, query) => {
                if !matches!(syntax, NativeSyntax::Vue(_) | NativeSyntax::Angular(_))
                    || mount.range().end != at
                {
                    return None;
                }
                let doc = route::html_host(syntax)?;
                let HtmlEmbeddedDocument::Css(css) = mount.document() else {
                    return None;
                };
                let value = route::html_left_node(doc, at);
                if value.kind() != HtmlNodeKind::UnquotedAttributeValue
                    || value.range() != mount.range()
                {
                    return None;
                }
                // At this unquoted boundary the right side selects the HTML
                // facet, but the callable resolves the CSS token on the left.
                let mut node = css.root();
                while let Some(child) = node.children().find(|child| {
                    let range = child.range();
                    range.start < query.at && query.at <= range.end
                }) {
                    node = child;
                }
                if node.kind() != CssNodeKind::TagName {
                    return None;
                }
                Some(doc.mounted_tag_name_completion(value, query.source(node.range().start)?, at))
            }
            HtmlDataSite::Script(..)
            | HtmlDataSite::JinjaLiquidRoot(_)
            | HtmlDataSite::MarkdownRoot
            | HtmlDataSite::PhpRoot
            | HtmlDataSite::Invalid => None,
        }
    });
    Extension::group([
        data_input(LanguageData::default().with("autocomplete", schema)),
        CLOSE_TAGS.clone(),
        script_install::javascript_support(false),
        stylesheet_install::css_support(),
    ])
}

pub(super) fn data_input(data: LanguageData) -> Extension {
    HTML_DATA.of(data)
}

pub(super) fn configured_data(
    state: &EditorState,
    active: &SupportedDocument,
    at: usize,
    side: Assoc,
) -> Vec<Arc<LanguageData>> {
    match route::html_data_site(active, at, side) {
        Some(HtmlDataSite::Host { .. }) => HTML_DATA.read(state),
        Some(HtmlDataSite::JinjaLiquidRoot(language)) => {
            jinja_liquid_install::data(state, language)
        }
        Some(HtmlDataSite::MarkdownRoot) => markdown_install::data(state),
        Some(HtmlDataSite::PhpRoot) => vec![Arc::new(metadata(NativeLanguage::Php))],
        Some(HtmlDataSite::Script(..)) => script_install::javascript_data(state),
        Some(HtmlDataSite::Mount(mount, _)) => match mount.document() {
            HtmlEmbeddedDocument::Javascript(_) => script_install::javascript_data(state),
            HtmlEmbeddedDocument::Css(_) => NativeLanguage::Css
                .stylesheet_data_facet()
                .expect("stock CSS")
                .read(state),
            HtmlEmbeddedDocument::Custom(_) => Vec::new(),
        },
        Some(HtmlDataSite::Template | HtmlDataSite::Invalid) | None => Vec::new(),
    }
}

static CLOSE_TAGS: LazyLock<Extension> = LazyLock::new(|| {
    input_completion_handler(|input, default| {
        let start = input.state();
        let range = input.range();
        let typed = input.text();
        if start.read_only() || !range.is_empty() || !matches!(typed, ">" | "/") {
            return Ok(None);
        }
        let Some(active) = attached_language_snapshot(start) else {
            return Ok(None);
        };
        let syntax = active.syntax();
        let Some(HtmlDataSite::Host {
            document: before,
            query,
            projection,
        }) = route::html_data_site(&active, range.start, Assoc::Before)
        else {
            return Ok(None);
        };
        let plain = matches!(syntax, NativeSyntax::Html(_));
        let original = projection
            .as_ref()
            .filter(|_| !matches!(syntax, NativeSyntax::Markdown(_)))
            .map(|map| (start.text().as_str(), map, active.language));
        if !plain && unfinished_raw_mount(before, query.at, original) {
            return Ok(None);
        }
        let base = default()?;
        let Some(updated) = attached_language_snapshot(&base.state) else {
            return Ok(None);
        };
        let after = updated.syntax();
        if projection.is_some() {
            if matches!(syntax, NativeSyntax::Markdown(_)) {
                return close_projected_tags_after(base, typed, |selection| {
                    route::markdown_html_projection(&updated, selection.head(), Assoc::Before)
                        .map(|map| map.close_tag_after(&base.state, typed, selection))
                        .transpose()
                });
            }
            if active.language == updated.language
                && let Some(projection) = HtmlProjection::new(after)
            {
                return projection.close_tags_after(base, typed);
            }
            return Ok(None);
        }
        if plain && !matches!(after, NativeSyntax::Html(_)) {
            if typed != ">" || !matches!(after, NativeSyntax::Xml(_)) {
                return Ok(None);
            }
            // XML cannot report an HTML raw mount after the switch. Check the
            // original unfinished tag, including space before the missing `>`.
            let tag_end = before.text().as_str()[..query.at]
                .trim_end_matches([' ', '\t', '\r', '\n'])
                .len();
            if !html_input_is_active(before, range.start, None, |_| None)
                || unfinished_raw_mount(before, tag_end, None)
            {
                return Ok(None);
            }
            return close_switched_end_tags(&base.state, after);
        }
        let Some(after) = route::html_host(after) else {
            return Ok(None);
        };
        if plain {
            // Keep the existing HTML recovery guard. Template input checks
            // the original route instead, even if a filter rewrites the tag.
            let reverse = base
                .transaction
                .changes()
                .desc(start.text().len())?
                .inverted();
            if !html_input_is_active(
                before,
                range.start,
                Some((after, base.state.selection().main().head())),
                |at| reverse.map_pos(at, Assoc::After, MapMode::Simple),
            ) {
                return Ok(None);
            }
        }
        close_tags_after(&base.state, after, typed)
    })
});

fn unfinished_raw_mount(
    doc: &HtmlDocument,
    at: usize,
    original: Option<(&str, &HtmlProjection<'_>, NativeLanguage)>,
) -> bool {
    use HtmlNodeKind::{Error, OpenTag, TagName};
    let source = doc.text().as_str();
    let tag = std::iter::successors(Some(route::html_left_node(doc, at)), |node| node.parent())
        .find(|node| node.kind() == OpenTag);
    let Some(tag) = tag.filter(|tag| {
        tag.range().end == at
            && tag
                .children()
                .next_back()
                .is_some_and(|last| last.kind() == Error && last.range().is_empty())
    }) else {
        return false;
    };
    let name = tag.children().find(|node| node.kind() == TagName);
    if !name.is_some_and(|name| {
        let name = &source[name.range()];
        name.eq_ignore_ascii_case("script") || name.eq_ignore_ascii_case("style")
    }) {
        return false;
    }
    // Native recovery omits the raw body at a missing `>`. Probe only this
    // opening tag so the stock MIME rules run on the original attributes.
    let at_body_start = |probe: &HtmlDocument| {
        probe
            .mounts()
            .iter()
            .any(|mount| mount.range().start == probe.text().len())
    };
    if let Some((source, map, language)) = original {
        let Some(range) = map.selection_bounds(tag.range()) else {
            return false;
        };
        return language
            .parse(Text::new(format!("{}>", &source[range])))
            .is_ok_and(|probe| {
                HtmlProjection::new(&probe).is_some_and(|html| at_body_start(html.document))
            });
    }
    at_body_start(&HtmlDocument::parse(Text::new(format!(
        "{}>",
        &source[tag.range()]
    ))))
}
