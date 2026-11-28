use gpui::{black, blue, font, red};
use gpui_editor_codemirror::{
    EditorState, Precedence, Text,
    language::{BidiIsolateOptions, wast::WastDocument},
    view::{
        Viewport,
        bidi::{BIDI_ISOLATED_RANGES, Direction, line_isolates},
    },
};
use objc2_app_kit::NSGraphicsContext;

use super::*;

// Line 1 of `mixed-breaks` of the codemirror-rs fixture `tests/fixtures/wast_bidi.jsonl`.
const SOURCE: &str = "(module (data \"ABC אב 🦀\") ;; عرب";

fn pinned_isolates() -> Vec<BidiIsolate> {
    let doc = WastDocument::parse(Text::new(SOURCE));
    let marks = doc
        .bidi_isolate_marks(
            &Viewport::complete(doc.text()),
            BidiIsolateOptions::default(),
        )
        .unwrap();
    let state = EditorState::create_from_text(
        doc.syntax_lines().clone(),
        None,
        BIDI_ISOLATED_RANGES
            .of(marks)
            .with_precedence(Precedence::Lowest),
    )
    .unwrap();
    let isolates = line_isolates(&state, 1).unwrap();
    assert_eq!(
        isolates
            .iter()
            .map(|isolate| isolate.from..isolate.to)
            .collect::<Vec<_>>(),
        [14..29, 31..40]
    );
    isolates
}

fn options(width: f32) -> TextKitOptions {
    TextKitOptions {
        width: px(width),
        margins: Edges::default(),
        font_size: px(16.),
        line_height: px(26.),
        scale_factor: 1.,
    }
}

fn run(len: usize) -> TextRun {
    TextRun {
        len,
        font: font(".SystemUIFont"),
        color: black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    }
}

fn xs(row: &TextKitRow, byte: usize) -> Vec<Pixels> {
    row.carets
        .iter()
        .filter_map(|caret| (caret.byte == byte).then_some(caret.x))
        .collect()
}

#[test]
fn pinned_mixed_wast_has_native_dual_carets_hits_and_disjoint_selection() {
    let isolates = pinned_isolates();
    let layout =
        TextKitLayout::shape(SOURCE, &isolates, &[run(SOURCE.len())], &options(320.)).unwrap();
    assert_eq!(layout.source(), SOURCE);
    assert_eq!(layout.display_units_at(14), Some(14..=15));
    assert_eq!(layout.display_units_at(29), Some(26..=27));
    assert_eq!(layout.display_units_at(31), Some(29..=30));
    assert_eq!(layout.display_units_at(40), Some(36..=37));
    assert_eq!(layout.source_byte_at(23), Some(24));
    assert_eq!(layout.source_byte_at(24), None);
    assert_eq!(layout.source_byte_at(25), Some(28));
    assert_eq!(layout.display_units_at(25), None);
    assert_eq!(layout.rows().len(), 1);
    let row = &layout.rows()[0];
    assert_eq!(row.bytes, 0..40);
    assert_eq!(
        row.logical_stops,
        SOURCE
            .char_indices()
            .map(|(b, _)| b)
            .chain([40])
            .collect::<Vec<_>>()
    );
    let edge = xs(row, 19);
    assert_eq!(edge.len(), 2);
    assert_eq!(edge, xs(row, 23));
    assert_eq!(xs(row, 31).len(), 2);
    assert!(
        row.carets
            .iter()
            .any(|caret| caret.byte == 19 && caret.alternate && caret.x == edge[1])
    );

    let rects = layout.selection_rects(18..21).unwrap();
    assert_eq!(rects.len(), 2);
    assert_eq!(rects[0].origin.x, xs(row, 18)[0]);
    assert_eq!(rects[0].right(), edge[0]);
    assert_eq!(rects[1].origin.x, xs(row, 21)[0]);
    assert_eq!(rects[1].right(), edge[1]);
    assert!(rects[0].right() < rects[1].origin.x);
    // Bytes 19 and 23 share each bidi edge. Test both sides of both edges.
    for (edge, offset, byte) in [
        (edge[0], -1., 19),
        (edge[0], 1., 23),
        (edge[1], -1., 19),
        (edge[1], 1., 23),
    ] {
        let (row_index, hit) = layout.hit(point(edge + px(offset), px(13.))).unwrap();
        assert_eq!((row_index, hit.byte, hit.x), (0, byte, edge));
    }
    let crab = layout.selection_rects(24..28).unwrap();
    assert_eq!(crab[0].origin.x, xs(row, 24)[0]);
    assert_eq!(crab[0].right(), xs(row, 28)[0]);
    assert_eq!(
        layout
            .hit(point(crab[0].right() - px(1.), px(13.)))
            .unwrap()
            .1
            .byte,
        28
    );
    assert!(layout.selection_rects(24..25).is_none());
    assert!(layout.selection_rects(0..usize::MAX).is_none());

    // The isolate moves the neutral comment punctuation to its RTL edge.
    let plain = TextKitLayout::shape(SOURCE, &[], &[run(SOURCE.len())], &options(320.)).unwrap();
    let comment_parts = |layout: &TextKitLayout| {
        (
            layout.selection_rects(31..33).unwrap()[0],
            layout.selection_rects(34..40).unwrap()[0],
        )
    };
    let (semicolons, arabic) = comment_parts(&layout);
    assert!(semicolons.origin.x > arabic.right());
    let (semicolons, arabic) = comment_parts(&plain);
    assert!(semicolons.right() < arabic.origin.x);
    assert_ne!(layout.image.as_bytes(0), plain.image.as_bytes(0));
}

