use std::{fmt, ops::Range, sync::Arc};

use unicode_segmentation::UnicodeSegmentation;

use crate::{Assoc, CharCategorizer, CharCategory, EditorState, Text, find_cluster_break};

use super::SearchMatch;

type Predicate = dyn Fn(&SearchMatch<'_>, &EditorState) -> bool + Send + Sync;

#[derive(Clone)]
pub(super) struct MatchTest(Arc<Predicate>);

impl MatchTest {
    pub(super) fn new(
        test: impl Fn(&SearchMatch<'_>, &EditorState) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(test))
    }
}

impl PartialEq for MatchTest {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for MatchTest {}

impl fmt::Debug for MatchTest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MatchTest(..)")
    }
}

pub(super) struct MatchFilter {
    word_category: Option<CharCategorizer>,
    test: Option<(MatchTest, Arc<EditorState>)>,
}

impl MatchFilter {
    pub(super) fn new(whole_word: bool) -> Self {
        Self {
            word_category: whole_word.then(CharCategorizer::default),
            test: None,
        }
    }

    pub(super) fn for_query(
        whole_word: bool,
        test: Option<&MatchTest>,
        text: &Text,
        state: Option<&EditorState>,
    ) -> Self {
        let word_category = whole_word.then(|| {
            state.map_or_else(CharCategorizer::default, |state| {
                state.char_categorizer(state.selection().main().head())
            })
        });
        let test = test.map(|test| {
            let state = state
                .cloned()
                .unwrap_or_else(|| EditorState::from_text(text.clone()));
            (test.clone(), Arc::new(state))
        });
        Self {
            word_category,
            test,
        }
    }

    pub(super) fn has_whole_word(&self) -> bool {
        self.word_category.is_some()
    }

    pub(super) fn matches_literal_word(
        &self,
        source: &str,
        range: Range<usize>,
        fragment: Range<usize>,
    ) -> bool {
        let cat = self.word_category.as_ref().expect("whole-word cursor");
        if fragment.start < range.start && range.end < fragment.end {
            let local = range.start - fragment.start..range.end - fragment.start;
            return literal_word_boundaries(cat, &source[fragment], local, [None; 2]);
        }

        // At either fragment edge, the pin replaces the entire buffer with
        // the match and at most two UTF-16 units of context on each side.
        let Range {
            start: from,
            end: to,
        } = range;
        let (before, left_unit) = context_bytes(source[..from].chars().rev(), Assoc::Before);
        let (after, right_unit) = context_bytes(source[to..].chars(), Assoc::After);
        let start = from - before;
        let end = to + after;
        if left_unit.is_none() && right_unit.is_none() && !source[start..end].contains("\r\n") {
            return literal_word_boundaries(
                cat,
                &source[start..end],
                before..to - start,
                [None; 2],
            );
        }

        let mut buffer = String::with_capacity(end - start + 2);
        // A lone surrogate has the cluster behavior of NUL. Track its value
        // separately for categorization and keep it out of native match text.
        if left_unit.is_some() {
            buffer.push('\0');
        }
        let mut append = |segment: &str| {
            for (i, part) in segment.split("\r\n").enumerate() {
                if i != 0 {
                    buffer.push('\n');
                }
                buffer.push_str(part);
            }
            buffer.len()
        };
        let local = append(&source[start..from])..append(&source[from..to]);
        append(&source[to..end]);
        if right_unit.is_some() {
            buffer.push('\0');
        }
        literal_word_boundaries(cat, &buffer, local, [left_unit, right_unit])
    }

    pub(super) fn matches_word(&self, source: &str, range: Range<usize>) -> bool {
        self.word_category.as_ref().is_none_or(|cat| {
            let word = |cluster: &str| cat.category(cluster) == CharCategory::Word;
            [range.start, range.end].into_iter().all(|at| {
                let before = source[..at].graphemes(true).next_back();
                let after = source[at..].graphemes(true).next();
                !before.is_some_and(word) || !after.is_some_and(word)
            })
        })
    }

    pub(super) fn accepts(&self, found: &SearchMatch<'_>) -> bool {
        self.test
            .as_ref()
            .is_none_or(|(test, state)| (test.0)(found, state))
    }
}

fn context_bytes(chars: impl Iterator<Item = char>, side: Assoc) -> (usize, Option<u16>) {
    let (first, second, half) = match side {
        Assoc::Before => ('\n', '\r', 1),
        Assoc::After => ('\r', '\n', 0),
    };
    let mut chars = chars.peekable();
    let mut bytes = 0;
    let mut units = 2;
    while units > 0 {
        let Some(ch) = chars.next() else { break };
        if ch.len_utf16() > units {
            return (bytes, Some(ch.encode_utf16(&mut [0; 2])[half]));
        }
        bytes += ch.len_utf8();
        // A complete CRLF is one separator in CodeMirror's Text.
        if ch == first && chars.next_if_eq(&second).is_some() {
            bytes += 1;
        }
        units -= ch.len_utf16();
    }
    (bytes, None)
}

fn literal_word_boundaries(
    cat: &CharCategorizer,
    buffer: &str,
    range: Range<usize>,
    [left_unit, right_unit]: [Option<u16>; 2],
) -> bool {
    let is_word = |s| cat.category(s) == CharCategory::Word;
    let before = |at| {
        let start = find_cluster_break(buffer, at, Assoc::Before, true).unwrap();
        let cluster = &buffer[start..at];
        if let Some(unit) = left_unit.filter(|_| start == 0) {
            cat.is_word_unit(unit) || is_word(&cluster[1..])
        } else {
            is_word(cluster)
        }
    };
    let after = |at| {
        let end = find_cluster_break(buffer, at, Assoc::After, true).unwrap();
        let cluster = &buffer[at..end];
        if let Some(unit) = right_unit.filter(|_| end == buffer.len()) {
            cat.is_word_unit(unit) || is_word(&cluster[..cluster.len() - 1])
        } else {
            is_word(cluster)
        }
    };
    (!before(range.start) || !after(range.start)) && (!after(range.end) || !before(range.end))
}
