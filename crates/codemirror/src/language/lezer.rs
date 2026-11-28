//! Strict LR, bounded GLR, and bounded recovery for version 14 Lezer tables.
//!
//! [`Tables`] decodes the four arrays. [`Parser`] also validates the metadata
//! and token DFAs. It supports numeric token groups, [`LocalTokenGroup`] with
//! its own DFA, precedence and optional `@else` token, external tokenizers,
//! token specialization, context trackers, and skip rules. The owned tree has
//! byte ranges and omits anonymous repeat nodes.
//!
//! [`Parser::parse`] stops on ambiguity. [`Parser::parse_glr`] splits the stack
//! and can rank the results by dynamic precedence. Both require exact goto
//! sources and stop on invalid input or excess work. [`Parser::parse_incremental`]
//! can resume from a validated unchanged prefix and share its output nodes.
//! It can also reuse consuming token/action decisions inside a byte-identical
//! shifted suffix with matching LR states. This does not share suffix nodes.
//! Immutable text specialization tables are eligible. External tokenizers and
//! context trackers are eligible only for prefix reuse and only with an explicit
//! deterministic callback contract. Specialization callbacks are not eligible.
//! [`Parser::parse_recovering`] adds error nodes and can return partial output
//! at EOF. It does not implement all upstream recovery heuristics. No mode
//! supports dialects, ranged or nested parsing, or CodeMirror's time and
//! viewport scheduling. The tree does not use Lezer's packed buffers.
//! WAST, JSON, and Lezer grammar language handles can attach immutable
//! node-type rows to the output tree. A bare LR parser does not attach rows.
//!
//! Upstream `TreeFragment` maps edit ranges and open edges in UTF-16. Above
//! its input size threshold, the LR parser trims open edges, skips packed
//! buffers, and checks stored lookahead. It reuses a nonempty tree through a
//! goto, a node type identity check, and (for strict trackers) a context hash
//! check. It can update context with a reuse hook. The native parser instead
//! checks bytes and reuses checkpoints or scan decisions. It does not implement
//! upstream fragment traversal, edge heuristics, or context reuse hooks.
//!
//! State masks select tokenizer slots. Each local DFA scans with group bit 0
//! and its own precedence list. `@else` consumes input up to the next match
//! or the end of input, and never emits an empty fallback token. Checkpoints
//! include all probe reads, including failed probes and EOF reads.
//!
//! The DFAs and external tokenizers read UTF-16 code units, but token spans
//! must start and end at UTF-8 character boundaries. Local `@else` probes
//! therefore start at scalar boundaries. An explicit `@eof` edge is distinct
//! from a range for U+FFFF.
//!
//! Format and parse references: `@lezer/lr` 1.4.13, `@lezer/generator` 1.8.1,
//! `@lezer/common` 1.5.7, and `@codemirror/language` 6.13.1. See
//! `docs/codemirror-upstream.json`.

// MIT License
// Copyright (C) 2018 by Marijn Haverbeke <marijn@haverbeke.berlin> and others
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

use std::fmt;

mod action;
mod bracket_handle;
mod context;
mod decode;
mod external;
mod fold;
mod fragments;
mod grammar;
pub(super) mod indent;
mod job;
mod language_facet;
mod node_props;
mod parse;
mod publication;
pub(super) mod service_node;
mod style_tags;
mod tokenize;
mod tree;

pub use action::{Action, ActionKind};
pub(in crate::language) use bracket_handle::match_lr_marked;
pub use bracket_handle::{BRACKET_MATCHING_HANDLE, LrBracketHandle};
pub use context::ContextTracker;
pub use decode::DecodeError;
pub use external::{
    ExternalOptions, ExternalToken, ExternalTokenizer, Specialization, Specializer, TokenInput,
    Tokenizer,
};
pub(in crate::language) use fold::stock_fold;
pub use fold::{FOLD, LrFoldCallback, LrFoldNode};
pub(crate) use fragments::NormalizedEdges;
pub use grammar::{GrammarError, GrammarSpec, Parser};
pub use indent::{INDENT, LrIndentContext, LrIndentStrategy};
pub(crate) use job::LrParseJob;
pub use job::{LrParseOutput, LrParseStep};
pub use language_facet::{LANGUAGE_DATA, LanguageFacet};
pub use node_props::{
    CLOSED_BY, GROUP, ISOLATE, Isolation, NodeProp, NodePropSource, NodeType, OPENED_BY,
};
pub(crate) use node_props::{NodeTypeId, NodeTypes};
pub(in crate::language) use parse::session;
pub use parse::{ParseError, ReuseStats, StackView, TreeFragment};
pub(crate) use publication::LrPublication;
pub use service_node::LrSyntaxNode;
pub(crate) use style_tags::{
    ConfiguredHighlights, STYLE_RULES, StyleRules, StyledNode, select_tags,
};
pub use style_tags::{StyleSelectorError, StyleTags, style_tags};
pub use tokenize::LocalTokenGroup;
pub use tree::{SyntaxNode, SyntaxTree};

