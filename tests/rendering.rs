use gpui_editor::rendering::{Raster, render_math, render_mermaid};

fn assert_visible(raster: &Raster) {
    assert!(raster.width > 0 && raster.height > 0);
    assert_eq!(
        raster.bgra.len(),
        raster.width as usize * raster.height as usize * 4
    );
    assert!(raster.bgra.chunks_exact(4).any(|pixel| pixel[3] != 0));
}

#[test]
fn math_reports_errors_and_renders_after_a_repair() {
    assert!(render_math(r"\frac{1}{", false, 1.0).is_err());
    let repaired = render_math(r"\frac{1}{2}", false, 1.0).unwrap();
    assert_visible(&repaired);
}

#[test]
fn math_uses_display_style_and_scale() {
    let formula = r"\sum_{i=1}^n i^2";
    let inline = render_math(formula, false, 1.0).unwrap();
    let display = render_math(formula, true, 1.0).unwrap();
    let doubled = render_math(formula, true, 2.0).unwrap();
    for raster in [&inline, &display, &doubled] {
        assert_visible(raster);
    }
    assert!(display.height > inline.height);
    assert!((display.width * 2).abs_diff(doubled.width) <= 1);
    assert!((display.height * 2).abs_diff(doubled.height) <= 1);
}

#[test]
fn flowchart_reports_errors_and_renders_cjk_after_a_repair() {
    assert!(render_mermaid("flowchart LR\nA[開始] -->", 1.0).is_err());
    let repaired = "flowchart LR\nA[開始] -->|완료| B[结束]";
    let raster = render_mermaid(repaired, 1.0).unwrap();
    let doubled = render_mermaid(repaired, 2.0).unwrap();
    assert_visible(&raster);
    assert_visible(&doubled);
    assert!((raster.width * 2).abs_diff(doubled.width) <= 1);
    assert!((raster.height * 2).abs_diff(doubled.height) <= 1);
}

#[test]
fn flowchart_click_directives_do_not_change_pixels() {
    let source = "graph TD\nA[Start] --> B[End]";
    let baseline = render_mermaid(source, 1.0).unwrap();
    let with_clicks = render_mermaid(
        &format!("{source}\nclick A callback\nclick B href \"https://example.invalid/image.svg\""),
        1.0,
    )
    .unwrap();
    assert_eq!(
        (baseline.width, baseline.height),
        (with_clicks.width, with_clicks.height)
    );
    assert_eq!(baseline.bgra, with_clicks.bgra);
}

#[test]
fn invalid_scales_and_large_inputs_are_rejected() {
    for scale in [0.0, -1.0, f32::NAN, f32::INFINITY, 9.0] {
        assert!(
            render_math("x", false, scale)
                .unwrap_err()
                .contains("Scale")
        );
        assert!(
            render_mermaid("graph TD\nA", scale)
                .unwrap_err()
                .contains("Scale")
        );
    }
    assert!(
        render_math(&"x".repeat(5000), false, 1.0)
            .unwrap_err()
            .contains("byte limit")
    );
    let large = format!("graph TD\nA[{}]", "x".repeat(9000));
    assert!(
        render_mermaid(&large, 1.0)
            .unwrap_err()
            .contains("byte limit")
    );
    assert!(
        render_math(&"W".repeat(100), false, 8.0)
            .unwrap_err()
            .contains("Raster side")
    );
}

#[test]
fn only_latex_and_flat_flowcharts_are_supported() {
    assert!(render_math("<math><mi>x</mi></math>", false, 1.0).is_err());
    assert!(render_mermaid("sequenceDiagram\nA->>B: Hello", 1.0).is_err());
    assert!(render_mermaid(r#"{"metadata":{"diagram_type":"flowchart"}}"#, 1.0).is_err());
    assert!(render_mermaid("graph TD\nsubgraph Box\nA --> B\nend", 1.0).is_err());
    assert!(render_mermaid("graph TD\n%% No nodes", 1.0).is_err());
}
