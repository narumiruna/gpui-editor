use std::borrow::Cow;

use gpui::{
    AssetSource, Context, Div, Entity, FontWeight, SharedString, Stateful, Window, div, prelude::*,
    px, svg,
};

use super::{Editor, commands::Command};
use crate::{app::developer::Picker, document::SectionKind, theme::Theme};

struct Tool {
    name: &'static str,
    icon: &'static str,
    command: Command,
}

macro_rules! glyph {
    ($path:literal) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><path d=""#,
            $path,
            r#""/></svg>"#,
        )
    };
}

macro_rules! tool {
    ($name:literal, $command:expr, $path:literal) => {
        Tool {
            name: $name,
            icon: glyph!($path),
            command: $command,
        }
    };
}

const SHELL_ICONS: &[(&str, &str)] = &[
    (
        "Open Folder…",
        glyph!(
            "M3 18V6a2 2 0 0 1 2-2h4l2 3h7a2 2 0 0 1 2 2v1M3 18l3.1-6.8A2 2 0 0 1 7.9 10H21a1 1 0 0 1 .9 1.4l-3 6.5a2 2 0 0 1-1.8 1.1H4a1 1 0 0 1-1-1Z"
        ),
    ),
    (
        "New note",
        glyph!(
            "M13 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V10l-7-7Zm0 0v7h7M12 12v6m-3-3h6"
        ),
    ),
    (
        "Save",
        glyph!("M12 3v11m-4-4 4 4 4-4M4 15v4a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-4"),
    ),
    ("More tools", glyph!("M5 12h.01M12 12h.01M19 12h.01")),
    ("Workspace", glyph!("M3 6h7l2 2h9v11H3z")),
    ("File", glyph!("M6 3h8l4 4v14H6zM14 3v5h5M9 12h6m-6 4h6")),
    ("Hide Sidebar", glyph!("m14 6-6 6 6 6")),
    ("Show Sidebar", glyph!("m10 6 6 6-6 6")),
    ("Expanded tree", glyph!("m6 9 6 6 6-6")),
    ("Collapsed tree", glyph!("m9 6 6 6-6 6")),
    (
        "Load remote images for this document",
        glyph!("M5 13H4a2 2 0 0 1-2-2V5h20v6a2 2 0 0 1-2 2h-1M5 5l4 4 3-3m0 4v11m-4-4 4 4 4-4"),
    ),
    (
        "Developer target",
        glyph!(
            "M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0Zm-4 0a5 5 0 1 1-10 0 5 5 0 0 1 10 0Zm-4 0a1 1 0 1 1-2 0 1 1 0 0 1 2 0Z"
        ),
    ),
];

const FORMAT: &[Tool] = &[
    tool!(
        "Heading",
        Command::Heading,
        "M4 5v14M12 5v14M4 12h8m4-1 3-2v10m-3 0h5"
    ),
    tool!(
        "Bold",
        Command::Bold,
        "M7 4h6a4 4 0 0 1 0 8H7m0-8v16h7a4 4 0 0 0 0-8H7"
    ),
    tool!("Italic", Command::Italic, "M11 5h8M5 19h8m2-14L9 19"),
    tool!(
        "List",
        Command::Bullet,
        "M9 6h11M9 12h11M9 18h11M4 6h.1M4 12h.1M4 18h.1"
    ),
];

const INSERT: &[Tool] = &[
    tool!(
        "Diagram",
        Command::Add(SectionKind::Mermaid),
        "M2 8h6v8H2zM16 8h6v8h-6zM8 12h8m-3-3 3 3-3 3"
    ),
    tool!(
        "Formula",
        Command::Add(SectionKind::InlineMath),
        "M2 13h3l2 5 3-12h12m-8 4 6 6m0-6-6 6"
    ),
    tool!(
        "Link",
        Command::Add(SectionKind::Link),
        "m10 13 4-4m-5 7-2 2a3 3 0 0 1-4-4l4-4a3 3 0 0 1 4 0m4-2 2-2a3 3 0 0 1 4 4l-4 4a3 3 0 0 1-4 0"
    ),
];

const MORE: &[Tool] = &[
    tool!(
        "Front matter",
        Command::Add(SectionKind::FrontMatter),
        "M5 3h10l4 4v14H5zM14 3v5h5M8 11h8M8 15h8M8 18h5"
    ),
    tool!(
        "Display formula",
        Command::Add(SectionKind::DisplayMath),
        "M19 4H6l7 8-7 8h13"
    ),
    tool!(
        "Image",
        Command::Add(SectionKind::Image),
        "M4 5h16v14H4zM4 16l5-5 4 4 2-2 5 5M15 9h.1"
    ),
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

fn assets() -> impl Iterator<Item = (&'static str, &'static str)> {
    SHELL_ICONS.iter().copied().chain(
        [FORMAT, INSERT, MORE]
            .into_iter()
            .flatten()
            .map(|tool| (tool.name, tool.icon)),
    )
}

pub(crate) struct ToolbarAssets;

impl AssetSource for ToolbarAssets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(assets()
            .find(|(name, _)| *name == path)
            .map(|(_, glyph)| Cow::Borrowed(glyph.as_bytes())))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(assets()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| name.into())
            .collect())
    }
}

