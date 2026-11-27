#[path = "../../../tests/e2e/fixture.rs"]
mod fixture;

use gpui::{Pixels, Point, size};

use super::*;
use crate::{editor::TestPaint, rendering::render_math};

const SIZES: [&str; 2] = ["small", "long"];
const CONTROL: &str = "Control paragraph.";
const CODE_ENTRY: &str = "Code entry\n\n";
const CODE_EXIT: &str = "Leave code here.";
const MATH_EXIT: &str = "Leave formula here.";
const INITIAL_FORMULA: &str = "x^2";
const UPDATED_FORMULA: &str = r"\sum_{i=1}^{12} i^2 + x^2";
const INVALID_FORMULA: &str = r"\frac{1}{";
const REPAIRED_FORMULA: &str = r"\frac{1}{2}";

fn document(size: &str, variant: &str) -> String {
    fixture::document(size, variant).expect("valid block editing fixture")
}

fn open_fixture<'a>(
    app: &'a mut TestAppContext,
    fixture: &Fixture,
) -> (Entity<Shell>, &'a mut VisualTestContext) {
    let (shell, cx) = app.add_window_view(Shell::new);
    cx.simulate_resize(size(px(1040.), px(720.)));
    cx.update(|_, cx| {
        editor::bind_keys(cx);
        shell.read(cx).hotkeys.bindings.clone().bind(cx);
    });
    fixture.open(cx, &shell);
    click_tool(cx, "workspace-file-0");
    (shell, cx)
}

fn current(cx: &mut VisualTestContext, shell: &Entity<Shell>) -> Entity<Editor> {
    cx.update(|_, cx| shell.read(cx).current().unwrap().editor.clone())
}

fn click_caret(cx: &mut VisualTestContext, editor: &Entity<Editor>, offset: usize) {
    let location = cx.update(|window, cx| {
        editor.update(cx, |editor, cx| {
            let offset = editor.source()[..offset].encode_utf16().count();
            editor
                .bounds_for_range(offset..offset, Bounds::default(), window, cx)
                .unwrap()
                .center()
        })
    });
    cx.simulate_click(location, Modifiers::none());
}

fn assert_source_frame(cx: &mut VisualTestContext, editor: &Entity<Editor>, visible: bool) {
    cx.update(|_, cx| assert_eq!(editor.read(cx).test_source_frame_is_visible(), visible));
}

fn reopen(cx: &mut VisualTestContext, shell: &Entity<Shell>, expected: &str) -> Entity<Editor> {
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    cx.update(|_, cx| assert!(shell.read(cx).current().is_none()));
    click_tool(cx, "workspace-file-0");
    let editor = current(cx, shell);
    cx.update(|_, cx| assert_eq!(editor.read(cx).source(), expected));
    editor
}

#[gpui::test]
fn block_editing_control_paragraph_accepts_continuous_input(app: &mut TestAppContext) {
    for size in SIZES {
        let fixture = Fixture::new();
        let original = document(size, "start");
        let path = fixture.document("note.md", &original);
        let (shell, cx) = open_fixture(app, &fixture);
        let editor = current(cx, &shell);
        let end = original.find(CONTROL).unwrap() + CONTROL.len();
        click_caret(cx, &editor, end);
        cx.simulate_input(" 12345678901234567890x");
        cx.simulate_keystrokes("backspace");
        let mut expected = original.clone();
        expected.insert_str(end, " 12345678901234567890");
        cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), expected);
            assert!(matches!(
                editor.test_painted_item(end - CONTROL.len()).1,
                TestPaint::Text("Control paragraph. 12345678901234567890")
            ));
        });
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
    }
}

fn assert_code_preview(cx: &mut VisualTestContext, editor: &Entity<Editor>, block: usize) {
    cx.update(|_, cx| {
        let editor = editor.read(cx);
        assert!(!editor.test_source_frame_is_visible());
        assert!(matches!(
            editor.test_painted_item(block).1,
            TestPaint::Block
        ));
    });
}

