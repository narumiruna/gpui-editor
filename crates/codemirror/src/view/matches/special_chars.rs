use std::sync::{Arc, LazyLock};

use regex::Regex;

use crate::{
    ConfigurationError, EditError, EditorState, Extension, Facet, Range, Text,
    language::line_projection::configured_editor_lines,
};

use super::{DEFAULT_CONTEXT, Viewport, for_line_fragment, make_set, match_ranges};
use crate::view::decorations::{Decoration, DecorationSet, ReplaceSpec};

/// The text to draw in place of a special character. The host supplies
/// rendering and accessibility for the `cm-specialChar` class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpecialChar {
    pub character: char,
}

impl SpecialChar {
    pub fn placeholder(self) -> char {
        match self.character {
            '\n' => '\u{2424}',
            ch if ch < ' ' => char::from_u32(0x2400 + u32::from(ch)).unwrap(),
            _ => '\u{2022}',
        }
    }

    /// The default English description. The host may localize this text.
    pub fn description(self) -> String {
        self.describe("Control character")
    }

    fn describe(self, phrase: &str) -> String {
        let name = match self.character {
            '\0' => "null",
            '\u{0007}' => "bell",
            '\u{0008}' => "backspace",
            '\n' => "newline",
            '\u{000b}' => "vertical tab",
            '\r' => "carriage return",
            '\u{001b}' => "escape",
            '\u{200b}' => "zero width space",
            '\u{200c}' => "zero width non-joiner",
            '\u{200d}' => "zero width joiner",
            '\u{200e}' => "left-to-right mark",
            '\u{200f}' => "right-to-left mark",
            '\u{2028}' => "line separator",
            '\u{2029}' => "paragraph separator",
            '\u{202d}' => "left-to-right override",
            '\u{202e}' => "right-to-left override",
            '\u{2066}' => "left-to-right isolate",
            '\u{2067}' => "right-to-left isolate",
            '\u{2069}' => "pop directional isolate",
            '\u{feff}' => "zero width no-break space",
            '\u{fffc}' => "object replacement",
            ch => return format!("{phrase} 0x{:x}", u32::from(ch)),
        };
        format!("{phrase} {name}")
    }
}

static SPECIALS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"[\x00-\x08\x0a-\x1f\x7f-\x9f\u{ad}\u{61c}\u{200b}\u{200e}\u{200f}",
        r"\u{2028}\u{2029}\u{202d}\u{202e}\u{2066}\u{2067}\u{2069}\u{feff}\u{fff9}-\u{fffc}]",
    ))
    .expect("valid stock special-character pattern")
});

/// Replace stock special characters in the visible ranges and their context.
/// Logical line endings are not matched. Tabs stay under the host's normal
/// layout policy; the browser's tab-width fallback is not part of this API.
pub fn highlight_special_chars(
    text: &Text,
    viewport: &Viewport,
) -> Result<DecorationSet<SpecialChar>, EditError> {
    project(text, viewport, &SPECIALS)
}

fn project(
    text: &Text,
    viewport: &Viewport,
    pattern: &Regex,
) -> Result<DecorationSet<SpecialChar>, EditError> {
    let mut ranges = Vec::new();
    for range in match_ranges(text, viewport, DEFAULT_CONTEXT)? {
        for_line_fragment(text, range, |start, fragment| {
            ranges.extend(pattern.find_iter(fragment).filter_map(|found| {
                let character = found.as_str().chars().next()?;
                if character == '\t' {
                    return None;
                }
                let widget =
                    Decoration::replace(Some(SpecialChar { character }), ReplaceSpec::default());
                Some(Range::new(
                    start + found.start(),
                    start + found.end(),
                    widget,
                ))
            }));
            Ok(())
        })?;
    }
    make_set(text, ranges, false)
}

/// Return custom display text, or `None` to keep the stock glyph. The
/// arguments are the first matched scalar, its localized description, and
/// the stock glyph. The host owns widget layout and accessibility.
pub type SpecialCharRender = Arc<dyn Fn(char, &str, char) -> Option<String> + Send + Sync>;

