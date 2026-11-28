use std::cell::RefCell;

use gpui::{
    Background, Div, Edges, Entity, EntityInputHandler, Hsla, Refineable, Style, TestAppContext,
    VisualTestContext, rgb,
};
use gpui_editor_codemirror::{
    Compartment, Extension, Transaction,
    autocomplete::Completion,
    language::xml::{XmlElementSpec, XmlSchema},
    theme_one_dark::one_dark_theme,
};
use serde::Deserialize;

use super::{tests::activate_draft, *};
use crate::editor::bind_keys;

#[derive(Debug, PartialEq)]
struct Paint {
    text: String,
    background: Background,
    foreground: Hsla,
    border: Edges<Pixels>,
}

thread_local! {
    static PAINT: RefCell<Option<Paint>> = const { RefCell::new(None) };
}

pub(super) fn probe(mut body: Div, text: &str) -> Div {
    let mut style = Style::default();
    style.refine(body.style());
    let text = text.to_owned();
    body.debug_selector(|| "source-completion-info".into())
        .child(
            canvas(
                |_, _, _| (),
                move |_, (), window, _| {
                    PAINT.replace(Some(Paint {
                        text,
                        background: style.background.unwrap().color().unwrap(),
                        foreground: window.text_style().color,
                        border: style.border_widths.to_pixels(window.rem_size()),
                    }));
                },
            )
            .absolute()
            .w_0()
            .h_0(),
        )
}

