use std::{
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Assoc, EditorState, Extension, Facet, Precedence, Transaction,
    autocomplete::session::SourceResult,
    commands::CommandResult,
    language::{
        IndentContext, IndentOptions, LanguageData,
        fold::fold_service,
        line_projection::splits_crlf,
        markdown::{
            MarkdownMarkupCommand, markdown_completion_source, markdown_keymap, paste_url_as_link,
        },
    },
    setup::{SetupCommand, setup_keymap},
    view::keymap::KeyBinding,
};

use super::{
    LrDataFacet, NativeLanguage, NativeSyntax, ScriptCompletionSource, SupportedDocument,
    attached_language_snapshot, html_install, metadata,
    route::{self, HtmlDataSite},
};

static DATA: LazyLock<LrDataFacet> =
    LazyLock::new(|| LrDataFacet::new(Some(metadata(NativeLanguage::Markdown))));
static INSTALLED: LazyLock<Facet<(), bool>> =
    LazyLock::new(|| Facet::new(|inputs| !inputs.is_empty()));

pub(super) fn extension() -> Extension {
    static BUNDLE: LazyLock<Extension> = LazyLock::new(|| {
        let tags: ScriptCompletionSource = Arc::new(complete_tag);
        let keys = markdown_keymap()
            .map(|key| KeyBinding::new(key.key.unwrap(), SetupCommand::Markdown(key.run.unwrap())));
        Extension::group([
            NativeLanguage::Markdown
                .support()
                .tracked_syntax_extension(),
            html_install::support(),
            fold_service(|state, start, end| {
                let active = attached_language_snapshot(state)?;
                let NativeSyntax::Markdown(host) = active.syntax() else {
                    return None;
                };
                host.heading_fold(start, end)
            }),
            INSTALLED.of(()),
            setup_keymap(Vec::from(keys)).with_precedence(Precedence::High),
            DATA.of(LanguageData::default().with("autocomplete", tags)),
        ])
    });
    BUNDLE.clone()
}

pub(super) fn data(state: &EditorState) -> Vec<Arc<LanguageData>> {
    DATA.read(state)
}

fn complete_tag(state: &EditorState, at: usize, explicit: bool) -> Option<SourceResult> {
    let active = attached_language_snapshot(state)?;
    let NativeSyntax::Markdown(doc) = active.syntax() else {
        return None;
    };
    if !matches!(
        route::html_data_site(&active, at, Assoc::Before),
        Some(HtmlDataSite::MarkdownRoot)
    ) {
        return None;
    }
    markdown_completion_source(doc, at, explicit)
}

pub(crate) fn mounted_fold(
    state: &EditorState,
    active: &SupportedDocument,
    start: usize,
    end: usize,
) -> Option<Range<usize>> {
    installed(state)?;
    let map = route::markdown_html_projection(active, end, Assoc::Before)?;
    let range = map
        .document
        .foldable(map.project_boundary(start), map.project_boundary(end))?;
    // A fold can contain quote markup. Its endpoints must belong to the mount.
    map.selection_bounds(range)
}

pub(crate) fn mounted_indent(
    state: &EditorState,
    active: &SupportedDocument,
    lines: &IndentContext<'_>,
) -> Option<usize> {
    installed(state)?;
    let at = lines.position();
    let map = route::markdown_html_projection(active, at, Assoc::Before)?;
    let (_, local) = map.at(at, Assoc::Before)?;
    let options = lines.options();
    if options.override_indentation.is_none() && options.simulate_break.is_none() {
        return map.document.indentation(local, lines.settings(), options);
    }
    let projected = map.source(0..map.document.text().len()).filter(|range| {
        lines
            .text()
            .line_at(range.start)
            .is_some_and(|line| line.range.start == range.start)
            && matches!(
                route::html_data_site(active, at, Assoc::Before),
                Some(HtmlDataSite::Host { .. })
            )
    });
    let lookup = |line| {
        let origin = projected.as_ref()?.start;
        options
            .override_indentation
            .and_then(|get| get(origin + line))
    };
    let simulate_break = if let Some(point) = options.simulate_break {
        let range = projected.as_ref()?;
        if !range.contains(&point) || splits_crlf(lines.text().as_str(), point) {
            return None;
        }
        Some(point - range.start)
    } else {
        None
    };
    let local_options = IndentOptions {
        override_indentation: options
            .override_indentation
            .map(|_| &lookup as &dyn Fn(usize) -> Option<usize>),
        simulate_break,
        simulate_double_break: options.simulate_double_break,
    };
    map.document
        .indentation(local, lines.settings(), local_options)
}

pub(crate) fn plan_markup(state: &EditorState, command: MarkdownMarkupCommand) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let Some(active) = attached_language_snapshot(state) else {
        return Ok(None);
    };
    let NativeSyntax::Markdown(doc) = active.syntax() else {
        return Ok(None);
    };
    if state.selection().ranges().iter().any(|range| {
        let at = range.range().start;
        let markdown = |side| {
            matches!(
                route::html_data_site(&active, at, side),
                Some(HtmlDataSite::MarkdownRoot)
            )
        };
        !markdown(Assoc::Before)
            && (command != MarkdownMarkupCommand::Continue || !markdown(Assoc::After))
    }) {
        return Ok(None);
    }
    command.run(doc, state)
}

fn installed(state: &EditorState) -> Option<()> {
    state
        .context()
        .facet(&INSTALLED)
        .ok()?
        .as_ref()
        .then_some(())
}

pub(crate) fn plan_paste(state: &EditorState, clipboard: &str) -> Option<Transaction> {
    // Bare tracked Markdown and inactive Markdown ancestors do not install paste.
    installed(state)?;
    let active = attached_language_snapshot(state)?;
    let NativeSyntax::Markdown(doc) = active.syntax() else {
        return None;
    };
    paste_url_as_link(doc, state, clipboard)
}