#[test]
fn offscreen_pixels_follow_source_runs_and_restore_graphics_context() {
    let isolates = pinned_isolates();
    let mut prefix = run(14);
    prefix.color = blue();
    let mut comment = run(40 - 31);
    comment.color = red();
    let before = NSGraphicsContext::currentContext();
    let layout = TextKitLayout::shape(
        SOURCE,
        &isolates,
        &[prefix, run(31 - 14), comment],
        &options(320.),
    )
    .unwrap();
    assert_eq!(NSGraphicsContext::currentContext(), before);
    let line = &layout.rows()[0];
    let comment_rect = layout.selection_rects(31..40).unwrap();
    assert_eq!(comment_rect.len(), 1);
    let comment = comment_rect[0];
    assert_eq!(comment.origin.x, xs(line, 31)[0]);
    assert_eq!(comment.right(), xs(line, 31)[1]);
    let crab = layout.selection_rects(24..28).unwrap()[0];
    let mut blue_ink = 0;
    let mut red_ink = 0;
    let mut black_ink = 0;
    let mut partial_red = 0;
    for (at, [b, g, r, a]) in pixels(&layout, 1.) {
        if a < 25 {
            continue;
        }
        if b > 220 && r < 30 && g < 30 {
            assert!(at.x < xs(line, 14)[0] + px(1.));
            blue_ink += 1;
        }
        if r > 220 && g < 30 && b < 30 {
            // The source emoji contains red-orange pixels.
            if crab.origin.x <= at.x && at.x <= crab.right() {
                continue;
            }
            assert!(comment.origin.x - px(1.) <= at.x && at.x <= comment.right() + px(1.));
            red_ink += 1;
            partial_red += usize::from(a < 160);
        }
        if a > 100 && r < 30 && g < 30 && b < 30 {
            assert!(at.x >= xs(line, 14)[0] - px(1.) && at.x < comment.origin.x + px(1.));
            black_ink += 1;
        }
    }
    assert!(blue_ink > 30 && red_ink > 20 && black_ink > 20 && partial_red > 0);
    let monochrome =
        TextKitLayout::shape(SOURCE, &isolates, &[run(SOURCE.len())], &options(320.)).unwrap();
    assert_eq!(line.carets, monochrome.rows()[0].carets);
    assert_eq!(layout.image_bounds, monochrome.image_bounds);
    assert_eq!(
        layout
            .image
            .as_bytes(0)
            .unwrap()
            .iter()
            .skip(3)
            .step_by(4)
            .collect::<Vec<_>>(),
        monochrome
            .image
            .as_bytes(0)
            .unwrap()
            .iter()
            .skip(3)
            .step_by(4)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn crlf_wraps_keep_shared_stops_separate_and_use_scaled_margins() {
    let source = format!("{SOURCE}\r\n\r\nx\r\n");
    let opts = TextKitOptions {
        margins: Edges {
            top: px(3.),
            right: px(11.),
            bottom: px(5.),
            left: px(7.),
        },
        scale_factor: 2.,
        ..options(83.)
    };
    let layout =
        TextKitLayout::shape(&source, &pinned_isolates(), &[run(source.len())], &opts).unwrap();
    assert_eq!(layout.source().as_bytes(), source.as_bytes());
    let rows = layout.rows();
    assert_eq!(
        rows.iter().map(|row| row.bytes.clone()).collect::<Vec<_>>(),
        [0..8, 8..14, 14..24, 24..34, 34..42, 42..44, 44..47, 47..47]
    );
    assert_eq!(layout.size, size(px(83.), px(216.)));
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(
            row.bounds,
            Bounds::new(
                point(px(7.), px(3. + 26. * i as f32)),
                size(px(65.), px(26.))
            )
        );
        assert!(
            row.carets
                .iter()
                .all(|caret| ![41, 43, 46].contains(&caret.byte))
        );
    }
    assert_eq!(xs(&rows[0], 8).len(), 1);
    assert_eq!(xs(&rows[1], 8), [px(7.)]);
    for (i, byte, x) in [
        (0, 8, px(75.)),
        (1, 8, px(7.)),
        (5, 42, px(30.)),
        (7, 47, px(30.)),
    ] {
        let (row, caret) = layout
            .hit(point(x, rows[i].bounds.origin.y + px(13.)))
            .unwrap();
        assert_eq!((row, caret.byte), (i, byte));
    }
    assert!(layout.hit(point(px(7.), px(2.))).is_none());
    assert!(layout.selection_rects(40..41).is_none());
    let break_rects = layout.selection_rects(40..44).unwrap();
    assert!(break_rects.iter().any(|r| r.origin.y <= rows[5].bounds.origin.y && r.bottom() >= rows[5].bounds.bottom()));
    let wrapped = layout.selection_rects(22..28);
    assert!(wrapped.is_none()); // UTF-8 interior of Bet.
    let wrapped = layout.selection_rects(23..28).unwrap();
    assert!(
        wrapped
            .iter()
            .any(|r| r.origin.y == rows[2].bounds.origin.y)
    );
    assert!(
        wrapped
            .iter()
            .any(|r| r.origin.y == rows[3].bounds.origin.y)
    );

    let mut has_ink = [false; 8];
    for (at, [_, _, _, alpha]) in pixels(&layout, opts.scale_factor) {
        if alpha > 80 {
            let row = rows
                .iter()
                .position(|row| row.bounds.origin.y <= at.y && at.y < row.bounds.bottom())
                .unwrap();
            has_ink[row] = true;
        }
    }
    assert_eq!(has_ink, [true, true, true, true, true, false, true, false]);
}

#[test]
fn lam_alef_has_native_insertion_inside_the_ligature() {
    let source = "x ;; لا";
    let isolate = BidiIsolate {
        from: 2,
        to: source.len(),
        direction: None,
        inner: Vec::new(),
    };
    let isolates = [isolate];
    let layout =
        TextKitLayout::shape(source, &isolates, &[run(source.len())], &options(150.)).unwrap();
    let row = &layout.rows()[0];
    let interior = xs(row, 7)[0];
    assert!(xs(row, 9)[0] < interior && interior < xs(row, 5)[0]);
    assert_eq!(layout.hit(point(interior, px(13.))).unwrap().1.byte, 7);
    // A paint run may not silently recolor both letters of the ligature.
    let mut alef = run(2);
    alef.color = red();
    assert!(matches!(
        TextKitLayout::shape(source, &isolates, &[run(7), alef], &options(150.)),
        Err(TextKitError::UnsupportedStyle)
    ));
}

#[test]
fn nested_projection_preserves_bytes_and_validates_source_ranges_and_styles() {
    let isolate = |range: Range<usize>, direction| BidiIsolate {
        from: range.start,
        to: range.end,
        direction,
        inner: Vec::new(),
    };
    let mut outer = isolate(0..6, Some(Direction::Ltr));
    outer.inner.push(isolate(1..5, Some(Direction::Rtl)));
    let nested = [outer, isolate(6..7, None)];
    let source = "A🦀BC";
    let projection = Projection::new(source, &nested).unwrap();
    assert_eq!(
        projection.text,
        "\u{2066}A\u{2067}🦀\u{2069}B\u{2069}\u{2068}C\u{2069}"
    );
    assert_eq!(projection.units_at(6), Some(7..=9));
    assert_eq!(projection.byte_at(4), None);
    assert_eq!(projection.source(), source);
    assert!(matches!(
        Projection::new(source, &[isolate(2..5, None)]),
        Err(TextKitError::InvalidIsolate)
    ));
    assert!(matches!(
        Projection::new("a\r\nb", &[isolate(0..4, None)]),
        Err(TextKitError::InvalidIsolate)
    ));
    assert!(matches!(
        Projection::new("a\u{2069}b", &[isolate(0..5, None)]),
        Err(TextKitError::UnsupportedSource)
    ));
    let shape =
        |source: &str, runs: &[TextRun]| TextKitLayout::shape(source, &[], runs, &options(320.));
    assert!(matches!(
        shape(source, &[run(2), run(5)]),
        Err(TextKitError::InvalidRuns)
    ));
    let mut decorated = run(source.len());
    decorated.background_color = Some(red());
    assert!(matches!(
        shape(source, &[decorated]),
        Err(TextKitError::UnsupportedStyle)
    ));
    let mut menlo = run(source.len());
    menlo.font = font("Menlo");
    let menlo = shape(source, &[menlo]).unwrap();
    let system = shape(source, &[run(source.len())]).unwrap();
    assert_ne!(menlo.rows()[0].carets, system.rows()[0].carets);
    assert!(matches!(
        shape(&"a".repeat(32 * 1024 + 1), &[run(32 * 1024 + 1)]),
        Err(TextKitError::Limit)
    ));
}

fn pixels(
    layout: &TextKitLayout,
    scale: f32,
) -> impl Iterator<Item = (Point<Pixels>, [u8; 4])> + '_ {
    let width = (f32::from(layout.image_bounds.size.width) * scale) as usize;
    layout
        .image
        .as_bytes(0)
        .unwrap()
        .chunks_exact(4)
        .enumerate()
        .map(move |(i, p)| {
            let origin = layout.image_bounds.origin;
            let at = point(
                origin.x + px(((i % width) as f32 + 0.5) / scale),
                origin.y + px(((i / width) as f32 + 0.5) / scale),
            );
            (at, p.try_into().unwrap())
        })
}
