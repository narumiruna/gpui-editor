use std::{
    borrow::Cow,
    cell::{Cell, OnceCell},
    collections::HashMap,
    fmt,
    sync::Arc,
};

use super::{LocalTokenGroup, ParseError, StackView, parse::Work, tokenize::ReadExtent};

/// One slot in the grammar's tokenizer list. State masks index this list.
#[derive(Clone, Debug)]
pub enum Tokenizer {
    /// A group ID in the shared DFA.
    Internal(u8),
    /// An independent DFA and optional fallback token from `@local tokens`.
    Local(LocalTokenGroup),
    External(ExternalTokenizer),
}

impl From<u8> for Tokenizer {
    fn from(group: u8) -> Self {
        Self::Internal(group)
    }
}

/// Options for an external tokenizer. All are false by default.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExternalOptions {
    /// Always scan again. Set this if the tokenizer or its specializer uses
    /// stack information other than the input position and context value.
    pub contextual: bool,
    /// Run even if a higher-priority tokenizer returned a main token without
    /// a new action.
    pub fallback: bool,
    /// Add actions but do not set the main token or prevent other scans.
    pub extend: bool,
}

type Scan = dyn Fn(&mut TokenInput<'_>, &StackView<'_>) -> Result<Option<ExternalToken>, ParseError>
    + Send
    + Sync;

/// A Rust callback for an `@external tokens` declaration.
#[derive(Clone)]
pub struct ExternalTokenizer {
    pub(super) options: ExternalOptions,
    pub(super) scan: Arc<Scan>,
    pub(super) deterministic: bool,
}

impl ExternalTokenizer {
    /// Return `Ok(None)` when no token matches. Each scan starts at the stack's
    /// input position. Propagate errors from input and stack methods with `?`.
    /// The parser meters these methods, not arbitrary work in trusted callbacks.
    /// This constructor does not opt in to incremental checkpoints.
    pub fn new(
        options: ExternalOptions,
        scan: impl Fn(&mut TokenInput<'_>, &StackView<'_>) -> Result<Option<ExternalToken>, ParseError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            options,
            scan: Arc::new(scan),
            deterministic: false,
        }
    }

    /// Opt in to strict LR prefix checkpoints. A resumed parse can omit calls
    /// in the verified prefix. Use only a pure function of the cursor, stack
    /// values, and fixed, immutable configuration (for example, a dialect).
    /// Read document text only through `TokenInput`. Do not use mutable state,
    /// object identity, or side effects. Propagate all input and stack errors.
    /// Set [`ExternalOptions::contextual`] when using LR state or stack probes.
    /// This contract is not checked by Rust and does not change token caching.
    pub fn new_deterministic(
        options: ExternalOptions,
        scan: impl Fn(&mut TokenInput<'_>, &StackView<'_>) -> Result<Option<ExternalToken>, ParseError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            deterministic: true,
            ..Self::new(options, scan)
        }
    }
}

impl fmt::Debug for ExternalTokenizer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalTokenizer")
            .field("options", &self.options)
            .field("deterministic", &self.deterministic)
            .finish_non_exhaustive()
    }
}

/// A token from an external scan. It starts at the scan's initial position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ExternalToken {
    pub term: u16,
    /// Absolute UTF-16 offset. It must be in bounds and on a scalar boundary.
    pub end: usize,
}

/// A result from a grammar's specialization table or callback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Specialization {
    /// Replace the original token's term.
    Replace(u16),
    /// Try this term first, then the original term. Both have the same span.
    Extend(u16),
}

type Specialize =
    dyn Fn(&str, &StackView<'_>) -> Result<Option<Specialization>, ParseError> + Send + Sync;

#[derive(Clone)]
enum SpecializerKind {
    ContextFree(Arc<HashMap<Box<str>, Specialization>>),
    Callback(Arc<Specialize>),
}

/// A handler for one base term: a context-free text table or an external hook.
/// The first handler for the term that returns a result wins.
#[derive(Clone)]
pub struct Specializer {
    pub(super) term: u16,
    kind: SpecializerKind,
}

impl Specializer {
    /// A handler sees the matched text and the read-only stack at scan time.
    /// The result is cached with its tokenizer by position, mask, and context
    /// (using context equality instead of a hash). Only use LR stack probes
    /// when the term comes from an external tokenizer set to contextual.
    /// The parser meters lexeme size and stack probes, not arbitrary callback work.
    pub fn new(
        term: u16,
        get: impl Fn(&str, &StackView<'_>) -> Result<Option<Specialization>, ParseError>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        Self {
            term,
            kind: SpecializerKind::Callback(Arc::new(get)),
        }
    }

    /// Copy an exact-text specialization table. For duplicate keys, the last
    /// entry wins. Parser construction checks the base and result terms.
    /// Unlike callbacks, this table is eligible for incremental checkpoints
    /// because its results depend only on the token bytes.
    pub fn context_free(term: u16, entries: &[(&str, Specialization)]) -> Self {
        let mut table = HashMap::with_capacity(entries.len());
        for &(text, result) in entries {
            table.insert(text.into(), result);
        }
        Self {
            term,
            kind: SpecializerKind::ContextFree(Arc::new(table)),
        }
    }

    pub(super) fn is_context_free(&self) -> bool {
        matches!(self.kind, SpecializerKind::ContextFree(_))
    }

    pub(super) fn result_terms_valid(&self, valid: impl Fn(u16) -> bool) -> bool {
        match &self.kind {
            SpecializerKind::ContextFree(entries) => entries.values().all(|result| match *result {
                Specialization::Replace(term) | Specialization::Extend(term) => valid(term),
            }),
            SpecializerKind::Callback(_) => true,
        }
    }

    pub(super) fn get(
        &self,
        text: &str,
        stack: &StackView<'_>,
    ) -> Result<Option<Specialization>, ParseError> {
        match &self.kind {
            SpecializerKind::ContextFree(entries) => Ok(entries.get(text).copied()),
            SpecializerKind::Callback(get) => get(text, stack),
        }
    }
}

impl fmt::Debug for Specializer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Specializer")
            .field("term", &self.term)
            .field("context_free", &self.is_context_free())
            .finish_non_exhaustive()
    }
}