fn sample(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Option<Paint> {
    let _ = PAINT.take();
    cx.update(|_, cx| editor.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    PAINT.take()
}

#[derive(Deserialize)]
struct BrowserStep {
    name: String,
    selected: Option<String>,
    pane: Option<BrowserPane>,
}

#[derive(Deserialize)]
struct BrowserPane {
    text: String,
    literal: bool,
    arrow: bool,
    #[serde(rename = "describedBy")]
    described_by: bool,
    right: bool,
    background: String,
    foreground: String,
    border: [f32; 4],
}

#[gpui::test]
fn selected_info_tracks_keys_theme_and_composition(cx: &mut TestAppContext) {
    let browser: Vec<BrowserStep> = include_str!("info_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let browser = |name: &str| browser.iter().find(|row| row.name == name).unwrap();
    let info_a = browser("initial").pane.as_ref().unwrap().text.as_str();
    let info_c = browser("next-info").pane.as_ref().unwrap().text.as_str();
    let elements = [
        ("choiceA", Some(info_a)),
        ("choiceB", None),
        ("choiceC", Some(info_c)),
    ]
    .map(|(name, info)| XmlElementSpec {
        name,
        completion: Some(Completion {
            info,
            ..Completion::new("")
        }),
        ..Default::default()
    });
    let schema = NativeXmlSchemaExtension::new(XmlSchema {
        elements: &elements,
        ..Default::default()
    });
    let slot = Compartment::new();
    let draft = "```xml\n<choice\n```\n\nOutside.";
    let head = draft.find("<choice").unwrap() + "<choice".len();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(
            draft.into(),
            Extension::group([schema.extension(), slot.of(Extension::default())]),
            window,
            cx,
        )
        .unwrap()
    });
    cx.simulate_resize(size(px(900.), px(480.)));
    let (clock, baseline, stock_border) = cx.update(|window, cx| {
        bind_keys(cx);
        window.activate_window();
        editor.update(cx, |editor, cx| activate_draft(editor, head, window, cx));
        (
            cx.background_executor().clone(),
            Theme::for_window(window),
            px(if Theme::is_system_dark(window) {
                0.
            } else {
                1.
            }),
        )
    });
    let open = |cx: &mut VisualTestContext| {
        cx.simulate_keystrokes("ctrl-space");
        cx.run_until_parked();
        clock.advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        clock.advance_clock(Duration::from_millis(75));
        cx.run_until_parked();
    };
    open(cx);
    let stock_menu = cx.debug_bounds("source-completion-menu").unwrap();
    let scroll = cx.update(|_, cx| editor.read(cx).scroll.offset());

    let check = |cx: &mut VisualTestContext, name, themed, menu: Bounds<Pixels>| {
        let step = browser(name);
        let painted = sample(&editor, cx);
        assert_eq!(painted.is_some(), step.pane.is_some(), "{name}");
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            let current = editor.current_source_completion().and_then(|current| {
                current
                    .session
                    .selected_completion()
                    .unwrap()
                    .map(|choice| choice.item.label.as_str())
            });
            assert_eq!(current, step.selected.as_deref(), "{name}");
            assert!(editor.focus.is_focused(window));
            assert_eq!(editor.selection(), head..head);
            assert_eq!(editor.source(), draft);
            assert_eq!(editor.view.undo_depth(), 0);
            assert_eq!(editor.scroll.offset(), scroll);
        });
        if step.selected.is_some() {
            assert_eq!(
                cx.debug_bounds("source-completion-menu"),
                Some(menu),
                "{name}"
            );
        }
        if let (Some(paint), Some(pane)) = (painted, &step.pane) {
            assert!(pane.literal && !pane.arrow && pane.described_by && pane.right);
            assert_eq!(paint.text, pane.text, "{name}");
            if themed {
                let color = |hex: &str| Hsla::from(rgb(u32::from_str_radix(hex, 16).unwrap()));
                assert_eq!(paint.background, Background::from(color(&pane.background)));
                assert_eq!(paint.foreground, color(&pane.foreground));
                assert_eq!(
                    [
                        paint.border.top,
                        paint.border.right,
                        paint.border.bottom,
                        paint.border.left
                    ],
                    pane.border.map(px)
                );
            } else {
                assert_eq!(paint.background, baseline.tooltip.into());
                assert_eq!(paint.foreground, baseline.on_tooltip.into());
                assert_eq!(paint.border, Edges::all(stock_border));
            }
            let info = cx.debug_bounds("source-completion-info").unwrap();
            let label = match step.selected.as_deref() {
                Some("choiceA") => "source-completion: choiceA",
                Some("choiceC") => "source-completion: choiceC",
                _ => panic!("unexpected selected choice with info: {name}"),
            };
            let row = cx.debug_bounds(label).unwrap();
            assert_eq!(info.left(), menu.right());
            assert_eq!(info.top(), row.top());
            assert!(!info.intersects(&menu));
        }
    };
    let configure = |cx: &mut VisualTestContext, extension| {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                let active = editor.active.clone();
                editor
                    .view
                    .update(
                        Transaction::default().with_effects([slot.reconfigure(extension)]),
                        active.clone(),
                        |_| (None, active),
                    )
                    .unwrap();
                cx.notify();
            });
        });
    };
    check(cx, "initial", false, stock_menu);
    configure(cx, one_dark_theme());
    cx.run_until_parked();
    let themed_menu = cx.debug_bounds("source-completion-menu").unwrap();
    check(cx, "themed", true, themed_menu);
    cx.simulate_keystrokes("down");
    check(cx, "no-info", true, themed_menu);
    cx.simulate_keystrokes("down");
    check(cx, "next-info", true, themed_menu);
    configure(cx, Extension::default());
    check(cx, "theme-removed", false, stock_menu);

    // A narrow viewport cannot fit the pane next to the list.
    cx.simulate_resize(size(px(290.), px(480.)));
    assert_eq!(sample(&editor, cx).unwrap().text, info_c);
    let narrow_menu = cx.debug_bounds("source-completion-menu").unwrap();
    let narrow_info = cx.debug_bounds("source-completion-info").unwrap();
    assert!(narrow_info.top() >= narrow_menu.bottom() || narrow_info.bottom() <= narrow_menu.top());
    cx.simulate_resize(size(px(900.), px(480.)));
    cx.run_until_parked();

    cx.simulate_keystrokes("escape");
    check(cx, "closed", false, stock_menu);

    open(cx);
    assert!(sample(&editor, cx).is_some());
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            editor.replace_and_mark_text_in_range(None, "字", None, window, cx);
            assert!(editor.is_composing());
        });
    });
    assert!(sample(&editor, cx).is_none());
    cx.update(|_, cx| assert!(editor.read(cx).current_source_completion().is_none()));
}

#[test]
fn completion_info_has_a_byte_limit_and_rejects_blank_or_control_text() {
    let exact = "😀".repeat(MAX_INFO_BYTES / 4);
    assert!(valid_completion_info(&exact));
    assert!(!valid_completion_info(&(exact + "x")));
    for invalid in ["", "\n\t ", "a\u{0}b", "a\u{1b}b"] {
        assert!(!valid_completion_info(invalid));
    }
    assert!(valid_completion_info(" <b>字</b>\n\t**literal** "));
}
