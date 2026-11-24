use std::borrow::Cow;

use gpui::{AssetSource, Context, SharedString, Window, div, prelude::*, px, rgb, svg};

use super::{Editor, commands::Command};
use crate::document::SectionKind;

struct Tool {
    name: &'static str,
    icon: &'static str,
    command: Command,
}

macro_rules! tool {
    ($name:literal, $command:expr, $path:literal) => {
        Tool {
            name: $name,
            icon: concat!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d=""#,
                $path,
                r#""/></svg>"#,
            ),
            command: $command,
        }
    };
}

const FORMAT: &[Tool] = &[
    tool!("Heading", Command::Heading, "M5 5v14M19 5v14M5 12h14"),
    tool!(
        "List",
        Command::Bullet,
        "M9 6h11M9 12h11M9 18h11M4 6h.1M4 12h.1M4 18h.1"
    ),
    tool!(
        "Bold",
        Command::Bold,
        "M7 4h6a4 4 0 0 1 0 8H7m0-8v16h7a4 4 0 0 0 0-8H7"
    ),
    tool!("Italic", Command::Italic, "M11 5h8M5 19h8m2-14L9 19"),
];

const INSERT: &[Tool] = &[
    tool!(
        "Front matter",
        Command::Add(SectionKind::FrontMatter),
        "M5 3h10l4 4v14H5zM14 3v5h5M8 11h8M8 15h8M8 18h5"
    ),
    tool!(
        "Mermaid",
        Command::Add(SectionKind::Mermaid),
        "M9 3h6v5H9zM3 16h6v5H3zM15 16h6v5h-6zM12 8v4m-6 4v-4h12v4"
    ),
    tool!(
        "Inline math",
        Command::Add(SectionKind::InlineMath),
        "M3 12h3m12 0h3M9 8l6 8m0-8-6 8"
    ),
    tool!(
        "Display math",
        Command::Add(SectionKind::DisplayMath),
        "M19 4H6l7 8-7 8h13"
    ),
    tool!(
        "Image",
        Command::Add(SectionKind::Image),
        "M4 5h16v14H4zM4 16l5-5 4 4 2-2 5 5M15 9h.1"
    ),
    tool!(
        "Link",
        Command::Add(SectionKind::Link),
        "m10 13 4-4m-5 7-2 2a3 3 0 0 1-4-4l4-4a3 3 0 0 1 4 0m4-2 2-2a3 3 0 0 1 4 4l-4 4a3 3 0 0 1-4 0"
    ),
];

const EDIT: &[Tool] = &[
    tool!(
        "Edit link",
        Command::EditLink,
        "m10 12 4-4m-5 7-2 2a3 3 0 0 1-4-4l4-4a3 3 0 0 1 4 0m4-2 2-2a3 3 0 0 1 4 4l-2 2m-6 8 1-4 5-5 3 3-5 5z"
    ),
    tool!(
        "Remove item",
        Command::RemoveItem,
        "M4 7h16M9 7V4h6v3m3 0-1 13H7L6 7m4 4v5m4-5v5"
    ),
];

const GROUPS: &[&[Tool]] = &[FORMAT, INSERT, EDIT];

pub(crate) struct ToolbarAssets;

impl AssetSource for ToolbarAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(GROUPS
            .iter()
            .flat_map(|group| group.iter())
            .find(|tool| tool.name == path)
            .map(|tool| Cow::Borrowed(tool.icon.as_bytes())))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(GROUPS
            .iter()
            .flat_map(|group| group.iter())
            .filter(|tool| tool.name.starts_with(path))
            .map(|tool| tool.name.into())
            .collect())
    }
}

pub(super) fn toolbar(cx: &mut Context<Editor>) -> impl IntoElement {
    div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .bg(rgb(0xf8fafc))
        .border_b_1()
        .border_color(rgb(0xe2e8f0))
        .children(GROUPS.iter().enumerate().map(|(index, group)| {
            div()
                .flex()
                .items_center()
                .gap_1()
                .when(index != 0, |group| {
                    group.border_l_1().border_color(rgb(0xdce3ed)).pl_2()
                })
                .children(group.iter().map(|tool| {
                    let name = tool.name;
                    let command = tool.command;
                    div()
                        .id(name)
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_md()
                        .cursor_pointer()
                        .hover(|style| style.bg(rgb(0xe2e8f0)))
                        .active(|style| style.bg(rgb(0xcbd5e1)))
                        .tooltip(move |_, cx| cx.new(|_| Tooltip(name)).into())
                        .on_click(cx.listener(move |editor, _, window, cx| {
                            editor.command(command, window, cx)
                        }))
                        .child(svg().path(name).size(px(16.)).text_color(rgb(0x334155)))
                }))
        }))
}

struct Tooltip(&'static str);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(rgb(0x1e293b))
            .text_color(rgb(0xffffff))
            .text_sm()
            .child(self.0)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use resvg::{
        render,
        tiny_skia::{Pixmap, Transform},
        usvg,
    };

    use super::*;

    #[test]
    fn toolbar_assets_are_distinct_and_draw_visible_icons() {
        let mut names = HashSet::new();
        for tool in GROUPS.iter().flat_map(|group| group.iter()) {
            assert!(names.insert(tool.name), "duplicate icon: {}", tool.name);
            let data = ToolbarAssets.load(tool.name).unwrap().unwrap();
            let svg = usvg::Tree::from_data(&data, &usvg::Options::default()).unwrap();
            let mut pixels = Pixmap::new(24, 24).unwrap();
            render(&svg, Transform::identity(), &mut pixels.as_mut());
            assert!(
                pixels.pixels().iter().any(|pixel| pixel.alpha() != 0),
                "blank icon: {}",
                tool.name
            );
        }
    }
}
