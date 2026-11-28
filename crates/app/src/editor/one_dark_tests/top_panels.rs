use gpui::{BorderStyle, Edges, Modifiers, VisualTestContext};
use serde::Deserialize;

use super::{
    panel_menu::{Appearance, Role, Sample, sample},
    *,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserFrame {
    mode: String,
    root: String,
    group: Chrome,
    panels: [Chrome; 2],
    edge: f32,
    find_button: String,
    find_field: String,
    go_button: String,
    go_field: String,
}

#[derive(Deserialize)]
struct Chrome {
    fill: Option<String>,
    text: String,
    border: Option<String>,
    bottom: f32,
}

fn color(hex: &str) -> Rgba {
    rgb(u32::from_str_radix(hex, 16).unwrap())
}

impl Chrome {
    fn appearance(&self) -> Appearance {
        Appearance {
            fill: self.fill.as_deref().map(|hex| color(hex).into()),
            text: color(&self.text).into(),
            border: self
                .border
                .as_deref()
                .map(|hex| (color(hex).into(), BorderStyle::Solid)),
            widths: Edges {
                bottom: px(self.bottom),
                ..Edges::default()
            },
        }
    }
}

fn appearance(samples: &[Sample], role: Role) -> &Appearance {
    let mut matches = samples.iter().filter(|sample| sample.role == role);
    let mounted = matches.next().unwrap_or_else(|| panic!("missing {role:?}"));
    assert!(matches.next().is_none(), "duplicate {role:?}");
    &mounted.appearance
}

fn bounds(cx: &mut VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("missing {selector}"))
}