pub(crate) fn icon(name: &'static str, size: f32) -> gpui::Svg {
    svg().path(name).size(px(size)).flex_none()
}

pub(crate) fn marked_button(
    name: &'static str,
    enabled: bool,
    theme: Theme,
    picker: Option<&Picker>,
) -> Stateful<Div> {
    Picker::mark(picker, button(name, enabled, true, theme), name, name)
}

fn button(name: &'static str, enabled: bool, icon_only: bool, theme: Theme) -> Stateful<Div> {
    let primary = matches!(name, "Save" | "Open Folder…") && enabled;
    let expanded = name == "Fewer tools";
    let icon_name = if expanded { "More tools" } else { name };
    let color = if primary {
        theme.on_accent
    } else if enabled {
        theme.text
    } else {
        theme.muted
    };
    div()
        .id(name)
        .debug_selector(|| name.to_owned())
        .flex_none()
        .flex()
        .items_center()
        .gap_1()
        .min_h(px(30.))
        .px(px(6.))
        .rounded_md()
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(px(13.))
        .text_color(color)
        .when(primary, |el| el.bg(theme.accent))
        .when(expanded, |el| el.bg(theme.selected))
        .when(enabled, |el| {
            el.cursor_pointer()
                .hover(move |s| {
                    s.bg(if primary {
                        theme.surface.blend(gpui::Rgba {
                            a: 0.85,
                            ..theme.accent
                        })
                    } else {
                        theme.selected
                    })
                })
                .active(move |s| {
                    s.bg(if primary {
                        theme.surface.blend(gpui::Rgba {
                            a: 0.75,
                            ..theme.accent
                        })
                    } else {
                        theme.border
                    })
                })
        })
        .child(icon(icon_name, 17.).text_color(color))
        .when(icon_only, |el| {
            el.w(px(30.))
                .justify_center()
                .tooltip(move |_, cx| cx.new(|_| Tooltip(name)).into())
        })
        .when(!icon_only, |el| el.child(name))
}

pub(crate) struct Tooltip(pub(crate) &'static str);

impl gpui::Render for Tooltip {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        tooltip(self.0.into(), window).debug_selector(|| format!("toolbar-tooltip: {}", self.0))
    }
}

pub(crate) struct TextTooltip(pub(crate) SharedString);

impl gpui::Render for TextTooltip {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        tooltip(self.0.clone(), window)
    }
}

fn tooltip(label: SharedString, window: &Window) -> Div {
    let theme = Theme::for_window(window);
    div()
        .px_2()
        .py_1()
        .rounded_md()
        .bg(theme.tooltip)
        .text_color(theme.on_tooltip)
        .child(label)
}

pub(crate) fn group() -> Div {
    div().flex().flex_none().items_center().gap_1()
}

pub(crate) fn primary_groups(
    editor: &Entity<Editor>,
    enabled: bool,
    theme: Theme,
    picker: Option<&Picker>,
) -> [Div; 2] {
    [FORMAT, INSERT].map(|tools| {
        tool_group(tools, editor, enabled, theme, picker, true)
            .pl_2()
            .border_l_1()
            .border_color(theme.border)
    })
}

pub(crate) fn more_group(
    editor: &Entity<Editor>,
    enabled: bool,
    theme: Theme,
    picker: Option<&Picker>,
) -> Div {
    tool_group(MORE, editor, enabled, theme, picker, false)
}

fn tool_group(
    tools: &[Tool],
    editor: &Entity<Editor>,
    enabled: bool,
    theme: Theme,
    picker: Option<&Picker>,
    icon_only: bool,
) -> Div {
    div()
        .flex()
        .flex_none()
        .gap_1()
        .when(icon_only, |group| group.items_center())
        .when(!icon_only, |group| group.flex_col())
        .children(tools.iter().map(|tool| {
            let command = tool.command;
            let editor = editor.clone();
            let button = button(tool.name, enabled, icon_only, theme);
            Picker::mark(picker, button, tool.name, tool.name).when(enabled, |button| {
                button.on_click(move |_, window, cx| {
                    editor.update(cx, |editor, cx| editor.command(command, window, cx));
                })
            })
        }))
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
        let mut drawings = HashSet::new();
        for (name, _) in assets() {
            assert!(names.insert(name), "duplicate icon name: {name}");
            let data = ToolbarAssets.load(name).unwrap().unwrap();
            assert!(
                drawings.insert(data.clone()),
                "duplicate icon drawing: {name}"
            );
            let svg = usvg::Tree::from_data(&data, &usvg::Options::default()).unwrap();
            let mut pixels = Pixmap::new(24, 24).unwrap();
            render(&svg, Transform::identity(), &mut pixels.as_mut());
            assert!(
                pixels.pixels().iter().any(|pixel| pixel.alpha() != 0),
                "blank icon: {name}"
            );
        }
    }
}
