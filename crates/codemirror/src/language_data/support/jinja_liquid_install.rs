use std::sync::{Arc, LazyLock};

use crate::{
    EditorState, Extension,
    language::{
        LanguageData,
        jinja::{jinja_completion_source, plan_percent_brace_input},
        line_projection::splits_crlf,
        liquid::liquid_completion_source,
    },
    view::input::headless_input_handler,
};

use super::{
    CloseBracketData, LrDataFacet, NativeLanguage, NativeSyntax, ScriptCompletionSource,
    attached_language_snapshot, html_install, metadata,
};

fn facet(language: NativeLanguage) -> &'static LrDataFacet {
    static JINJA: LazyLock<LrDataFacet> =
        LazyLock::new(|| LrDataFacet::new(Some(metadata(NativeLanguage::Jinja))));
    static LIQUID: LazyLock<LrDataFacet> =
        LazyLock::new(|| LrDataFacet::new(Some(metadata(NativeLanguage::Liquid))));
    match language {
        NativeLanguage::Jinja => &JINJA,
        NativeLanguage::Liquid => &LIQUID,
        _ => unreachable!("stock Jinja or Liquid bundle"),
    }
}

pub(super) fn data(state: &EditorState, language: NativeLanguage) -> Vec<Arc<LanguageData>> {
    facet(language).read(state)
}

pub(super) fn extension(language: NativeLanguage) -> Extension {
    static JINJA: LazyLock<Extension> = LazyLock::new(|| install(NativeLanguage::Jinja));
    static LIQUID: LazyLock<Extension> = LazyLock::new(|| install(NativeLanguage::Liquid));
    match language {
        NativeLanguage::Jinja => JINJA.clone(),
        NativeLanguage::Liquid => LIQUID.clone(),
        _ => unreachable!("stock Jinja or Liquid bundle"),
    }
}

fn install(language: NativeLanguage) -> Extension {
    let source: ScriptCompletionSource = Arc::new(move |state, at, explicit| {
        let active = attached_language_snapshot(state)?;
        let text = active.text();
        if !text.is_char_boundary(at) || splits_crlf(text.as_str(), at) {
            return None;
        }
        // The facet selects the source. The callable resolves left-biased
        // template syntax even when the facet was selected from the right.
        let result = match (language, active.syntax()) {
            (NativeLanguage::Jinja, NativeSyntax::Jinja(doc)) => {
                jinja_completion_source(doc, at, explicit)
            }
            (NativeLanguage::Liquid, NativeSyntax::Liquid(doc)) => {
                liquid_completion_source(doc, at, explicit)
            }
            _ => None,
        }?;
        text.is_valid_range(result.from..result.to.unwrap_or(at))
            .then_some(result)
    });
    Extension::group([
        language.support().tracked_syntax_extension(),
        html_install::support(),
        facet(language).of(LanguageData::default().with("autocomplete", source)),
        html_install::data_input(LanguageData::default().with(
            "closeBrackets",
            CloseBracketData {
                brackets: Some(&["{"]),
                ..Default::default()
            },
        )),
        headless_input_handler(plan_percent_brace_input),
    ])
}
