//! Node verified the npm tarballs against the pinned SHA-512 values and
//! loaded their modules. Headless Chromium mounted the extensions in isolated
//! editor views and measured computed styles for upstream CSS rules,
//! `highlightingFor` classes, and the syntax spans in a rendered JSON document.

use gpui_editor_codemirror::{
    EditorState, Text,
    language::{
        Color, DEFAULT_HIGHLIGHT_STYLE, HighlightLayer, HighlightProperties, HighlightRule,
        HighlightSelector as Tag, HighlightStyle, HighlightTag, HighlightTheme, TagModifier,
        TextDecoration, highlighting_for, json::JsonDocument,
    },
    setup::{basic_setup, minimal_setup},
    theme_one_dark,
    view::keymap::KeyPlatform,
};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/default_highlight_chromium.json")).unwrap()
}

#[test]
fn oracle_uses_the_pinned_packages_and_exports() {
    let oracle = fixture();
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let inventory: Value = serde_json::from_str(include_str!(
        "../../../docs/codemirror-upstream-exports.json"
    ))
    .unwrap();
    let packages = manifest["packages"].as_array().unwrap();
    let references = manifest["referenceDependencies"].as_array().unwrap();
    let provenance = &oracle["provenance"];
    for (name, version) in provenance["packages"].as_object().unwrap() {
        if name != "style-mod" {
            let pin = packages
                .iter()
                .chain(references)
                .find(|package| package["name"] == name.as_str())
                .unwrap();
            assert_eq!(version, &pin["version"], "{name}");
        }
    }
    let name = "@codemirror/language";
    let pin = packages
        .iter()
        .find(|package| package["name"] == name)
        .unwrap();
    assert_eq!(provenance["tarball"], pin["tarball"]);
    assert_eq!(provenance["integrity"], pin["integrity"]);
    let exports = inventory["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == name)
        .unwrap();
    assert_eq!(oracle["exports"], exports["modules"][0]["exports"]);
}

const MODIFIERS: [(&str, TagModifier); 6] = [
    ("definition", TagModifier::Definition),
    ("constant", TagModifier::Constant),
    ("function", TagModifier::Function),
    ("standard", TagModifier::Standard),
    ("local", TagModifier::Local),
    ("special", TagModifier::Special),
];

fn tag(name: &str, mask: u64) -> HighlightTag {
    // Reverse the insertion order to verify canonical modifier ordering.
    MODIFIERS.iter().enumerate().rev().fold(
        HighlightTag::new(name),
        |tag, (index, &(_, modifier))| {
            if mask & (1 << index) == 0 {
                tag
            } else {
                tag.with_modifier(modifier)
            }
        },
    )
}

fn selector_tag(value: &Value) -> HighlightTag {
    if let Some(name) = value.as_str() {
        tag(name, 0)
    } else {
        tag(value[0].as_str().unwrap(), value[1].as_u64().unwrap())
    }
}

fn properties(value: &Value) -> HighlightProperties {
    HighlightProperties {
        foreground: value[0]
            .as_str()
            .map(|rgba| Color::from_rgba32(u32::from_str_radix(rgba, 16).unwrap())),
        bold: value[1].as_bool().unwrap(),
        italic: value[2].as_bool().unwrap(),
        text_decoration: value[3].as_str().map(|line| match line {
            "underline" => TextDecoration::Underline,
            "line-through" => TextDecoration::LineThrough,
            _ => panic!("unexpected decoration: {line}"),
        }),
    }
}

#[test]
fn every_default_rule_matches_the_mounted_css() {
    let oracle = fixture();
    let rules = oracle["rules"].as_array().unwrap();
    assert_eq!(rules.len(), 19);
    for rule in rules {
        let expected = properties(&rule["properties"]);
        for selector in rule["selectors"].as_array().unwrap() {
            assert_eq!(
                DEFAULT_HIGHLIGHT_STYLE.resolve(&[selector_tag(selector)], None),
                expected,
                "{selector}",
            );
        }
    }
}

#[test]
fn default_style_matches_each_builtin_tag_and_modifier_subset() {
    let oracle = fixture();
    assert_eq!(oracle["modifiers"], json!(MODIFIERS.map(|(name, _)| name)));
    let matrix = oracle["matrix"].as_object().unwrap();
    assert_eq!(matrix.len(), 78);
    for (name, row) in matrix {
        let row = row.as_str().unwrap();
        assert!(matches!(row.len(), 1 | 64));
        for mask in 0..64 {
            let offset = if row.len() == 1 { 0 } else { mask };
            let code = &row[offset..offset + 1];
            assert_eq!(
                DEFAULT_HIGHLIGHT_STYLE.resolve(&[tag(name, mask.try_into().unwrap())], None),
                properties(&oracle["styleCodes"][code]),
                "{name}: {mask}",
            );
        }
    }
}

fn check_cascades(style: HighlightStyle<'_>, cases: &Value) {
    for case in cases.as_array().unwrap() {
        let mut tags: Vec<_> = case["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(selector_tag)
            .collect();
        let expected = properties(&case["properties"]);
        assert_eq!(style.resolve(&tags, None), expected, "{case}");
        tags.reverse();
        assert_eq!(style.resolve(&tags, None), expected, "{case}");
    }
}

#[test]
fn separate_tags_follow_default_css_rule_order() {
    check_cascades(DEFAULT_HIGHLIGHT_STYLE, &fixture()["cascades"]);
}

const EMPTY: HighlightProperties = HighlightProperties {
    foreground: None,
    bold: false,
    italic: false,
    text_decoration: None,
};

const fn rgb(color: u32) -> HighlightProperties {
    HighlightProperties {
        foreground: Some(Color::from_rgba32((color << 8) | 0xff)),
        ..EMPTY
    }
}

#[test]
fn custom_rules_resolve_specificity_duplicate_selectors_order_and_all() {
    use TagModifier::{Constant, Definition, Function, Special, Standard};

    const VARIABLE: Tag<'static> = Tag::new("variableName");
    const DEFINITION_STANDARD: Tag<'static> =
        VARIABLE.with_modifier(Definition).with_modifier(Standard);
    const RULES: &[HighlightRule<'static>] = &[
        HighlightRule::new(
            &[Tag::new("name")],
            HighlightProperties {
                bold: true,
                ..rgb(0x991100)
            },
        ),
        HighlightRule::new(&[DEFINITION_STANDARD], rgb(0x994400)),
        HighlightRule::new(
            &[VARIABLE.with_modifier(Constant).with_modifier(Function)],
            rgb(0x559900),
        ),
        HighlightRule::new(&[DEFINITION_STANDARD.with_modifier(Special)], rgb(0x999900)),
        HighlightRule::new(
            &[Tag::new("brace")],
            HighlightProperties {
                text_decoration: Some(TextDecoration::LineThrough),
                ..EMPTY
            },
        ),
        HighlightRule::new(
            &[Tag::new("name")],
            HighlightProperties {
                italic: true,
                ..EMPTY
            },
        ),
        HighlightRule::new(
            &[Tag::new("link")],
            HighlightProperties {
                text_decoration: Some(TextDecoration::Underline),
                ..rgb(0x009999)
            },
        ),
        HighlightRule::new(
            &[Tag::new("name").with_modifier(Definition)],
            HighlightProperties {
                bold: true,
                ..rgb(0x994499)
            },
        ),
        HighlightRule::new(&[VARIABLE], rgb(0x440099)),
    ];
    let style = HighlightStyle::define(RULES).with_all(HighlightProperties {
        italic: true,
        ..rgb(0x123456)
    });
    check_cascades(style, &fixture()["orderedCases"]);
}

#[test]
fn scopes_match_native_tags_by_value_not_ancestry_or_identity() {
    let scope = HighlightTag::new("variableName").with_modifier(TagModifier::Definition);
    let custom = HighlightTag::new("extension:special");
    let selectors = [Tag::from(&custom)];
    let rules = [HighlightRule::new(&selectors, rgb(0x123456))];
    let style = HighlightStyle::define(&rules).with_scope(&scope);
    assert_eq!(
        style.resolve(std::slice::from_ref(&custom), Some(&scope.clone())),
        rgb(0x123456)
    );
    assert_eq!(
        style.resolve(
            std::slice::from_ref(&custom),
            Some(&HighlightTag::new(scope.name()))
        ),
        EMPTY
    );
    assert_eq!(
        style.resolve(
            std::slice::from_ref(&custom),
            Some(&HighlightTag::new("name"))
        ),
        EMPTY
    );
    assert_eq!(style.resolve(std::slice::from_ref(&custom), None), EMPTY);
}

#[test]
fn native_layers_match_chromium_fallback_scope_and_appearance_selection() {
    use HighlightLayer::{Active, Fallback, Resolved};
    use HighlightTheme::{Dark, Light};

    const KEYWORD: HighlightStyle<'static> = HighlightStyle::define(&[HighlightRule::new(
        &[Tag::new("keyword")],
        HighlightProperties {
            italic: true,
            ..rgb(0x654321)
        },
    )]);
    const PUNCTUATION: HighlightStyle<'static> = HighlightStyle::define(&[HighlightRule::new(
        &[Tag::new("punctuation")],
        HighlightProperties {
            bold: true,
            ..rgb(0xaabb33)
        },
    )]);
    const DECORATION: HighlightStyle<'static> = HighlightStyle::define(&[HighlightRule::new(
        &[Tag::new("link")],
        HighlightProperties {
            text_decoration: Some(TextDecoration::LineThrough),
            ..EMPTY
        },
    )]);
    let json_scope = HighlightTag::new("JsonText");
    let scoped = KEYWORD.with_scope(&json_scope);
    let language = Active(scoped.with_all(HighlightProperties {
        bold: true,
        ..EMPTY
    }));
    let dark = Active(KEYWORD.with_theme(Dark));
    let light = Active(PUNCTUATION.with_theme(Light));
    let fallback = Fallback(DEFAULT_HIGHLIGHT_STYLE);
    let default_active = Active(DEFAULT_HIGHLIGHT_STYLE);
    let basic = basic_setup::<()>(EditorState::new(""), KeyPlatform::Mac).unwrap();
    let minimal = minimal_setup::<()>(EditorState::new(""), KeyPlatform::Mac);
    for case in fixture()["layerCases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let theme = match case["appearance"].as_str().unwrap() {
            "light" => Light,
            "dark" => Dark,
            appearance => panic!("unexpected appearance: {appearance}"),
        };
        let scope = case["scope"].as_str().map(HighlightTag::new);
        for (tag, expected) in case["properties"].as_object().unwrap() {
            let tags = [HighlightTag::new(tag)];
            let one_dark = Resolved(theme_one_dark::highlight_style(&tags));
            let layers: &[HighlightLayer<'_>] = match name {
                "plain" => &[],
                "basic" => &[basic.highlight_fallback()],
                "minimal" => &[minimal.highlight_fallback()],
                "appearance" => &[fallback, dark, light],
                "themeMiss" => &[fallback, dark],
                "emptyActive" => &[fallback, Active(HighlightStyle::define(&[]))],
                "fallbackFirst" => &[fallback, Fallback(PUNCTUATION)],
                "customFallbackFirst" => &[Fallback(PUNCTUATION), basic.highlight_fallback()],
                "fallbackIgnoresAppearance" => &[Fallback(KEYWORD.with_theme(Dark)), fallback],
                "fallbackScopeMiss" => &[Fallback(scoped), fallback],
                "scopeMatch" | "scopeMiss" | "scopeAbsent" => &[fallback, language],
                "exactScope" | "exactScopeMiss" => &[fallback, Active(scoped)],
                "differentStyles" => &[fallback, language, light],
                "decorationFirst" => &[Active(DECORATION), default_active],
                "decorationSecond" => &[default_active, Active(DECORATION)],
                "oneDark" => &[basic.highlight_fallback(), one_dark],
                "oneDarkFirst" => &[basic.highlight_fallback(), one_dark, dark],
                "oneDarkSecond" => &[dark, basic.highlight_fallback(), one_dark],
                _ => panic!("unexpected layer case: {name}"),
            };
            assert_eq!(
                highlighting_for(layers, &tags, scope.as_ref(), theme),
                properties(expected),
                "{name}: {tag} ({theme:?})"
            );
        }
    }
}

#[test]
fn default_style_passes_native_json_spans_to_a_host_without_a_view() {
    let fixture = fixture();
    let rendered = &fixture["renderedJson"];
    let source = rendered["source"].as_str().unwrap();
    let doc = JsonDocument::parse(Text::new(source));
    let layers = [HighlightLayer::Fallback(DEFAULT_HIGHLIGHT_STYLE)];
    let ranges = doc.highlights(0..source.len()).unwrap().filter_map(|span| {
        let style = highlighting_for(&layers, span.tags, None, HighlightTheme::Light);
        (style != EMPTY).then_some((span.range, style))
    });
    let byte_offset = |position: &Value| {
        let target: usize = position.as_u64().unwrap().try_into().unwrap();
        let mut utf16 = 0;
        for (byte, ch) in source.char_indices() {
            if utf16 == target {
                return byte;
            }
            utf16 += ch.len_utf16();
        }
        assert_eq!(utf16, target);
        source.len()
    };
    let expected: Vec<_> = rendered["spans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|span| {
            (
                byte_offset(&span["from"])..byte_offset(&span["to"]),
                properties(&span["properties"]),
            )
        })
        .collect();
    assert_eq!(ranges.collect::<Vec<_>>(), expected);
}
