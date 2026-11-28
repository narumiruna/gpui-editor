use std::sync::LazyLock;

use crate::{
    Extension,
    language::{LanguageData, angular, vue},
};

use super::{CloseBracketData, NativeLanguage, html_install};

pub(super) fn extension(language: NativeLanguage) -> Extension {
    static VUE: LazyLock<Extension> = LazyLock::new(|| {
        install(
            NativeLanguage::Vue,
            LanguageData::default().with(
                "closeBrackets",
                CloseBracketData {
                    brackets: Some(&vue::CLOSE_BRACKETS),
                    ..Default::default()
                },
            ),
        )
    });
    static ANGULAR: LazyLock<Extension> = LazyLock::new(|| {
        install(
            NativeLanguage::Angular,
            LanguageData::default()
                .with(
                    "closeBrackets",
                    CloseBracketData {
                        brackets: Some(&["[", "{", "\""]),
                        ..Default::default()
                    },
                )
                .with(
                    "indentOnInput",
                    angular::template_indent_on_input as fn(&str) -> bool,
                ),
        )
    });
    match language {
        NativeLanguage::Vue => VUE.clone(),
        NativeLanguage::Angular => ANGULAR.clone(),
        _ => unreachable!("stock HTML template bundle"),
    }
}

fn install(language: NativeLanguage, overlay: LanguageData) -> Extension {
    Extension::group([
        language.support().tracked_syntax_extension(),
        html_install::support(),
        html_install::data_input(overlay),
    ])
}