#[gpui::test]
fn one_dark_composes_live_top_panels_at_the_body_edge(cx: &mut TestAppContext) {
    let source = "```json\r\n{\"key\": \"字\"}\r\n```\r\n\r\nOutside.";
    let slot = Compartment::new();
    let (editor, cx) = cx.add_window_view(|window, cx| {
        Editor::with_native_extensions(source.into(), slot.of(Extension::default()), window, cx)
            .unwrap()
    });
    cx.simulate_resize(size(px(740.), px(480.)));
    let selection = cx.update(|window, cx| {
        assert_eq!(window.appearance(), WindowAppearance::Light);
        bind_keys(cx);
        window.activate_window();
        editor.update(cx, |editor, cx| {
            let at = activate_code(editor, "key", cx);
            editor.set_selection(at, at);
            window.focus(&editor.focus);
            editor.view.state().selection().clone()
        })
    });
    cx.run_until_parked();
    let full = cx.update(|_, cx| editor.read(cx).scroll.bounds());
    cx.simulate_keystrokes("cmd-shift-m");
    cx.update(|window, cx| {
        editor.update(cx, |editor, cx| editor.open_find(window, cx));
    });
    cx.run_until_parked();
    let replace = bounds(cx, "document-find-show-replacement");
    cx.simulate_click(replace.center(), Modifiers::none());
    cx.simulate_keystrokes("cmd-alt-g");
    cx.run_until_parked();
    let focus = cx.update(|window, cx| window.focused(cx).unwrap());
    let native_body = cx.update(|_, cx| editor.read(cx).scroll.bounds());

    let panels = [
        (Role::Find, "document-find-panel"),
        (Role::GotoLine, "goto-line-panel"),
        (Role::Diagnostics, "source-diagnostics-panel"),
    ];
    let controls = [
        (0, "document-find-input"),
        (0, "document-find-next"),
        (0, "document-find-close"),
        (0, "document-replacement-input"),
        (0, "document-find-replace-all"),
        (1, "goto-line-value"),
        (1, "goto-line-close"),
        (2, "source-diagnostics-close"),
    ];
    let native = panels.each_ref().map(|(_, selector)| bounds(cx, selector));
    let relative_controls = |cx: &mut VisualTestContext, panels: &[Bounds<Pixels>; 3]| {
        controls.map(|(panel, selector)| {
            let bounds = bounds(cx, selector);
            (bounds.origin - panels[panel].origin, bounds.size)
        })
    };
    let native_controls = relative_controls(cx, &native);
    let host = override_editor_surface(EditorSurfaceColors {
        foreground: Some(Color::from_rgba32(0x3040_50ff)),
        ..Default::default()
    });
    for row in include_str!("top_panels_browser.jsonl").lines() {
        let frame: BrowserFrame = serde_json::from_str(row).unwrap();
        let themed = matches!(frame.mode.as_str(), "theme" | "override");
        let extension = match frame.mode.as_str() {
            "native" | "removed" => Extension::default(),
            "theme" => one_dark_theme(),
            "override" => Extension::group([host.clone(), one_dark_theme()]),
            mode => panic!("unexpected mode {mode}"),
        };
        cx.update(|_, cx| {
            editor.update(cx, |editor, cx| {
                configure(editor, slot.reconfigure(extension), cx)
            });
        });
        let samples = sample(&editor, cx);
        let panel_bounds = panels.each_ref().map(|(_, selector)| bounds(cx, selector));
        let body = cx.update(|window, cx| {
            let editor = editor.read(cx);
            assert!(focus.is_focused(window), "{}", frame.mode);
            assert_eq!(editor.source(), source);
            assert_eq!(editor.view.state().selection(), &selection);
            assert_eq!(editor.view.undo_depth(), 0);
            let expected = if themed {
                color(&frame.root)
            } else {
                rgb(0x172336)
            };
            assert_eq!(editor.layout.as_ref().unwrap().theme.text, expected);
            editor.scroll.bounds()
        });
        assert_eq!(panel_bounds[0].top(), full.top());
        assert_eq!(body.bottom(), full.bottom());
        assert_eq!(relative_controls(cx, &panel_bounds), native_controls);
        for pair in panel_bounds.windows(2) {
            assert_eq!(pair[0].bottom(), pair[1].top());
        }

        let panel_text = if themed {
            color(&frame.group.text)
        } else {
            rgb(0x172336)
        };
        for (i, ((role, _), current)) in panels.iter().zip(panel_bounds).enumerate() {
            let expected = if themed {
                // The browser has two children, and both inherit the same roles.
                frame.panels[usize::from(i == 1)].appearance()
            } else {
                Appearance {
                    fill: Some(rgb(if i == 1 { 0xf8faff } else { 0xf0f3f9 }).into()),
                    text: panel_text.into(),
                    border: Some((rgb(0xd3dce9).into(), BorderStyle::Solid)),
                    widths: Edges {
                        bottom: px(1.),
                        ..Edges::default()
                    },
                }
            };
            assert_eq!(
                appearance(&samples, role.clone()),
                &expected,
                "{}: {role:?}",
                frame.mode
            );
            assert_eq!(current.size.width, native[i].size.width);
            assert_eq!(
                current.size.height + if themed { px(1.) } else { px(0.) },
                native[i].size.height
            );
        }
        let find_text = if themed {
            color(&frame.find_button)
        } else {
            panel_text
        };
        for sample in &samples {
            if matches!(sample.role, Role::FindControl(_)) {
                assert_eq!(
                    sample.appearance.text,
                    find_text.into(),
                    "{}: {:?}",
                    frame.mode,
                    sample.role
                );
            }
        }
        assert_eq!(
            appearance(&samples, Role::FindControl("document-find-next")).text,
            find_text.into()
        );
        // The native Close controls have no browser .cm-button counterpart.
        for role in [Role::GotoClose, Role::DiagnosticsClose] {
            assert_eq!(appearance(&samples, role).text, panel_text.into());
        }
        if themed {
            for hex in [&frame.find_field, &frame.go_button, &frame.go_field] {
                assert_eq!(color(hex), panel_text);
            }
            assert_eq!(
                appearance(&samples, Role::TopPanels),
                &frame.group.appearance()
            );
            let group = bounds(cx, "editor-top-panels");
            assert_eq!(group.top(), full.top());
            assert_eq!(group.bottom(), body.top());
            assert_eq!(body.top() - panel_bounds[2].bottom(), px(frame.edge));
            assert_eq!(body.top(), native_body.top() - px(3. - frame.edge));
        } else {
            assert!(!samples.iter().any(|sample| sample.role == Role::TopPanels));
            assert_eq!(panel_bounds, native);
            assert_eq!(body, native_body);
            assert_eq!(body.top(), panel_bounds[2].bottom());
        }
    }
}
