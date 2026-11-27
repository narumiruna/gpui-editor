use std::ops::Range;

use crate::{Assoc, CharCategorizer, CharCategory};

use super::{CloseBracketConfig, Plan, next_char, paired, prev_char, skip, space};

/// Syntax queries used by the quote rules. Node IDs belong to this tree.
/// Ranges and positions use byte offsets in the current document.
pub trait CloseBracketSyntax {
    /// Find the innermost node, including in nested languages. At an edge,
    /// `Before` includes nodes ending here, `After` includes nodes starting
    /// here, and `None` excludes both.
    fn resolve(&self, pos: usize, side: Option<Assoc>) -> Option<usize>;
    fn range(&self, node: usize) -> Range<usize>;
    fn parent(&self, node: usize) -> Option<usize>;
    fn first_child(&self, node: usize) -> Option<usize>;
}

pub(super) fn insert(
    source: &str,
    pos: usize,
    token: &str,
    tracked: bool,
    config: CloseBracketConfig<'_>,
    words_at: &impl Fn(usize) -> Option<CharCategorizer>,
) -> Option<Plan> {
    let quote = token.chars().next()?;
    let triple = token.repeat(3);
    let allow_triple = config.brackets.contains(&triple.as_str());
    let node_starts_at = |pos| {
        let Some(tree) = config.syntax else {
            return false;
        };
        next_char(source, pos)
            .and_then(|ch| tree.resolve(pos + ch.len_utf8(), None))
            .is_some_and(|node| tree.parent(node).is_some() && tree.range(node).start == pos)
    };
    let next = next_char(source, pos);
    if next == Some(quote) {
        if node_starts_at(pos) {
            return Some(paired(pos, token, token));
        }
        if tracked {
            let content = if allow_triple && source[pos..].starts_with(&triple) {
                &triple
            } else {
                token
            };
            return Some(skip(pos, content));
        }
    } else if allow_triple
        && source[..pos].ends_with(&token.repeat(2))
        && can_start(source, pos - 2 * token.len(), config, words_at).is_some_and(node_starts_at)
    {
        return Some(paired(pos, token, &triple));
    } else if !next.is_some_and(|ch| is_word(ch, config.word_chars, words_at(pos).as_ref()))
        && can_start(source, pos, config, words_at).is_some()
        && !probably_in_string(source, pos, token, config)
    {
        return Some(paired(pos, token, token));
    }
    None
}

fn can_start(
    source: &str,
    pos: usize,
    config: CloseBracketConfig<'_>,
    words_at: &impl Fn(usize) -> Option<CharCategorizer>,
) -> Option<usize> {
    let words = words_at(pos);
    let follows_word = |at| {
        prev_char(source, at).is_some_and(|ch| {
            // The pinned helper reads only one UTF-16 unit before this position.
            let mut buffer = [0; 2];
            let units = ch.encode_utf16(&mut buffer);
            let last = units[units.len() - 1];
            match &words {
                Some(words) => words.is_word_unit(last),
                None if units.len() == 1 => is_word(ch, config.word_chars, None),
                None => matches_extra_unit(config.word_chars, &[last]),
            }
        })
    };
    if !follows_word(pos) {
        return Some(pos);
    }
    config.string_prefixes.iter().find_map(|prefix| {
        let before = source[..pos].strip_suffix(prefix)?.len();
        (!follows_word(before)).then_some(before)
    })
}

fn is_word(ch: char, extra: &str, words: Option<&CharCategorizer>) -> bool {
    if let Some(words) = words {
        words.category(ch.encode_utf8(&mut [0; 4])) == CharCategory::Word
    } else {
        !space(ch)
            && (ch.is_alphanumeric()
                || ch == '_'
                || matches_extra_unit(extra, ch.encode_utf16(&mut [0; 2])))
    }
}

fn matches_extra_unit(word_chars: &str, units: &[u16]) -> bool {
    word_chars.encode_utf16().any(|unit| units.contains(&unit))
}

fn probably_in_string(
    source: &str,
    pos: usize,
    token: &str,
    config: CloseBracketConfig<'_>,
) -> bool {
    let Some(tree) = config.syntax else {
        return false;
    };
    let Some(mut node) = tree.resolve(pos, Some(Assoc::Before)) else {
        return false;
    };
    let max_prefix = config
        .string_prefixes
        .iter()
        .map(|prefix| prefix.len())
        .max()
        .unwrap_or(0);
    for _ in 0..5 {
        let range = tree.range(node);
        let Some(text) = source.get(range.clone()) else {
            return false;
        };
        let max_start = text.floor_char_boundary(text.len().min(token.len() + max_prefix));
        if let Some(quote) = text[..max_start].find(token)
            && (quote == 0 || config.string_prefixes.contains(&&text[..quote]))
        {
            let mut first = tree.first_child(node);
            while let Some(child) = first {
                let child_range = tree.range(child);
                if child_range.start != range.start || child_range.len() <= token.len() + quote {
                    break;
                }
                if source
                    .get(child_range.clone())
                    .is_some_and(|text| text.ends_with(token))
                {
                    return false;
                }
                first = tree.first_child(child);
            }
            return true;
        }
        match tree.parent(node) {
            Some(parent) if range.end == pos => node = parent,
            _ => break,
        }
    }
    false
}