#[gpui::test]
fn block_editing_saves_a_code_draft_and_reopens_the_preview(app: &mut TestAppContext) {
    for size in SIZES {
        let fixture = Fixture::new();
        let original = document(size, "start");
        let expected = document(size, "code");
        let path = fixture.document("note.md", &original);
        let (shell, cx) = open_fixture(app, &fixture);
        let editor = current(cx, &shell);
        let entry = original.find(CODE_ENTRY).unwrap() + CODE_ENTRY.len();
        click_caret(cx, &editor, entry - 2);
        cx.simulate_keystrokes("right right");
        assert_eq!(selection(cx, &editor), entry..entry);
        cx.simulate_input("```");
        assert_eq!(
            selection(cx, &editor),
            entry + 4..entry + 4,
            "{size} fixture"
        );
        assert_source_frame(cx, &editor, true);

        cx.simulate_input("let result = 123456789;");
        cx.simulate_keystrokes("enter");
        cx.simulate_input("    return result + 2;x");
        cx.simulate_keystrokes("backspace");
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
        cx.simulate_keystrokes("secondary-s");
        cx.run_until_parked();
        assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
        assert_source_frame(cx, &editor, true);

        click_caret(cx, &editor, expected.find(CODE_EXIT).unwrap());
        assert_code_preview(cx, &editor, entry);
        let reopened = reopen(cx, &shell, &expected);
        assert_code_preview(cx, &reopened, entry);
    }
}

fn assert_formula_preview(
    cx: &mut VisualTestContext,
    editor: &Entity<Editor>,
    offset: usize,
    body: &str,
) -> Point<Pixels> {
    cx.update(|window, cx| {
        let editor = editor.read(cx);
        assert!(!editor.test_source_frame_is_visible());
        let (center, paint) = editor.test_painted_item(offset);
        let TestPaint::Graphic(image) = paint else {
            panic!("expected a rendered display formula");
        };
        let raster = render_math(&format!("\n{body}\n"), true, window.scale_factor()).unwrap();
        assert_eq!(image.as_bytes(0).unwrap(), raster.bgra);
        center
    })
}

fn change_formula(cx: &mut VisualTestContext, point: Point<Pixels>, body: &str) {
    cx.simulate_click(point, Modifiers::none());
    cx.simulate_keystrokes("secondary-a enter");
    cx.simulate_input(body);
    cx.simulate_keystrokes("enter");
}

#[gpui::test]
fn block_editing_repairs_a_formula_and_reopens_the_updated_preview(app: &mut TestAppContext) {
    for size in SIZES {
        let fixture = Fixture::new();
        let original = document(size, "start");
        let expected = document(size, "latex");
        let path = fixture.document("note.md", &original);
        let (shell, cx) = open_fixture(app, &fixture);
        let editor = current(cx, &shell);
        let offset = original.find("$$\n").unwrap();
        let preview = assert_formula_preview(cx, &editor, offset, INITIAL_FORMULA);

        change_formula(cx, preview, UPDATED_FORMULA);
        assert_source_frame(cx, &editor, true);
        let updated = original.replacen(INITIAL_FORMULA, UPDATED_FORMULA, 1);
        click_caret(cx, &editor, updated.find(MATH_EXIT).unwrap());
        let preview = assert_formula_preview(cx, &editor, offset, UPDATED_FORMULA);

        change_formula(cx, preview, INVALID_FORMULA);
        let invalid = original.replacen(INITIAL_FORMULA, INVALID_FORMULA, 1);
        click_caret(cx, &editor, invalid.find(MATH_EXIT).unwrap());
        let error = cx.update(|_, cx| {
            let editor = editor.read(cx);
            assert_eq!(editor.source(), invalid);
            let (center, paint) = editor.test_painted_item(offset);
            assert!(matches!(
                paint,
                TestPaint::Text("Invalid formula — click to edit")
            ));
            center
        });

        change_formula(cx, error, REPAIRED_FORMULA);
        assert_source_frame(cx, &editor, true);
        click_caret(cx, &editor, expected.find(MATH_EXIT).unwrap());
        assert_formula_preview(cx, &editor, offset, REPAIRED_FORMULA);
        assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
        cx.simulate_keystrokes("secondary-s");
        cx.run_until_parked();
        assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
        let reopened = reopen(cx, &shell, &expected);
        assert_formula_preview(cx, &reopened, offset, REPAIRED_FORMULA);
    }
}
