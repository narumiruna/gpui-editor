use std::cell::RefCell;
use std::time::Duration;

use gpui::{
    Background, BorderStyle, Edges, Entity, Hsla, ParentElement, Refineable, Style, Styled,
    VisualTestContext,
};
use serde::Deserialize;

use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::editor) enum Role {
    TopPanels,
    Find,
    FindControl(&'static str),
    GotoLine,
    GotoClose,
    Diagnostics,
    DiagnosticsClose,
    References,
    CompletionMenu,
    CompletionOption { label: String, selected: bool },
}

#[derive(Debug, PartialEq)]
pub(super) struct Appearance {
    pub(super) fill: Option<Background>,
    pub(super) text: Hsla,
    pub(super) border: Option<(Hsla, BorderStyle)>,
    pub(super) widths: Edges<Pixels>,
}

#[derive(Debug)]
pub(in crate::editor) struct Sample {
    pub(super) role: Role,
    pub(super) appearance: Appearance,
}

thread_local! {
    static SAMPLES: RefCell<Option<Vec<Sample>>> = const { RefCell::new(None) };
}

pub(in crate::editor) fn probe<E: Styled + ParentElement>(mut element: E, role: Role) -> E {
    if !SAMPLES.with_borrow(Option::is_some) {
        return element;
    }
    // These elements have no hover/focus style refinements of their own. Resolve their
    // base style with GPUI defaults; read inherited text in the child paint callback.
    // Neither value is a readback of submitted scene quads or glyphs.
    let mut style = Style::default();
    style.refine(element.style());
    element.child(
        canvas(
            |_, _, _| (),
            move |_, (), window, _| {
                let widths = style.border_widths.to_pixels(window.rem_size());
                let appearance = Appearance {
                    fill: style
                        .background
                        .and_then(|fill| fill.color())
                        .filter(|fill| !fill.is_transparent()),
                    text: window.text_style().color,
                    border: style
                        .border_color
                        .filter(|_| widths != Edges::default())
                        .map(|color| (color, style.border_style)),
                    widths,
                };
                SAMPLES.with_borrow_mut(|samples| {
                    if let Some(samples) = samples {
                        samples.push(Sample { role, appearance });
                    }
                });
            },
        )
        .absolute()
        .w_0()
        .h_0(),
    )
}

pub(in crate::editor) fn sample(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
) -> Vec<Sample> {
    SAMPLES.replace(Some(Vec::new()));
    cx.update(|_, cx| editor.update(cx, |_, cx| cx.notify()));
    cx.run_until_parked();
    SAMPLES.take().unwrap()
}

// Pinned Chromium measurements from the real panels and completion list.
// The top Search and bottom lint wrappers provide shared roles, not native placement.
fn browser(name: &str) -> Appearance {
    #[derive(Deserialize)]
    struct RoleCss {
        name: String,
        fill: Option<String>,
        text: String,
        border: Option<String>,
        top: f32,
        bottom: f32,
    }
    let css = include_str!("../../../tests/fixtures/codemirror/one_dark_panel_menu_browser.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<RoleCss>(line).unwrap())
        .find(|row| row.name == name)
        .unwrap();
    let color = |hex: String| rgb(u32::from_str_radix(&hex, 16).unwrap());
    Appearance {
        fill: css.fill.map(|hex| color(hex).into()),
        text: color(css.text).into(),
        border: css
            .border
            .map(|hex| (color(hex).into(), BorderStyle::Solid)),
        widths: Edges {
            top: px(css.top),
            bottom: px(css.bottom),
            ..Edges::default()
        },
    }
}

pub(in crate::editor) fn assert_panel(samples: &[Sample], role: Role, themed: bool) {
    let [panel] = samples else {
        panic!("expected one panel: {samples:?}");
    };
    let name = match role {
        Role::Diagnostics | Role::TopPanels => "stock-top-search",
        Role::References => "stock-bottom-lint",
        _ => unreachable!(),
    };
    assert_eq!(panel.role, role);
    let expected = if themed {
        browser(name)
    } else {
        Appearance {
            fill: Some(rgb(0xf0f3f9).into()),
            text: rgb(0x172336).into(),
            border: Some((rgb(0xd3dce9).into(), BorderStyle::Solid)),
            widths: Edges {
                top: px(if role == Role::References { 1. } else { 0. }),
                bottom: px(if role == Role::Diagnostics { 1. } else { 0. }),
                ..Edges::default()
            },
        }
    };
    assert_eq!(panel.appearance, expected, "{role:?}");
}

#[gpui::test]
fn one_dark_top_diagnostics_panel_roles_follow_live_reconfiguration(cx: &mut TestAppContext) {
    let source = "```json\r\n{\"key\": \"字\", }\r\n```\r\n\r\nOutside.";
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.into(), slot.of(Extension::default()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(650.), px(400.)));
    let selection = cx.update(|window, cx| {
        bind_keys(cx);
        window.activate_window();
        editor.update(cx, |editor, cx| {
            let at = activate_code(editor, "key", cx);
            editor.set_selection(at + 3, at);
            window.focus(&editor.focus);
            editor.view.state().selection().clone()
        })
    });
    cx.run_until_parked();
    let full = cx.update(|_, cx| editor.read(cx).scroll.bounds());
    cx.simulate_keystrokes("cmd-shift-m");
    cx.run_until_parked();
    for (themed, extension) in [
        (false, None),
        (true, Some(one_dark_theme())),
        (false, Some(Extension::default())),
    ] {
        if let Some(extension) = extension {
            cx.update(|_, cx| {
                editor.update(cx, |editor, cx| {
                    configure(editor, slot.reconfigure(extension), cx)
                })
            });
        }
        let (role, selector) = if themed {
            (Role::TopPanels, "editor-top-panels")
        } else {
            (Role::Diagnostics, "source-diagnostics-panel")
        };
        let mut samples = sample(&editor, cx);
        samples.retain(|sample| sample.role == role);
        assert_panel(&samples, role, themed);
        let panel = cx.debug_bounds(selector).unwrap();
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            let body = editor.scroll.bounds();
            assert_eq!(panel.top(), full.top());
            assert_eq!(panel.bottom(), body.top());
            assert_eq!(body.bottom(), full.bottom());
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.undo_depth(), 0);
        });
    }
    cx.simulate_keystrokes("escape");
    assert!(sample(&editor, cx).is_empty());
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(editor.focus.is_focused(window));
        assert_eq!(editor.scroll.bounds(), full);
        assert_eq!(editor.view.state().selection(), &selection);
    });
}

