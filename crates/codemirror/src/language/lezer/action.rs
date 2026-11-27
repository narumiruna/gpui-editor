const REDUCE: u32 = 1 << 16;
const REPEAT_OR_GOTO: u32 = 1 << 17;
const STAY: u32 = 1 << 18;
const DEPTH_SHIFT: u32 = 19;
const END: u16 = u16::MAX;

/// One packed Lezer parser action.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Action(u32);

/// A decoded action. The state and term IDs come from the grammar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionKind {
    /// Consume the token and enter the given state.
    Shift(u16),
    /// Enter a separate skip rule without consuming the token.
    Goto(u16),
    /// Consume the skipped token without changing state.
    Skip,
    Reduce {
        term: u16,
        depth: u16,
        /// This combines repeated items into a repeat node.
        repeat: bool,
        /// This ends a skip rule and restores the state before that rule.
        stay: bool,
    },
}

impl Action {
    pub(super) fn from_raw(raw: u32) -> Self {
        Self(raw)
    }

    pub(super) fn is_reduce(self) -> bool {
        self.0 & REDUCE != 0
    }

    pub(super) fn has_valid_flags(self) -> bool {
        if self.is_reduce() {
            // The pinned runtime extracts the depth with a signed right shift.
            self.0 <= i32::MAX as u32
        } else {
            self.0 < REDUCE || self.0 >> 16 == REPEAT_OR_GOTO >> 16 || self.0 == STAY
        }
    }

    pub fn raw(self) -> u32 {
        self.0
    }

    pub fn kind(self) -> ActionKind {
        let value = self.0 as u16;
        if self.is_reduce() {
            ActionKind::Reduce {
                term: value,
                depth: (self.0 >> DEPTH_SHIFT) as u16,
                repeat: self.0 & REPEAT_OR_GOTO != 0,
                stay: self.0 & STAY != 0,
            }
        } else if self.0 & REPEAT_OR_GOTO != 0 {
            ActionKind::Goto(value)
        } else if self.0 & STAY != 0 {
            ActionKind::Skip
        } else {
            ActionKind::Shift(value)
        }
    }
}

#[derive(Clone, Copy)]
enum Record {
    Token(u16, Action),
    Next(usize),
    Other(Action),
    Done,
}

impl Record {
    fn read(data: &[u16], offset: usize) -> Option<Self> {
        fn pair(lo: u16, hi: u16) -> u32 {
            u32::from(lo) | (u32::from(hi) << 16)
        }
        match *data.get(offset..)? {
            [END, 0, ..] => Some(Self::Done),
            [END, 1, lo, hi, ..] => Some(Self::Next(pair(lo, hi) as usize)),
            [END, 2, lo, hi, ..] => Some(Self::Other(Action(pair(lo, hi)))),
            [term, lo, hi, ..] if term != END => Some(Self::Token(term, Action(pair(lo, hi)))),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Mark {
    Unseen,
    Visiting,
    Checked,
}

pub(super) fn validate(
    data: &[u16],
    offsets: impl Iterator<Item = u32>,
    valid: impl Fn(Option<u16>, Action) -> bool,
) -> Result<(), usize> {
    // Lists can share suffixes. Check each reachable record once and reject
    // jumps that form a cycle.
    let mut marks = vec![Mark::Unseen; data.len()];
    let mut path = Vec::new();
    for offset in offsets {
        let mut pos = offset as usize;
        loop {
            match marks.get(pos).ok_or(pos)? {
                Mark::Checked => break,
                Mark::Visiting => return Err(pos),
                Mark::Unseen => {}
            }
            marks[pos] = Mark::Visiting;
            path.push(pos);
            match Record::read(data, pos).ok_or(pos)? {
                Record::Token(term, action) if valid(Some(term), action) => pos += 3,
                Record::Next(next) => {
                    // Lezer uses the target as a token record directly.
                    if !matches!(Record::read(data, next), Some(Record::Token(..))) {
                        return Err(next);
                    }
                    pos = next;
                }
                Record::Other(action) if valid(None, action) => break,
                Record::Done => break,
                _ => return Err(pos),
            }
        }
        for visited in path.drain(..) {
            marks[visited] = Mark::Checked;
        }
    }
    Ok(())
}

pub(super) fn entries(
    data: &[u16],
    offset: usize,
) -> impl Iterator<Item = (Option<u16>, Action)> + '_ {
    let mut next = Some(offset);
    std::iter::from_fn(move || {
        let mut pos = next.take()?;
        loop {
            match Record::read(data, pos).expect("validated action record") {
                Record::Token(term, action) => {
                    next = Some(pos + 3);
                    return Some((Some(term), action));
                }
                Record::Next(offset) => pos = offset,
                Record::Other(action) => return Some((None, action)),
                Record::Done => return None,
            }
        }
    })
}