/// An external scanner's cursor. Positions and relative offsets count UTF-16
/// code units, as in Lezer. A cursor can inspect either half of a surrogate
/// pair, but a returned token cannot end between them.
pub struct TokenInput<'a> {
    text: &'a Input<'a>,
    pos: usize,
    start: usize,
    work: &'a Work,
    reads: Option<&'a Cell<ReadExtent>>,
    lookahead: Option<&'a Cell<usize>>,
}

impl<'a> TokenInput<'a> {
    pub(super) fn track_lookahead(&mut self, lookahead: &'a Cell<usize>) {
        lookahead.set(self.pos + 1);
        self.lookahead = Some(lookahead);
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    /// Read a code unit relative to the cursor. `None` means out of bounds.
    pub fn peek(&self, offset: isize) -> Result<Option<u16>, ParseError> {
        self.work.spend(self.start, 1)?;
        // Arithmetic failure is independent of the input length.
        Ok(self
            .pos
            .checked_add_signed(offset)
            .and_then(|pos| self.read(pos)))
    }

    /// Move forward by up to `units` code units and read the new unit.
    pub fn advance(&mut self, units: usize) -> Result<Option<u16>, ParseError> {
        self.work.spend(self.start, 1)?;
        self.pos = self.pos.saturating_add(units).min(self.text.len());
        Ok(self.read(self.pos))
    }

    fn read(&self, pos: usize) -> Option<u16> {
        let unit = self.text.unit(pos);
        // Peek outside the input and advance to EOF do not extend Lezer's
        // tokenizer observation. Checkpoints must still depend on EOF.
        if unit.is_some()
            && let Some(lookahead) = self.lookahead
        {
            lookahead.set(lookahead.get().max(pos + 1));
        }
        if self.reads.is_some() {
            // An in-bounds read proves the cursor did not clamp at EOF.
            // It also covers bytes skipped by advance and the entire scalar
            // when the callback reads only one half of a surrogate pair.
            let end = if unit.is_some() {
                self.text.unit_byte_end(pos)
            } else {
                self.text.source.len()
            };
            ReadExtent::observe(self.reads, end, unit.is_none());
        }
        unit
    }

    /// Make a token that ends at the current cursor position.
    pub fn token(&self, term: u16) -> ExternalToken {
        ExternalToken {
            term,
            end: self.pos,
        }
    }
}

/// Index UTF-16 input once per parse. ASCII needs no index.
pub(super) struct Input<'a> {
    source: Cow<'a, str>,
    utf16: OnceCell<Option<Utf16Index>>,
}

struct Utf16Index {
    units: Vec<u16>,
    offsets: Vec<usize>,
}

impl<'a> Input<'a> {
    pub(super) fn new(source: Cow<'a, str>) -> Self {
        Self {
            source,
            utf16: OnceCell::new(),
        }
    }

    pub(super) fn source(&self) -> &str {
        &self.source
    }

    pub(super) fn prepare(&self) {
        self.index();
    }

    fn index(&self) -> Option<&Utf16Index> {
        self.utf16
            .get_or_init(|| {
                (!self.source.is_ascii()).then(|| {
                    let mut units = Vec::new();
                    let mut offsets = Vec::new();
                    for (byte, ch) in self.source.char_indices() {
                        for &mut unit in ch.encode_utf16(&mut [0; 2]) {
                            units.push(unit);
                            // Both halves of a surrogate pair have the same byte start.
                            offsets.push(byte);
                        }
                    }
                    offsets.push(self.source.len());
                    Utf16Index { units, offsets }
                })
            })
            .as_ref()
    }

    pub(super) fn cursor<'b>(
        &'b self,
        start: usize,
        work: &'b Work,
        reads: Option<&'b Cell<ReadExtent>>,
    ) -> TokenInput<'b> {
        TokenInput {
            text: self,
            pos: self.utf16_offset(start),
            start,
            work,
            reads,
            lookahead: None,
        }
    }

    pub(super) fn utf16_offset(&self, byte: usize) -> usize {
        self.index().map_or(byte, |Utf16Index { offsets, .. }| {
            offsets.partition_point(|&offset| offset < byte)
        })
    }

    /// Byte end of the scalar that contains an in-bounds UTF-16 unit.
    fn unit_byte_end(&self, pos: usize) -> usize {
        self.index().map_or(pos + 1, |Utf16Index { offsets, .. }| {
            let next = offsets[pos + 1];
            if next == offsets[pos] {
                offsets[pos + 2]
            } else {
                next
            }
        })
    }

    pub(super) fn byte_end(&self, end: usize) -> Option<usize> {
        if let Some(Utf16Index { offsets, .. }) = self.index() {
            let &byte = offsets.get(end)?;
            (end == 0 || offsets[end - 1] != byte).then_some(byte)
        } else {
            (end <= self.source.len()).then_some(end)
        }
    }

    pub(super) fn len(&self) -> usize {
        self.index()
            .map_or(self.source.len(), |index| index.units.len())
    }

    fn unit(&self, pos: usize) -> Option<u16> {
        if let Some(Utf16Index { units, .. }) = self.index() {
            units.get(pos).copied()
        } else {
            self.source.as_bytes().get(pos).copied().map(u16::from)
        }
    }
}