#[derive(Clone, Default)]
pub struct SpecialCharConfig {
    /// Replace the stock matcher with this Rust regular expression.
    pub special_chars: Option<Arc<Regex>>,
    /// Add another branch after the stock or replacement pattern. As upstream
    /// does, combine the pattern sources, not flags supplied to a builder.
    pub add_special_chars: Option<Arc<Regex>>,
    pub render: Option<SpecialCharRender>,
}

#[derive(Clone)]
struct Configured {
    pattern: Regex,
    render: Option<SpecialCharRender>,
}

static OPTIONS: LazyLock<Facet<SpecialCharConfig, Vec<SpecialCharConfig>>> =
    LazyLock::new(Facet::collect);
static CONFIG: LazyLock<Facet<Configured, Option<Configured>>> = LazyLock::new(|| {
    Facet::new(|values: &[&Configured]| values.first().map(|&value| value.clone()))
});
static RESOLVE_CONFIG: LazyLock<Extension> = LazyLock::new(|| {
    CONFIG.compute_with_dependencies(
        |deps| deps.facet(&OPTIONS),
        |state| {
            let configs = state.facet(&OPTIONS)?;
            let mut pattern = None;
            let mut additional = None;
            let mut render = None;
            for config in configs.iter() {
                merge(&mut pattern, &config.special_chars)?;
                merge(&mut additional, &config.add_special_chars)?;
                merge(&mut render, &config.render)?;
            }
            let pattern = pattern.as_deref().unwrap_or(&SPECIALS);
            let pattern = if let Some(additional) = additional {
                Regex::new(&format!(
                    "(?:{})|(?:{})",
                    pattern.as_str(),
                    additional.as_str()
                ))
                .map_err(|_| ConfigurationError::InvalidFieldData)?
            } else {
                pattern.clone()
            };
            Ok(Configured { pattern, render })
        },
    )
});

fn merge<T: ?Sized>(
    slot: &mut Option<Arc<T>>,
    input: &Option<Arc<T>>,
) -> Result<(), ConfigurationError> {
    if let Some(value) = input {
        if slot
            .as_ref()
            .is_some_and(|previous| !Arc::ptr_eq(previous, value))
        {
            return Err(ConfigurationError::InvalidFieldData);
        }
        *slot = Some(value.clone());
    }
    Ok(())
}

/// Enable configurable special-character marks. Distinct values for one
/// option conflict. Reuse the same shared regex or callback for duplicates.
pub fn special_char_extension(config: SpecialCharConfig) -> Extension {
    Extension::group([OPTIONS.of(config), RESOLVE_CONFIG.clone()])
}

/// Project the enabled special-character marks from the current snapshot.
/// A replacement covers the full nonempty match; the widget represents its
/// first Unicode scalar. Tabs remain under the host's layout policy.
pub fn special_char_decorations(
    state: &EditorState,
    viewport: &Viewport,
) -> Result<DecorationSet<SpecialChar>, EditError> {
    let context = state.context();
    let options = context.facet(&CONFIG)?;
    let Some(config) = options.as_ref() else {
        return Ok(DecorationSet::empty());
    };
    let projected = configured_editor_lines(&context)?;
    let lines = projected.as_ref().as_ref().unwrap_or(context.text());
    project(lines, viewport, &config.pattern)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpecialCharAppearance {
    pub glyph: String,
    pub description: String,
}

/// Resolve appearance when painting a widget. Use the current snapshot for
/// phrase localization and the configured text renderer.
pub fn special_char_appearance(
    state: &EditorState,
    special: SpecialChar,
) -> Result<SpecialCharAppearance, ConfigurationError> {
    let config = state.context().facet(&CONFIG)?;
    let placeholder = special.placeholder();
    let description = special.describe(&state.phrase("Control character", &[]));
    let glyph = config
        .as_ref()
        .as_ref()
        .and_then(|config| config.render.as_ref())
        .and_then(|render| render(special.character, &description, placeholder))
        .unwrap_or_else(|| placeholder.to_string());
    Ok(SpecialCharAppearance { glyph, description })
}
