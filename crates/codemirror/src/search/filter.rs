use std::{fmt, ops::Range, sync::Arc};

use unicode_segmentation::UnicodeSegmentation;

use crate::{CharCategorizer, CharCategory, EditorState, Text};

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
