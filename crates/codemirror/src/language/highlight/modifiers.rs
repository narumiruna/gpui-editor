use std::{
    cmp::Ordering,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering as AtomicOrdering},
    },
};

/// The standard tag modifiers, in Lezer's definition order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TagModifier {
    Definition,
    Constant,
    Function,
    Standard,
    Local,
    Special,
}

impl TagModifier {
    pub(super) const ALL: [Self; 6] = [
        Self::Definition,
        Self::Constant,
        Self::Function,
        Self::Standard,
        Self::Local,
        Self::Special,
    ];

    pub(super) const fn bit(self) -> u8 {
        1 << self as u8
    }

    pub(super) fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "definition" => Self::Definition,
            "constant" => Self::Constant,
            "function" => Self::Function,
            "standard" => Self::Standard,
            "local" => Self::Local,
            "special" => Self::Special,
            _ => return None,
        })
    }
}

/// A custom highlight modifier. Independent definitions with the same name
/// do not match. Clones keep the same identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomTagModifier {
    id: usize,
    name: Arc<str>,
}

impl CustomTagModifier {
    /// Define a modifier. Its name is descriptive, not a selector key.
    pub fn define(name: impl Into<Arc<str>>) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let id = NEXT
            .fetch_update(AtomicOrdering::Relaxed, AtomicOrdering::Relaxed, |id| {
                id.checked_add(1)
            })
            .expect("highlight modifier identities exhausted");
        Self {
            id,
            name: name.into(),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Modifiers {
    standard: u8,
    custom: Option<Arc<[CustomTagModifier]>>,
}

impl Modifiers {
    pub(super) fn is_empty(&self) -> bool {
        self.standard == 0 && self.custom.is_none()
    }

    pub(super) fn add_standard(&mut self, modifier: TagModifier) {
        self.standard |= modifier.bit();
    }

    pub(super) fn add_custom(&mut self, modifier: &CustomTagModifier) {
        let old = self.custom();
        if let Err(index) = old.binary_search_by_key(&modifier.id, |entry| entry.id) {
            self.custom = Some(
                old[..index]
                    .iter()
                    .chain([modifier])
                    .chain(&old[index..])
                    .cloned()
                    .collect(),
            );
        }
    }

    pub(super) fn standard(&self) -> &[TagModifier] {
        // Expose an ordered slice without allocating a list for built-in modifiers.
        const SETS: [[TagModifier; 6]; 64] = {
            let mut sets = [[TagModifier::Definition; 6]; 64];
            let mut mask = 0;
            while mask < sets.len() {
                let mut i = 0;
                let mut count = 0;
                while i < TagModifier::ALL.len() {
                    let modifier = TagModifier::ALL[i];
                    if mask & modifier.bit() as usize != 0 {
                        sets[mask][count] = modifier;
                        count += 1;
                    }
                    i += 1;
                }
                mask += 1;
            }
            sets
        };
        &SETS[usize::from(self.standard)][..self.standard.count_ones() as usize]
    }

    pub(super) fn custom(&self) -> &[CustomTagModifier] {
        self.custom.as_deref().unwrap_or_default()
    }

    pub(super) fn pattern(&self) -> ModifierPattern<'_> {
        ModifierPattern {
            standard: self.standard,
            custom: self.custom(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ModifierPattern<'a> {
    pub(super) standard: u8,
    custom: &'a [CustomTagModifier],
}

impl ModifierPattern<'_> {
    pub(super) const EMPTY: Self = Self {
        standard: 0,
        custom: &[],
    };

    pub(super) fn is_subset_of(self, other: Self) -> bool {
        if self.standard & other.standard != self.standard || self.custom.len() > other.custom.len()
        {
            return false;
        }
        let mut other = other.custom.iter().map(|m| m.id).peekable();
        self.custom.iter().all(|m| {
            while other.next_if(|&id| id < m.id).is_some() {}
            other.next_if_eq(&m.id).is_some()
        })
    }

    pub(super) fn compare_specificity(self, other: Self) -> Ordering {
        let count = |set: Self| set.custom.len() + set.standard.count_ones() as usize;
        // For equal sizes Lezer tries the smaller subset in definition-bit
        // order. Compare the highest differing ID without building a power set.
        count(self)
            .cmp(&count(other))
            .then_with(|| {
                other
                    .custom
                    .iter()
                    .rev()
                    .map(|m| m.id)
                    .cmp(self.custom.iter().rev().map(|m| m.id))
            })
            .then_with(|| other.standard.cmp(&self.standard))
    }
}