pub const SERIALIZED_VERSION: u32 = 14;
const STATE_SIZE: usize = 6;

/// The four encoded arrays from a generated Lezer parser spec.
#[derive(Clone, Copy, Debug)]
pub struct SerializedTables<'a> {
    pub version: u32,
    pub states: &'a str,
    pub state_data: &'a str,
    pub goto: &'a str,
    pub token_data: &'a str,
}

/// The six fields of a v14 parser state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct State {
    pub flags: u32,
    pub actions_offset: u32,
    pub skip_offset: u32,
    pub tokenizer_mask: u32,
    pub default_reduce: Option<Action>,
    pub forced_reduce: Option<Action>,
}

impl State {
    /// The state's shifts belong to a skip rule.
    pub fn is_skipped(self) -> bool {
        self.flags & 1 != 0
    }

    /// The state can finish a parse at end of input.
    pub fn is_accepting(self) -> bool {
        self.flags & 2 != 0
    }
}

/// An invalid version, encoding, or LR table structure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TableError {
    UnsupportedVersion(u32),
    Encoding {
        table: &'static str,
        error: DecodeError,
    },
    InvalidStateTable,
    InvalidReduction {
        state: u16,
    },
    InvalidActionSequence {
        offset: usize,
    },
    InvalidGotoTable {
        offset: usize,
    },
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => write!(f, "unsupported Lezer version {version}"),
            Self::Encoding { table, error } => write!(f, "invalid Lezer {table}: {error}"),
            Self::InvalidStateTable => f.write_str("invalid Lezer state table shape"),
            Self::InvalidReduction { state } => {
                write!(f, "invalid Lezer reduction in state {state}")
            }
            Self::InvalidActionSequence { offset } => {
                write!(f, "invalid Lezer action sequence at word {offset}")
            }
            Self::InvalidGotoTable { offset } => {
                write!(f, "invalid Lezer goto table at word {offset}")
            }
        }
    }
}

impl std::error::Error for TableError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Encoding { error, .. } => Some(error),
            _ => None,
        }
    }
}

/// Decoded LR tables. State IDs and term IDs are local to one grammar.
#[derive(Debug)]
pub struct Tables {
    states: Vec<u32>,
    state_data: Vec<u16>,
    goto: Vec<u16>,
    token_data: Vec<u16>,
}

impl Tables {
    /// Decode all four arrays and check the LR record structure.
    ///
    /// This checks state rows and the bounds and termination of action/goto
    /// sequences. It does not check grammar metadata or the tokenizer DFA.
    pub fn deserialize(spec: SerializedTables<'_>) -> Result<Self, TableError> {
        if spec.version != SERIALIZED_VERSION {
            return Err(TableError::UnsupportedVersion(spec.version));
        }
        let tables = Self {
            states: decode_table("states", spec.states)?,
            state_data: decode_table("stateData", spec.state_data)?,
            goto: decode_table("goto", spec.goto)?,
            token_data: decode_table("tokenData", spec.token_data)?,
        };
        if tables.states.is_empty()
            || !tables.states.len().is_multiple_of(STATE_SIZE)
            || tables.state_count() > usize::from(u16::MAX) + 1
        {
            return Err(TableError::InvalidStateTable);
        }
        let rows = tables.states.as_chunks::<STATE_SIZE>().0.iter();
        for (state, row) in rows.clone().enumerate() {
            if row[4..]
                .iter()
                .any(|&code| code != 0 && !Action::from_raw(code).is_reduce())
            {
                return Err(TableError::InvalidReduction {
                    state: state as u16,
                });
            }
        }
        action::validate(
            &tables.state_data,
            rows.flat_map(|row| [row[1], row[2]]),
            |_, _| true,
        )
        .map_err(|offset| TableError::InvalidActionSequence { offset })?;
        validate_goto(&tables.goto, |_| true)
            .map_err(|offset| TableError::InvalidGotoTable { offset })?;
        Ok(tables)
    }

    pub fn states(&self) -> &[u32] {
        &self.states
    }

    pub fn state_data(&self) -> &[u16] {
        &self.state_data
    }

    pub fn goto_data(&self) -> &[u16] {
        &self.goto
    }

    pub fn token_data(&self) -> &[u16] {
        &self.token_data
    }

    pub fn state_count(&self) -> usize {
        self.states.len() / STATE_SIZE
    }

    pub fn state(&self, state: u16) -> Option<State> {
        let start = usize::from(state) * STATE_SIZE;
        let row = self.states.get(start..start + STATE_SIZE)?;
        Some(State {
            flags: row[0],
            actions_offset: row[1],
            skip_offset: row[2],
            tokenizer_mask: row[3],
            default_reduce: (row[4] != 0).then(|| Action::from_raw(row[4])),
            forced_reduce: (row[5] != 0).then(|| Action::from_raw(row[5])),
        })
    }

