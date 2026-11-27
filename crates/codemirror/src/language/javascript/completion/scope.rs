use std::{collections::HashSet, iter};

use crate::autocomplete::session::SourceResult;

use super::{JavascriptDocument, completion_path, identifier_result, item};

/// Caller-supplied property data for one object. Each prototype entry contains
/// the properties defined at that level, in nearest-to-farthest order.
#[derive(Clone, Copy, Debug, Default)]
pub struct JavascriptScope<'a> {
    pub properties: &'a [JavascriptScopeProperty<'a>],
    pub prototypes: &'a [&'a [JavascriptScopeProperty<'a>]],
}

/// One string-named property, in the caller's enumeration order.
#[derive(Clone, Copy, Debug)]
pub struct JavascriptScopeProperty<'a> {
    pub name: &'a str,
    pub value: JavascriptScopeValue<'a>,
}

/// Completion data for a property. No JavaScript values or getters are run.
#[derive(Clone, Copy, Debug)]
pub enum JavascriptScopeValue<'a> {
    /// Complete the name, but do not descend through it.
    Value,
    /// Complete the name and use this object for a subsequent path segment.
    Object(&'a JavascriptScope<'a>),
    /// Complete the name as a callable, with optional data for its properties.
    Function(Option<&'a JavascriptScope<'a>>),
    /// Hide the name, but still shadow prototype entries with the same name.
    Unavailable,
}

impl<'a> JavascriptScope<'a> {
    fn levels(self) -> impl Iterator<Item = &'a [JavascriptScopeProperty<'a>]> {
        iter::once(self.properties).chain(self.prototypes.iter().copied())
    }
}

/// Build a source for names and directly named properties in a supplied scope.
/// This is the native counterpart of `scopeCompletionSource`. Supply the
/// properties, callable kinds, and prototype order without using a JS runtime.
///
/// Mid-token queries use the upstream offset (the caret minus the full name's
/// UTF-16 length). If that offset does not have a valid UTF-8 byte position,
/// return `None`.
pub fn scope_completion_source<'a>(
    scope: &'a JavascriptScope<'a>,
) -> impl Fn(&JavascriptDocument, usize, bool) -> Option<SourceResult> + 'a {
    move |doc, position, explicit| {
        let query = completion_path(doc, position, explicit)?;
        let mut target = scope;
        for name in &query.path {
            target = match target
                .levels()
                .flatten()
                .find(|entry| entry.name == *name)?
                .value
            {
                JavascriptScopeValue::Object(next) | JavascriptScopeValue::Function(Some(next)) => {
                    next
                }
                JavascriptScopeValue::Value
                | JavascriptScopeValue::Function(None)
                | JavascriptScopeValue::Unavailable => return None,
            };
        }
        let top = query.path.is_empty();
        let from = subtract_utf16(
            doc.text().as_str(),
            position,
            query.name.encode_utf16().count(),
        )?;
        let mut seen = HashSet::new();
        let mut options = Vec::new();
        for (depth, level) in (0..).zip(target.levels()) {
            for entry in level {
                if !property_name(entry.name) || !seen.insert(entry.name) {
                    continue;
                }
                let kind = match entry.value {
                    JavascriptScopeValue::Unavailable => continue,
                    JavascriptScopeValue::Function(_)
                        if entry.name.as_bytes()[0].is_ascii_uppercase() =>
                    {
                        "class"
                    }
                    JavascriptScopeValue::Function(_) if top => "function",
                    JavascriptScopeValue::Function(_) => "method",
                    _ if top => "variable",
                    _ => "property",
                };
                let mut option = item(entry.name, kind);
                option.boost = -depth;
                options.push(option);
            }
        }
        Some(identifier_result(from, options))
    }
}

fn property_name(text: &str) -> bool {
    let letter = |ch: char| {
        ch.is_ascii_alphabetic()
            || matches!(ch, '_' | '$' | '\u{aa}'..='\u{ffdc}')
            || ch.len_utf16() == 2
    };
    let mut chars = text.chars();
    chars.next().is_some_and(letter) && chars.all(|ch| letter(ch) || ch.is_ascii_digit())
}

fn subtract_utf16(source: &str, position: usize, mut length: usize) -> Option<usize> {
    if length == 0 {
        return Some(position);
    }
    for (at, ch) in source[..position].char_indices().rev() {
        length = length.checked_sub(ch.len_utf16())?;
        if length == 0 {
            return Some(at);
        }
    }
    None
}
