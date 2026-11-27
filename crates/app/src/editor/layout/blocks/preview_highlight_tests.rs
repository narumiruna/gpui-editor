use gpui::{Hsla, Rgba, ShapedLine, TestAppContext, WindowAppearance, px, rgb, size};
use gpui_editor_codemirror::{
    Compartment, Extension, Transaction,
    language::{HighlightLayer, HighlightRule, HighlightSelector, HighlightStyle, HighlightTheme},
    language_data::NativeLanguage,
    theme_one_dark::{highlight_layer, one_dark_theme},
};

use crate::editor::{Editor, layout::ItemPaint};

use super::{Color, HighlightProperties};

const SOURCE: &str =
    "```json\r\n{\r\n\t\"字\": \"🐱\",\r\n\t\"n\": 7\r\n}\r\n```\r\n\r\nProse.\r\n";
const STRING: &str = "\"🐱\"";
const DISPLAY: [&str; 4] = ["{", "    \"字\": \"🐱\",", "    \"n\": 7", "}"];
const LEGACY_STRING: u32 = 0x14683f;
const LEGACY_NUMBER: u32 = 0x673ab7;
const LIGHT_TEXT: u32 = 0x172336;
const DARK_TEXT: u32 = 0xabb2bf;
const CUSTOM_STRING: u32 = 0x187ba3;
const CUSTOM_NUMBER: u32 = 0x93283b;

const fn rule(tag: &'static [HighlightSelector<'static>], color: u32) -> HighlightRule<'static> {
    HighlightRule::new(
        tag,
        HighlightProperties {
            foreground: Some(Color::from_rgba32(color << 8 | 0xff)),
            bold: false,
            italic: false,
            text_decoration: None,
        },
    )
}

const STRING_STYLE: HighlightStyle<'static> =
    HighlightStyle::define(&[rule(&[HighlightSelector::new("string")], CUSTOM_STRING)]);
const NUMBER_STYLE: HighlightStyle<'static> =
    HighlightStyle::define(&[rule(&[HighlightSelector::new("number")], CUSTOM_NUMBER)]);

fn assert_token_color(case: &str, line: &ShapedLine, token: &str, color: Rgba) {
    // GPUI's Debug output exposes the decoration runs consumed by ShapedLine::paint.
    let shaped = format!("{line:?}");
    let (_, runs) = shaped.split_once("decoration_runs:").unwrap();
    let start = line.text.find(token).unwrap();
    let end = start + token.len();
    let expected = format!("color: {:?},", Hsla::from(color));
    let mut offset = 0;
    for run in runs.split("DecorationRun { len: ").skip(1) {
        let (len, style) = run.split_once(", ").unwrap();
        let next = offset + len.parse::<usize>().unwrap();
        if offset < end && start < next {
            assert!(
                style.starts_with(&expected),
                "{case}: {token}: expected {expected}, got {style}"
            );
        }
        offset = next;
    }
    assert_eq!(offset, line.text.len());
}

