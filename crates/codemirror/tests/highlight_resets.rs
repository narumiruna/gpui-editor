//! Computed styles from Chrome 158.0.8127.37, not just class names or CSS text.
//! `capture_browser_fixture.mjs` verifies @codemirror/language 6.13.1 and its
//! dependency closure against the SHA-512 pins in docs/codemirror-upstream.json.
//! The capture asserts competing classes are on one unnested span.

use gpui_editor_codemirror::{
    language::{
        Color,
        HighlightLayer::{Active, Fallback, Resolved},
        HighlightProperties, HighlightResets, HighlightRule, HighlightSelector as Tag,
        HighlightStyle, HighlightTag, HighlightTheme, TextDecoration, highlighting_for,
        highlighting_for_native,
    },
    language_data::NativeLanguage,
};
use serde_json::Value;

const EMPTY: HighlightProperties = HighlightProperties {
    foreground: None,
    bold: false,
    italic: false,
    text_decoration: None,
};
const BASE: HighlightProperties = HighlightProperties {
    foreground: Some(Color::from_rgba32(0x234567ff)),
    bold: true,
    italic: true,
    text_decoration: Some(TextDecoration::Underline),
};
const RESET: HighlightResets = HighlightResets {
    normal_weight: true,
    normal_style: true,
    no_decoration: true,
};
const POSITIVE: HighlightRule<'static> = HighlightRule::new(&[Tag::new("keyword")], BASE);
const RESET_KEYWORD: HighlightRule<'static> =
    HighlightRule::new(&[Tag::new("keyword")], EMPTY).with_resets(RESET);
const RESET_COMMENT: HighlightRule<'static> =
    HighlightRule::new(&[Tag::new("comment")], EMPTY).with_resets(RESET);
const DECORATED: HighlightStyle<'static> = HighlightStyle::define(&[POSITIVE]);
const CLEARED: HighlightStyle<'static> = HighlightStyle::define(&[RESET_KEYWORD]);
const ALL_CLEAR: HighlightStyle<'static> = HighlightStyle::define(&[]).with_all_resets(RESET);

#[test]
fn same_span_resets_match_pinned_chromium() {
    const PARTIAL: HighlightStyle<'static> =
        HighlightStyle::define(&[
            HighlightRule::new(&[Tag::new("keyword")], EMPTY).with_resets(HighlightResets {
                normal_weight: true,
                ..HighlightResets::NONE
            }),
        ]);
    const PER_PROPERTY: HighlightStyle<'static> = HighlightStyle::define(&[
        POSITIVE,
        RESET_COMMENT,
        HighlightRule::new(
            &[Tag::new("atom")],
            HighlightProperties {
                bold: true,
                ..EMPTY
            },
        ),
    ]);
    const DUPLICATE: HighlightStyle<'static> = HighlightStyle::define(&[
        POSITIVE,
        RESET_COMMENT,
        HighlightRule::new(
            &[Tag::new("comment")],
            HighlightProperties {
                foreground: BASE.foreground,
                ..EMPTY
            },
        ),
    ]);
    const SCOPED: HighlightStyle<'static> = ALL_CLEAR.with_native_scope(NativeLanguage::Json);
    let rows = include_str!("fixtures/highlight_resets.jsonl");
    for line in rows.lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let tags: Vec<_> = row["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tag| HighlightTag::new(tag.as_str().unwrap()))
            .collect();
        let expected = HighlightProperties {
            foreground: match row["color"].as_str().unwrap() {
                "rgb(35, 69, 103)" => BASE.foreground,
                "rgb(16, 32, 48)" => None, // The line's base color, not a highlight declaration.
                value => panic!("unexpected color: {value}"),
            },
            bold: match row["weight"].as_str().unwrap() {
                "700" => true,
                "400" => false,
                value => panic!("unexpected weight: {value}"),
            },
            italic: match row["style"].as_str().unwrap() {
                "italic" => true,
                "normal" => false,
                value => panic!("unexpected style: {value}"),
            },
            text_decoration: match row["decoration"].as_str().unwrap() {
                "underline" => Some(TextDecoration::Underline),
                "none" => None,
                value => panic!("unexpected decoration: {value}"),
            },
        };
        let mut scope = Some(NativeLanguage::Json);
        let layers = match name {
            "positive" => vec![Active(DECORATED)],
            "layers_reset_first" => vec![Active(CLEARED), Active(DECORATED)],
            "layers_reset_last" => vec![Active(DECORATED), Active(CLEARED)],
            "layers_partial" => vec![Active(PARTIAL), Active(DECORATED)],
            "rules_reset_last" | "rules_reverse_tags" => {
                vec![Active(HighlightStyle::define(&[POSITIVE, RESET_COMMENT]))]
            }
            "rules_reset_first" => vec![Active(HighlightStyle::define(&[RESET_COMMENT, POSITIVE]))],
            "rules_per_property" => vec![Active(PER_PROPERTY)],
            "duplicate_selector" => {
                vec![Active(HighlightStyle::define(&[POSITIVE, RESET_KEYWORD]))]
            }
            "duplicate_replaces_reset" => vec![Active(DUPLICATE)],
            "rule_resets_all" => vec![Active(
                HighlightStyle::define(&[RESET_COMMENT]).with_all(BASE),
            )],
            "rule_over_all_resets" => vec![Active(DECORATED.with_all_resets(RESET))],
            "all_resets_other_layer" => vec![Active(ALL_CLEAR), Active(DECORATED)],
            "all_resets_untagged" => vec![
                Active(ALL_CLEAR),
                Active(HighlightStyle::define(&[]).with_all(BASE)),
            ],
            "scope_miss" => {
                scope = Some(NativeLanguage::Yaml);
                vec![Active(SCOPED), Active(DECORATED)]
            }
            "scope_match" => vec![Active(SCOPED), Active(DECORATED)],
            "scope_miss_suppresses_fallback" => {
                scope = None;
                vec![Active(SCOPED), Fallback(DECORATED)]
            }
            "theme_miss_allows_fallback" => vec![
                Active(ALL_CLEAR.with_theme(HighlightTheme::Dark)),
                Fallback(DECORATED),
            ],
            _ => panic!("unexpected case: {name}"),
        };
        assert_eq!(
            highlighting_for_native(&layers, &tags, scope, HighlightTheme::Light),
            expected,
            "{name}"
        );
    }
}

#[test]
fn tag_scope_and_resolved_layers_keep_legacy_absence_semantics() {
    let scope = HighlightTag::new("test");
    let tags = [HighlightTag::new("keyword")];
    let layers = [
        Resolved(EMPTY),
        Active(CLEARED.with_scope(&scope)),
        Resolved(BASE),
    ];
    let cleared = HighlightProperties {
        foreground: BASE.foreground,
        ..EMPTY
    };
    assert_eq!(
        highlighting_for(&layers, &tags, Some(&scope), HighlightTheme::Light),
        cleared
    );
    assert_eq!(
        highlighting_for(&layers, &tags, None, HighlightTheme::Light),
        BASE
    );
    assert_eq!(
        HighlightStyle::define(&[RESET_KEYWORD])
            .with_all(BASE)
            .resolve(&tags, None),
        cleared
    );
}