fn assert_menu(samples: &[Sample], themed: bool) -> &str {
    let mut menu = false;
    let mut selected_label = None;
    let mut unselected = false;
    for Sample { role, appearance } in samples {
        let (name, fill, border, width) = match role {
            Role::CompletionMenu => {
                assert!(!menu);
                menu = true;
                (
                    "completion-menu",
                    Some(rgb(0xf6f8fc)),
                    Some(rgb(0xd3dce9)),
                    1.,
                )
            }
            Role::CompletionOption {
                label,
                selected: true,
            } => {
                assert!(selected_label.replace(label.as_str()).is_none());
                ("completion-selected", Some(rgba(0x4a86ed50)), None, 0.)
            }
            Role::CompletionOption {
                selected: false, ..
            } => {
                unselected = true;
                ("completion-unselected", None, None, 0.)
            }
            _ => panic!("unexpected {role:?}"),
        };
        let expected = if themed {
            browser(name)
        } else {
            Appearance {
                fill: fill.map(Into::into),
                text: rgb(0x172336).into(),
                border: border.map(|color| (color.into(), BorderStyle::Solid)),
                widths: Edges::all(px(width)),
            }
        };
        assert_eq!(*appearance, expected, "{role:?}");
    }
    assert!(menu && unselected, "{samples:?}");
    selected_label.expect("selected completion")
}

#[gpui::test]
fn one_dark_completion_menu_roles_follow_keys_and_live_reconfiguration(cx: &mut TestAppContext) {
    let source = "```css\n.p { col }\n```\n\nOutside.";
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.into(), slot.of(Extension::default()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(650.), px(480.)));
    let (clock, selection) = cx.update(|window, cx| {
        bind_keys(cx);
        window.activate_window();
        let selection = editor.update(cx, |editor, cx| {
            let head = activate_code(editor, "col", cx) + 3;
            editor.set_selection(head, head);
            window.focus(&editor.focus);
            editor.view.state().selection().clone()
        });
        (cx.background_executor().clone(), selection)
    });
    cx.simulate_keystrokes("ctrl-space");
    cx.run_until_parked();
    clock.advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    clock.advance_clock(Duration::from_millis(75));
    cx.run_until_parked();
    let original = assert_menu(&sample(&editor, cx), false).to_owned();
    let menu = cx.debug_bounds("source-completion-menu").unwrap();
    let body = cx.update(|_, cx| editor.read(cx).scroll.bounds());
    assert!(body.contains(&menu.center()));
    let unchanged = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.undo_depth(), 0);
            assert_eq!(editor.scroll.bounds(), body);
            assert!(editor.focus.is_focused(window));
        });
    };
    let reconfigure = |cx: &mut VisualTestContext, extension| {
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, slot.reconfigure(extension), cx)
            })
        });
        let samples = sample(&editor, cx);
        assert_eq!(
            cx.debug_bounds("source-completion-menu").unwrap().origin,
            menu.origin
        );
        unchanged(cx);
        samples
    };
    assert_eq!(
        assert_menu(&reconfigure(cx, one_dark_theme()), true),
        original
    );
    cx.simulate_keystrokes("down");
    let moved = sample(&editor, cx);
    let selected = assert_menu(&moved, true);
    assert_ne!(selected, original);
    assert!(moved.iter().any(|row| matches!(&row.role,
        Role::CompletionOption { label, selected: false } if label == &original)));
    unchanged(cx);
    assert_eq!(
        assert_menu(&reconfigure(cx, Extension::default()), false),
        selected
    );
    cx.simulate_keystrokes("up");
    assert_eq!(assert_menu(&sample(&editor, cx), false), original);
    unchanged(cx);
    cx.simulate_keystrokes("escape");
    assert!(sample(&editor, cx).is_empty());
    unchanged(cx);
}