#[gpui::test]
fn preview_fallback_reconfigures_without_changing_markdown_or_layout(cx: &mut TestAppContext) {
    let config = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(SOURCE.into(), config.of(Extension::default()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(520.), px(360.)));
    let fallback = highlight_layer(HighlightLayer::Fallback(STRING_STYLE));
    let dark_only = highlight_layer(HighlightLayer::Active(
        STRING_STYLE.with_theme(HighlightTheme::Dark),
    ));
    let cases = [
        (
            "App palette",
            Extension::default(),
            LEGACY_STRING,
            LEGACY_NUMBER,
            LIGHT_TEXT,
        ),
        (
            "Active tag miss",
            Extension::group([
                highlight_layer(HighlightLayer::Active(NUMBER_STYLE)),
                fallback.clone(),
            ]),
            LIGHT_TEXT,
            CUSTOM_NUMBER,
            LIGHT_TEXT,
        ),
        (
            "Active scope miss",
            Extension::group([
                highlight_layer(HighlightLayer::Active(
                    STRING_STYLE.with_native_scope(NativeLanguage::Css),
                )),
                fallback.clone(),
            ]),
            LIGHT_TEXT,
            LIGHT_TEXT,
            LIGHT_TEXT,
        ),
        (
            "Active scope match",
            highlight_layer(HighlightLayer::Active(
                STRING_STYLE.with_native_scope(NativeLanguage::Json),
            )),
            CUSTOM_STRING,
            LIGHT_TEXT,
            LIGHT_TEXT,
        ),
        (
            "Empty resolved layer",
            highlight_layer(HighlightLayer::Resolved(HighlightProperties::default())),
            LIGHT_TEXT,
            LIGHT_TEXT,
            LIGHT_TEXT,
        ),
        (
            "Inactive dark layer",
            dark_only.clone(),
            LEGACY_STRING,
            LEGACY_NUMBER,
            LIGHT_TEXT,
        ),
        (
            "Inactive dark layer with native fallback",
            Extension::group([dark_only.clone(), fallback.clone()]),
            CUSTOM_STRING,
            LIGHT_TEXT,
            LIGHT_TEXT,
        ),
        (
            "Active dark layer",
            Extension::group([one_dark_theme(), dark_only, fallback.clone()]),
            CUSTOM_STRING,
            DARK_TEXT,
            DARK_TEXT,
        ),
        (
            "View-only override",
            one_dark_theme(),
            DARK_TEXT,
            DARK_TEXT,
            DARK_TEXT,
        ),
        (
            "First native fallback misses",
            Extension::group([
                highlight_layer(HighlightLayer::Fallback(NUMBER_STYLE)),
                fallback,
            ]),
            LIGHT_TEXT,
            CUSTOM_NUMBER,
            LIGHT_TEXT,
        ),
        (
            "App palette restored",
            Extension::default(),
            LEGACY_STRING,
            LEGACY_NUMBER,
            LIGHT_TEXT,
        ),
    ];
    let mut geometry = None;
    for (case, extensions, string, number, foreground) in cases {
        cx.update(|window, cx| {
            assert_eq!(window.appearance(), WindowAppearance::Light);
            editor.update(cx, |editor, cx| {
                editor
                    .view
                    .update(
                        Transaction::default().with_effects([config.reconfigure(extensions)]),
                        editor.active.clone(),
                        |_| (None, None),
                    )
                    .unwrap();
                cx.notify();
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source().as_bytes(), SOURCE.as_bytes(), "{case}");
            // Inspect the frame retained by the canvas paint callback.
            let layout = editor.layout.as_ref().unwrap();
            assert_eq!(layout.theme.text, rgb(foreground), "{case}");
            let mut blocks = layout.rows.iter().flat_map(|row| {
                row.items.iter().filter_map(move |item| match &item.paint {
                    ItemPaint::Block(block) => Some((row, item, block)),
                    _ => None,
                })
            });
            let (row, item, block) = blocks.next().unwrap();
            assert!(blocks.next().is_none());
            let lines: Vec<_> = block.code_lines().collect();
            assert_eq!(
                lines
                    .iter()
                    .map(|line| line.text.as_ref())
                    .collect::<Vec<_>>(),
                DISPLAY,
                "{case}"
            );
            let current = (
                item.source.clone(),
                row.y,
                row.height,
                item.left,
                item.width,
                block
                    .lines
                    .iter()
                    .map(|(origin, line)| (*origin, line.width))
                    .collect::<Vec<_>>(),
            );
            assert_eq!(
                &current,
                geometry.get_or_insert_with(|| current.clone()),
                "{case}"
            );
            assert_token_color(case, lines[0], "{", rgb(foreground));
            assert_token_color(case, lines[1], STRING, rgb(string));
            assert_token_color(case, lines[2], "7", rgb(number));
        });
    }
}