    /// Probe for the first matching action, as in Lezer's `hasAction`.
    ///
    /// This checks the regular list before the skip list. A row for term 0
    /// (the error term) matches any terminal in this probe. A list's `Other`
    /// record is a fallback. This does not check default or forced reductions.
    pub fn has_action(&self, state: u16, terminal: u16) -> Option<Action> {
        self.probe_action(self.state(state)?, terminal, || {
            Ok::<_, std::convert::Infallible>(())
        })
        .unwrap()
    }

    fn probe_action<E>(
        &self,
        state: State,
        terminal: u16,
        mut visit: impl FnMut() -> Result<(), E>,
    ) -> Result<Option<Action>, E> {
        for (term, action) in self.action_entries(state) {
            visit()?;
            if term.is_none() || term == Some(0) || term == Some(terminal) {
                return Ok((action.raw() != 0).then_some(action));
            }
        }
        Ok(None)
    }

    /// Get the ordered, distinct actions for one token, with no prior token
    /// candidates. Unlike [`Self::has_action`], term 0 is not a wildcard.
    ///
    /// `Other` adds an action only if neither list has added an action yet.
    /// A parser must apply a state's default reduction before it reads a token.
    /// Forced reductions are for recovery and are not included here.
    pub fn actions_for(&self, state: u16, terminal: u16) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(state) = self.state(state) {
            for (term, action) in self.action_entries(state) {
                if (term == Some(terminal) || (term.is_none() && actions.is_empty()))
                    && !actions.contains(&action)
                {
                    actions.push(action);
                }
            }
        }
        actions
    }

    fn action_entries(&self, state: State) -> impl Iterator<Item = (Option<u16>, Action)> + '_ {
        [state.actions_offset, state.skip_offset]
            .into_iter()
            .flat_map(|offset| action::entries(&self.state_data, offset as usize))
    }

    /// Find the next state after a reduction, as in Lezer's `getGoto`.
    ///
    /// If `loose` is true, the last group's target is used without checking
    /// its source list. Use `false` to test whether a transition exists.
    pub fn goto(&self, state: u16, term: u16, loose: bool) -> Option<u16> {
        if self.state(state).is_none() || term >= self.goto[0] {
            return None;
        }
        self.lookup_goto(
            state,
            term,
            loose,
            |_| Ok::<_, std::convert::Infallible>(()),
        )
        .unwrap()
    }

    fn lookup_goto<E>(
        &self,
        state: u16,
        term: u16,
        loose: bool,
        mut visit: impl FnMut(usize) -> Result<(), E>,
    ) -> Result<Option<u16>, E> {
        if term >= self.goto[0] {
            return Ok(None);
        }
        let mut offset = usize::from(self.goto[usize::from(term) + 1]);
        // An absent term points to word 1, which also contains 1. Lezer's
        // loose lookup reads word 2 as the target, if it exists.
        if offset == 1 {
            return Ok(if loose {
                self.goto.get(2).copied()
            } else {
                None
            });
        }
        loop {
            let group = GotoGroup::read(&self.goto, offset).expect("validated goto group");
            visit(group.words())?;
            if (group.last && loose) || group.sources.contains(&state) {
                return Ok(Some(group.target));
            }
            if group.last {
                return Ok(None);
            }
            offset += group.words();
        }
    }
}

fn decode_table<T: TryFrom<u32>>(table: &'static str, input: &str) -> Result<Vec<T>, TableError> {
    decode::array(input).map_err(|error| TableError::Encoding { table, error })
}

struct GotoGroup<'a> {
    target: u16,
    sources: &'a [u16],
    last: bool,
}

impl<'a> GotoGroup<'a> {
    fn read(data: &'a [u16], offset: usize) -> Option<Self> {
        let [tag, target, rest @ ..] = data.get(offset..)? else {
            return None;
        };
        Some(Self {
            target: *target,
            sources: rest.get(..usize::from(tag >> 1))?,
            last: tag & 1 != 0,
        })
    }

    fn words(&self) -> usize {
        2 + self.sources.len()
    }
}

fn validate_goto(data: &[u16], valid: impl Fn(&GotoGroup<'_>) -> bool) -> Result<(), usize> {
    let Some((&count, rest)) = data.split_first() else {
        return Err(0);
    };
    let offsets = rest.get(..usize::from(count)).ok_or(0_usize)?;
    let mut checked = vec![false; data.len()];
    for &offset in offsets {
        let mut pos = usize::from(offset);
        if pos == 1 && offsets[0] == 1 {
            continue;
        }
        if pos <= offsets.len() {
            return Err(pos);
        }
        loop {
            let seen = checked.get_mut(pos).ok_or(pos)?;
            if *seen {
                break;
            }
            let group = GotoGroup::read(data, pos).filter(&valid).ok_or(pos)?;
            *seen = true;
            if group.last {
                break;
            }
            pos += group.words();
        }
    }
    Ok(())
}
