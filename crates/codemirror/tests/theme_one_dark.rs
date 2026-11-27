//! Data from the SHA-512-verified theme tarball in headless Chromium.
//!
//! The scratch run mounted each extension set in an isolated EditorView,
//! read all emitted CSS rules, and measured computed properties on matching
//! view elements and syntax spans. The fixture records this browser output.
//! The Rust tests compare the native roles and supported highlight properties;
//! the host observations do not assert that Rust renders browser widgets.

use std::collections::BTreeMap;

use gpui_editor_codemirror::{
    language::{HighlightTag, TagModifier},
    theme_one_dark::{
        COLOR, Color, EditorSurfaceColors, HighlightLayer, HighlightStyle, ONE_DARK,
        ONE_DARK_THEME, Stroke, TextDecoration, editor_surface_colors, highlight_style,
        layered_highlight_style, override_editor_surface,
    },
};
use serde_json::{Value, json};

const HOST_SURFACE: EditorSurfaceColors = EditorSurfaceColors {
    foreground: Some(Color::from_rgba32(0x3040_50ff)),
    background: Some(Color::from_rgba32(0x1020_30ff)),
};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/theme_one_dark_chromium.json")).unwrap()
}

#[test]
fn chromium_fixture_matches_the_pinned_artifact_and_export_inventory() {
    let oracle = fixture();
    let manifest: Value =
        serde_json::from_str(include_str!("../../../docs/codemirror-upstream.json")).unwrap();
    let inventory: Value = serde_json::from_str(include_str!(
        "../../../docs/codemirror-upstream-exports.json"
    ))
    .unwrap();
    let name = "@codemirror/theme-one-dark";
    let pin = manifest["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == name)
        .unwrap();
    assert_eq!(oracle["provenance"]["tarball"], pin["tarball"]);
    assert_eq!(oracle["provenance"]["integrity"], pin["integrity"]);
    assert_eq!(oracle["provenance"]["packages"][name], pin["version"]);
    let package = inventory["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == name)
        .unwrap();
    assert_eq!(oracle["exports"], package["modules"][0]["exports"]);
    let reconfigure = reconfigure_fixture();
    assert_eq!(reconfigure["exports"], oracle["exports"]);
    for key in ["tarball", "integrity"] {
        assert_eq!(reconfigure["provenance"][key], pin[key]);
    }
    for (name, version) in reconfigure["provenance"]["packages"].as_object().unwrap() {
        assert_eq!(version, &oracle["provenance"]["packages"][name], "{name}");
    }
    assert_eq!(oracle["viewRuleCount"], 19);
    assert_eq!(oracle["viewSelectorBranches"], 23);
    assert_eq!(oracle["highlightRuleCount"], 16);
}

#[test]
fn exported_palette_matches_upstream_color() {
    let oracle = fixture();
    let palette = oracle["palette"].as_object().unwrap();
    let colors = [
        ("chalky", COLOR.chalky),
        ("coral", COLOR.coral),
        ("cyan", COLOR.cyan),
        ("invalid", COLOR.invalid),
        ("ivory", COLOR.ivory),
        ("stone", COLOR.stone),
        ("malibu", COLOR.malibu),
        ("sage", COLOR.sage),
        ("whiskey", COLOR.whiskey),
        ("violet", COLOR.violet),
        ("darkBackground", COLOR.dark_background),
        ("highlightBackground", COLOR.highlight_background),
        ("background", COLOR.background),
        ("tooltipBackground", COLOR.tooltip_background),
        ("selection", COLOR.selection),
        ("cursor", COLOR.cursor),
    ];
    assert_eq!(palette.len(), colors.len());
    for (name, color) in colors {
        let exported = palette[name].as_str().unwrap();
        assert_eq!(
            format!("#{:06x}", color.rgba32() >> 8),
            exported.to_ascii_lowercase(),
            "{name}",
        );
        assert_eq!(color.rgba32() & 0xff, 0xff, "{name}");
    }
}

#[test]
fn stock_view_roles_match_mounted_css_in_both_presets() {
    let theme = ONE_DARK_THEME;
    assert_eq!(ONE_DARK.view, theme);
    let oracle = fixture();
    let roles = &oracle["viewRoles"];
    for (color, keys) in [
        (theme.editor.foreground, "editor.color"),
        (theme.editor.background, "editor.backgroundColor"),
        (theme.caret, "content.caretColor"),
        (theme.cursor, "cursor.borderLeftColor"),
        (theme.drop_cursor, "dropCursor.borderLeftColor"),
        (
            theme.selection,
            "selection.focused.backgroundColor selection.unfocused.backgroundColor selection.native.backgroundColor",
        ),
        (
            theme.panels.foreground,
            "panelsTop.color panelsBottom.color",
        ),
        (
            theme.panels.background,
            "panelsTop.backgroundColor panelsBottom.backgroundColor",
        ),
        (theme.search_match.background, "searchMatch.backgroundColor"),
        (
            theme.selected_search_match.background,
            "selectedSearchMatch.backgroundColor",
        ),
        (theme.active_line, "activeLine.backgroundColor"),
        (theme.selection_match, "selectionMatch.backgroundColor"),
        (
            theme.matching_bracket,
            "matchingBracket.focused.backgroundColor",
        ),
        (
            theme.nonmatching_bracket,
            "nonmatchingBracket.focused.backgroundColor",
        ),
        (theme.gutters.foreground, "gutters.color"),
        (theme.gutters.background, "gutters.backgroundColor"),
        (theme.active_line_gutter, "activeLineGutter.backgroundColor"),
        (theme.fold_placeholder.foreground, "foldPlaceholder.color"),
        (
            theme.fold_placeholder.background,
            "foldPlaceholder.backgroundColor",
        ),
        (theme.tooltip.background, "tooltip.backgroundColor"),
        (
            theme.tooltip.arrow_before.top,
            "tooltip.arrowBefore.borderTopColor tooltip.arrowBeforeBelow.borderTopColor",
        ),
        (
            theme.tooltip.arrow_before.bottom,
            "tooltip.arrowBefore.borderBottomColor tooltip.arrowBeforeBelow.borderBottomColor",
        ),
        (
            theme.tooltip.arrow_after.top,
            "tooltip.arrowAfter.borderTopColor tooltip.arrowAfterBelow.borderTopColor",
        ),
        (
            theme.tooltip.arrow_after.bottom,
            "tooltip.arrowAfter.borderBottomColor tooltip.arrowAfterBelow.borderBottomColor",
        ),
        (
            theme.selected_completion.foreground,
            "completion.selected.color completion.selectedFalse.color",
        ),
        (
            theme.selected_completion.background,
            "completion.selected.backgroundColor completion.selectedFalse.backgroundColor",
        ),
    ] {
        let actual = format!("{:08x}", color.rgba32());
        for key in keys.split_whitespace() {
            assert_eq!(roles[key], actual, "{key}");
        }
    }
    let stroke = |style: Stroke| {
        json!([
            style.width_px,
            "solid",
            format!("{:08x}", style.color.rgba32())
        ])
    };
    for (key, style) in [
        ("panelsTop.borderBottom", Some(theme.panels.top_border)),
        ("panelsBottom.borderTop", Some(theme.panels.bottom_border)),
        ("searchMatch.outline", Some(theme.search_match.outline)),
        (
            "selectedSearchMatch.outline",
            Some(theme.selected_search_match.outline),
        ),
        ("gutters.border", theme.gutters.border),
        ("foldPlaceholder.border", theme.fold_placeholder.border),
        ("tooltip.border", theme.tooltip.border),
    ] {
        assert_eq!(
            roles.get(key).unwrap(),
            &style.map_or(Value::Null, stroke),
            "{key}"
        );
    }
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
    // Add modifiers in reverse to check that caller order has no effect.
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

fn style(value: &Value) -> HighlightStyle {
    HighlightStyle {
        foreground: value["color"]
            .as_str()
            .map(|hex| Color::from_rgba32(u32::from_str_radix(hex, 16).unwrap())),
        bold: value["fontWeight"] == "700",
        italic: value["fontStyle"] == "italic",
        text_decoration: value["textDecorationLine"].as_str().map(|line| match line {
            "underline" => TextDecoration::Underline,
            "line-through" => TextDecoration::LineThrough,
            _ => panic!("unexpected decoration: {line}"),
        }),
    }
}

#[test]
fn every_highlight_rule_matches_the_upstream_direct_selectors() {
    let oracle = fixture();
    let rules = oracle["highlightRules"].as_array().unwrap();
    assert_eq!(rules.len(), 16);
    let mut selectors = BTreeMap::new();
    for rule in rules {
        for selector in rule["tags"].as_array().unwrap() {
            // The later link rule replaces the cyan selector for the same tag.
            selectors.insert(selector.as_str().unwrap(), style(&rule["style"]));
        }
    }
    for (selector, expected) in selectors {
        let tag = if let Some((modifier, name)) = selector.split_once('(') {
            let index = MODIFIERS
                .iter()
                .position(|&(name, _)| name == modifier)
                .unwrap();
            tag(name.strip_suffix(')').unwrap(), 1 << index)
        } else {
            tag(selector, 0)
        };
        assert_eq!(highlight_style(&[tag]), expected, "{selector}");
    }
}

#[test]
fn standard_tag_fallbacks_match_in_light_and_dark_editors() {
    let oracle = fixture();
    assert_eq!(oracle["modifiers"], json!(MODIFIERS.map(|(name, _)| name)));
    let matrix = oracle["matrix"].as_object().unwrap();
    assert_eq!(matrix.len(), 78);
    for (name, row) in matrix {
        let row = row.as_str().unwrap().as_bytes();
        assert!(matches!(row.len(), 1 | 64));
        for mask in 0..64 {
            let code = row[if row.len() == 1 { 0 } else { mask }];
            let expected = HighlightStyle {
                foreground: match code {
                    b'C' | b'H' => Some(COLOR.coral),
                    b'I' => Some(COLOR.ivory),
                    b'M' => Some(COLOR.malibu),
                    b'W' => Some(COLOR.whiskey),
                    b'S' => Some(COLOR.sage),
                    b'Y' => Some(COLOR.cyan),
                    b'G' | b'U' => Some(COLOR.stone),
                    b'O' => Some(COLOR.chalky),
                    b'V' => Some(COLOR.violet),
                    b'X' => Some(COLOR.invalid),
                    b'.' | b'b' | b'i' | b'T' => None,
                    _ => panic!("unexpected style: {code}"),
                },
                bold: matches!(code, b'H' | b'b'),
                italic: code == b'i',
                text_decoration: match code {
                    b'U' => Some(TextDecoration::Underline),
                    b'T' => Some(TextDecoration::LineThrough),
                    _ => None,
                },
            };
            let tags = [tag(name, mask.try_into().unwrap())];
            assert_eq!(highlight_style(&tags), expected, "{name} {mask}");
        }
    }
}

#[test]
fn multiple_tags_cascade_by_property_independent_of_tag_order() {
    let oracle = fixture();
    for case in oracle["cascades"].as_array().unwrap() {
        let mut tags: Vec<_> = case["tags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| tag(item[0].as_str().unwrap(), item[1].as_u64().unwrap()))
            .collect();
        let expected = style(&case["style"]);
        assert_eq!(highlight_style(&tags), expected, "{tags:?}");
        tags.reverse();
        assert_eq!(highlight_style(&tags), expected, "{tags:?}");
    }
}

#[test]
fn stock_presets_match_active_highlighter_layering() {
    use HighlightLayer::Resolved;

    let oracle = fixture();
    let layering = oracle["layering"].as_object().unwrap();
    let full = ONE_DARK.highlight;
    // Host root colors compose independently from the active syntax list.
    for (name, expected) in layering {
        let theme = match name.as_str() {
            "bareFallback"
            | "barePunctuationFallback"
            | "lightScopedFallback"
            | "darkScopedFallback"
            | "highlighterOnlyFallback"
            | "scopeMissFallback" => None,
            "themeOnlyFallback" | "themeOnlyScopedFallback" => Some(ONE_DARK_THEME),
            "viewHostFirst" | "viewHostSecond" => {
                use gpui_editor_codemirror::{
                    EditorState, Extension, theme_one_dark::one_dark_theme,
                };
                let host = override_editor_surface(HOST_SURFACE);
                let theme = one_dark_theme();
                let layers = if name == "viewHostFirst" {
                    [host, theme]
                } else {
                    [theme, host]
                };
                let state = EditorState::with_extensions("", Extension::group(layers)).unwrap();
                let colors = editor_surface_colors(&state);
                let mut theme = ONE_DARK.view;
                theme.editor.foreground = colors.foreground.unwrap();
                theme.editor.background = colors.background.unwrap();
                Some(theme)
            }
            _ => Some(ONE_DARK.view),
        };
        assert_eq!(
            expected["dark"],
            theme.is_some_and(|style| style.dark),
            "{name}"
        );
        if let Some(theme) = theme {
            assert_eq!(
                expected["background"],
                format!("{:08x}", theme.editor.background.rgba32()),
                "{name}"
            );
            assert_eq!(
                expected["foreground"],
                format!("{:08x}", theme.editor.foreground.rgba32()),
                "{name}"
            );
        }
        for token in ["keyword", "punctuation", "heading", "link"] {
            let empty = HighlightStyle::default();
            let color = |rgba| Some(Color::from_rgba32(rgba));
            let (light, dark) = if matches!(token, "keyword" | "punctuation") {
                (
                    Resolved(HighlightStyle {
                        foreground: color(0xff00_00ff),
                        bold: true,
                        ..empty
                    }),
                    Resolved(HighlightStyle {
                        foreground: color(0x0080_80ff),
                        italic: true,
                        ..empty
                    }),
                )
            } else {
                (Resolved(empty), Resolved(empty))
            };
            let auxiliary = Resolved(if token == "link" {
                HighlightStyle {
                    foreground: color(0xff88_00ff),
                    text_decoration: Some(TextDecoration::LineThrough),
                    ..empty
                }
            } else {
                empty
            });
            let active: &[HighlightLayer] = match name.as_str() {
                "bareFallback"
                | "barePunctuationFallback"
                | "darkScopedFallback"
                | "themeOnlyFallback" => &[],
                "lightScopedFallback" => &[light],
                "themeOnlyScopedFallback" => &[dark],
                "fullFallback"
                | "fullPunctuationFallback"
                | "highlighterOnlyFallback"
                | "viewHostFirst"
                | "viewHostSecond" => &[full],
                "fullScoped" => &[full, dark],
                "fullScopedDarkFirst" => &[dark, full],
                "fullAuxiliaryFirst" | "fullPrec" => &[auxiliary, full],
                "fullAuxiliarySecond" => &[full, auxiliary],
                "scopeMissFallback" => &[Resolved(empty)],
                _ => panic!("unexpected layer case: {name}"),
            };
            let fallback = if matches!(
                name.as_str(),
                "barePunctuationFallback" | "fullPunctuationFallback"
            ) {
                // Only the first fallback participates, even for an unmatched tag.
                HighlightStyle {
                    foreground: (token == "punctuation").then(|| Color::from_rgba32(0x1122_33ff)),
                    ..empty
                }
            } else {
                style(&layering["bareFallback"][token])
            };
            assert_eq!(
                layered_highlight_style(&[HighlightTag::new(token)], active, fallback),
                style(&expected[token]),
                "{name}: {token}"
            );
        }
    }
}

#[test]
fn host_surface_layers_match_pinned_chromium_reconfiguration() {
    use gpui_editor_codemirror::{
        Compartment, EditorSelection, EditorState, Extension, Precedence, Transaction,
        theme_one_dark::{one_dark_theme, view_style},
        view::EditorView,
    };

    let host = override_editor_surface(HOST_SURFACE);
    let foreground = override_editor_surface(EditorSurfaceColors {
        foreground: HOST_SURFACE.foreground,
        background: None,
    });
    let background = override_editor_surface(EditorSurfaceColors {
        foreground: None,
        background: HOST_SURFACE.background,
    });
    let scope = Compartment::new();
    let source = "一 🦀";
    let selection = EditorSelection::single(0, "一".len());
    let mut state = EditorState::with_extensions(source, scope.of(Extension::default())).unwrap();
    state
        .update(Transaction::default().with_selection(selection.clone()))
        .unwrap();
    let mut view = EditorView::new(state);
    let initial = view.state().clone();
    for line in include_str!("fixtures/one_dark_surface_browser.jsonl").lines() {
        let row: Value = serde_json::from_str(line).unwrap();
        let name = row["name"].as_str().unwrap();
        let theme = one_dark_theme();
        let layers = match name {
            "bare" | "removed" => vec![],
            "hostOnly" => vec![host.clone()],
            "viewHostFirst" => vec![host.clone(), theme],
            "viewHostSecond" => vec![theme, host.clone()],
            "onlyForegroundWins" => vec![foreground.clone(), theme, background.clone()],
            "bothPropertiesWin" => vec![background.clone(), foreground.clone(), theme],
            "higherPrecedence" => vec![theme, host.clone().with_precedence(Precedence::High)],
            "lowerPrecedence" => vec![host.clone().with_precedence(Precedence::Low), theme],
            "foregroundOnly" => vec![foreground.clone()],
            _ => panic!("unknown surface case {name}"),
        };
        view.update(
            Transaction::default().with_effects([scope.reconfigure(Extension::group(layers))]),
            (),
            |_| (None, ()),
        )
        .unwrap();
        let state = view.state();
        let hex = |key: &str| {
            Color::from_rgba32(u32::from_str_radix(row[key].as_str().unwrap(), 16).unwrap())
        };
        let core_colors = editor_surface_colors(state);
        // Unset properties come from the host, not browser default foreground/fill.
        assert_eq!(
            core_colors.foreground,
            (!matches!(name, "bare" | "removed")).then(|| hex("foreground")),
            "{name}"
        );
        assert_eq!(
            core_colors.background,
            (!matches!(name, "bare" | "removed" | "foregroundOnly")).then(|| hex("background")),
            "{name}"
        );
        assert_eq!(
            view_style(state).is_some(),
            row["dark"].as_bool().unwrap(),
            "{name}"
        );
        assert_eq!(state.text().as_str(), source);
        assert_eq!(state.selection(), &selection);
        assert_eq!(
            (view.revision(), view.undo_depth(), view.redo_depth()),
            (0, 0, 0)
        );
    }
    assert_eq!(
        editor_surface_colors(&initial),
        EditorSurfaceColors::default()
    );
}

fn reconfigure_fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/theme_one_dark_reconfigure.json")).unwrap()
}

#[test]
fn installed_view_and_highlight_channels_follow_chromium_reconfiguration() {
    use std::sync::LazyLock;

    use gpui_editor_codemirror::{
        Compartment, EditorSelection, EditorState, Extension, Precedence, Transaction,
        language::{
            DEFAULT_HIGHLIGHT_STYLE,
            HighlightLayer::{Active, Fallback},
            HighlightRule, HighlightSelector as Tag, HighlightStyle as NativeStyle, HighlightTheme,
        },
        theme_one_dark::{
            highlight_layer, highlighting_for, one_dark, one_dark_highlighting, one_dark_theme,
            view_style,
        },
        view::EditorView,
    };

    const DARK: NativeStyle<'static> = NativeStyle::define(&[HighlightRule::new(
        &[Tag::new("keyword"), Tag::new("punctuation")],
        HighlightStyle {
            foreground: Some(Color::from_rgba32(0x0080_80ff)),
            bold: false,
            italic: true,
            text_decoration: None,
        },
    )])
    .with_theme(HighlightTheme::Dark);
    const SCOPED: NativeStyle<'static> = NativeStyle::define(&[HighlightRule::new(
        &[Tag::new("keyword")],
        HighlightStyle {
            foreground: Some(Color::from_rgba32(0x0099_66ff)),
            bold: true,
            italic: false,
            text_decoration: None,
        },
    )]);
    const PUNCTUATION: NativeStyle<'static> = NativeStyle::define(&[HighlightRule::new(
        &[Tag::new("punctuation")],
        HighlightStyle {
            foreground: Some(Color::from_rgba32(0x1122_33ff)),
            bold: false,
            italic: false,
            text_decoration: None,
        },
    )]);

    static SCOPE: LazyLock<HighlightTag> = LazyLock::new(|| HighlightTag::new("Section"));
    let other_scope = HighlightTag::new("Other");
    let dark = highlight_layer(Active(DARK));
    let scoped = highlight_layer(Active(SCOPED.with_scope(&SCOPE)));
    let oracle = reconfigure_fixture();
    let source = oracle["source"].as_str().unwrap();
    let position = |key| {
        oracle["selection"][key]
            .as_u64()
            .unwrap()
            .try_into()
            .unwrap()
    };
    let selection = EditorSelection::single(position("anchor"), position("head"));
    let slots = ["view", "highlight", "extra"].map(|name| (name, Compartment::new()));
    let mut state = EditorState::with_extensions(
        source,
        Extension::group(slots.iter().map(|(_, slot)| slot.of(Extension::default()))),
    )
    .unwrap();
    state
        .update(Transaction::default().with_selection(selection.clone()))
        .unwrap();
    let mut view = EditorView::<()>::new(state);

    for step in oracle["steps"].as_array().unwrap() {
        let name = step["name"].as_str().unwrap();
        let effects = step["changes"].as_array().unwrap().iter().map(|change| {
            let slot = change[0].as_str().unwrap();
            let extension = match change[1].as_str().unwrap() {
                "appendFallback" => {
                    assert_eq!(slot, "root");
                    return highlight_layer(Fallback(DEFAULT_HIGHLIGHT_STYLE)).append();
                }
                "empty" => Extension::default(),
                "theme" => one_dark_theme(),
                "highlight" => one_dark_highlighting(),
                "preset" => one_dark(),
                "dark" => dark.clone(),
                "darkHigh" => dark.clone().with_precedence(Precedence::High),
                "scoped" => scoped.clone(),
                "punctuationFallback" => highlight_layer(Fallback(PUNCTUATION)),
                other => panic!("unexpected extension: {other}"),
            };
            slots
                .iter()
                .find(|(key, _)| *key == slot)
                .unwrap()
                .1
                .reconfigure(extension)
        });
        view.update(Transaction::default().with_effects(effects), (), |_| {
            (None, ())
        })
        .unwrap();
        let state = view.state();
        assert_eq!(state.text().as_str(), source, "{name}");
        assert_eq!(state.selection(), &selection, "{name}");
        let roles = view_style(state);
        assert_eq!(
            roles,
            step["dark"].as_bool().unwrap().then_some(&ONE_DARK_THEME),
            "{name}"
        );
        if let Some(roles) = roles {
            assert_eq!(
                step["background"],
                format!("{:08x}", roles.editor.background.rgba32()),
                "{name}"
            );
            assert_eq!(
                step["foreground"],
                format!("{:08x}", roles.editor.foreground.rgba32()),
                "{name}"
            );
        }
        for (key, tag, scope) in [
            ("keyword", "keyword", None),
            ("punctuation", "punctuation", None),
            ("link", "link", None),
            ("inScope", "keyword", Some(&*SCOPE)),
            ("otherScope", "keyword", Some(&other_scope)),
        ] {
            assert_eq!(
                highlighting_for(state, &[HighlightTag::new(tag)], scope),
                style(&step[key]),
                "{name}: {key}"
            );
        }
    }
}
