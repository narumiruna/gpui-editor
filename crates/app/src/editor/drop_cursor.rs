//! Paint the native drop marker without handling drag or drop events.

use gpui::{Bounds, Pixels, Window, fill, point, px, rgb, rgba, size};
use gpui_editor_codemirror::view::selection_layers::{LayerRect, drop_cursor_marker};

use super::{Editor, Layout, SectionKind};

pub(super) fn paint(
    editor: &Editor,
    layout: &mut Layout,
    revision: u64,
    clip: Bounds<Pixels>,
    window: &mut Window,
) {
    let state = editor.view.state();
    if editor.view.revision() != revision
        || !editor.input_enabled
        || state.read_only()
        || editor.is_composing()
    {
        return;
    }
    let Some(marker) = drop_cursor_marker(state, |offset| {
        let bounds = layout.drop_cursor_bounds(offset)?;
        if !(clip.left()..clip.right()).contains(&bounds.left())
            || clip.size.height <= px(0.)
            || bounds.bottom() <= clip.top()
            || bounds.top() >= clip.bottom()
        {
            return None;
        }
        // A code draft can display its fences, but they are not drop targets.
        if editor.active.as_ref().is_some_and(|section| {
            section.kind == SectionKind::DraftCode
                && layout.source_frame_contains(bounds.center())
                && !editor
                    .source_fence()
                    .is_some_and(|fence| (fence.body.start..=fence.body.end).contains(&offset))
        }) {
            return None;
        }
        if editor.is_markdown() {
            editor
                .markdown()
                .plan_replacement(offset..offset, "", editor.active.as_ref())
                .ok()?;
        }
        Some(LayerRect {
            left: bounds.left().into(),
            top: bounds.top().into(),
            right: bounds.right().into(),
            bottom: bounds.bottom().into(),
        })
    }) else {
        return;
    };

    // The pinned view uses a 1.2 CSS px stroke centered on the measured x.
    let width = px(1.2);
    let bounds = Bounds::new(
        point(
            Pixels::from(marker.left) - width / 2.,
            Pixels::from(marker.top),
        ),
        size(width, Pixels::from(marker.height)),
    );
    let color = layout
        .theme
        .view
        .map_or(rgb(0x000000), |style| rgba(style.drop_cursor.rgba32()));
    let quad = fill(bounds, color);
    #[cfg(test)]
    let submitted = quad.clone();
    window.paint_quad(quad);
    #[cfg(test)]
    {
        layout.painted_drop_cursor = Some(submitted);
    }
}

#[cfg(test)]
mod tests;
