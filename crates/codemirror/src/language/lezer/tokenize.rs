use std::{borrow::Cow, cell::Cell, sync::Arc};

use super::{
    Action, GrammarError, Parser, Specialization, StackView, State, TokenInput, Tokenizer,
    context::Context,
    decode_table,
    external::Input,
    parse::{ParseError, ReductionGotos, Work},
};

const END: u16 = u16::MAX;
const LOOKAHEAD_MARGIN: usize = 25;

/// The data for a generated `LocalTokenGroup(data, precTable, elseToken)`.
#[derive(Clone, Debug)]
pub struct LocalTokenGroup {
    pub(super) data: Arc<[u16]>,
    precedence: Arc<[u16]>,
    else_token: Option<u16>,
}

impl LocalTokenGroup {
    /// Decode the combined DFA and precedence list. `prec_table` is the
    /// word offset that separates them. The parser checks the DFA and term IDs
    /// when it loads the group with [`Parser::deserialize_with_hooks`].
    pub fn new(
        data: &str,
        prec_table: usize,
        else_token: Option<u16>,
    ) -> Result<Self, GrammarError> {
        let data = decode_table("local token data", data).map_err(GrammarError::Tables)?;
        let invalid = |offset| GrammarError::InvalidLocalTokenPrecedence { offset };
        let Some((dfa, tail)) = data.split_at_checked(prec_table) else {
            return Err(invalid(prec_table));
        };
        let Some((&END, terms)) = tail.split_last() else {
            return Err(invalid(data.len()));
        };
        if let Some(end) = terms.iter().position(|&term| term == END) {
            return Err(invalid(prec_table + end));
        }
        let max_term = terms.iter().copied().max().unwrap_or(0);
        Ok(Self {
            data: dfa.into(),
            precedence: precedence(&data, prec_table, max_term)
                .map_err(invalid)?
                .into(),
            else_token,
        })
    }

    pub(super) fn validate(&self, valid_token: impl Fn(u16) -> bool) -> Result<(), GrammarError> {
        if self.else_token.is_some_and(|term| !valid_token(term)) {
            return Err(GrammarError::InvalidMetadata("local elseToken"));
        }
        if self
            .precedence
            .iter()
            .enumerate()
            .any(|(term, &rank)| rank != END && !valid_token(term as u16))
        {
            return Err(GrammarError::InvalidLocalTokenPrecedence {
                offset: self.data.len(),
            });
        }
        validate(&self.data, valid_token)
            .map_err(|offset| GrammarError::InvalidLocalTokenData { offset })
    }
}

/// A token starts at the current byte position and ends at `end`.
#[derive(Clone, Copy)]
pub(super) struct Token {
    pub(super) term: u16,
    pub(super) end: usize,
}

struct Row<'a> {
    mask: u16,
    accepts: &'a [[u16; 2]],
    edges: &'a [[u16; 3]],
}

impl<'a> Row<'a> {
    fn read(data: &'a [u16], at: usize) -> Option<Self> {
        let &[mask, accept_end, edge_count] = data.get(at..)?.first_chunk()?;
        let accept_end = usize::from(accept_end);
        let (accepts, accept_tail) = data.get(at + 3..accept_end)?.as_chunks();
        let (edges, _) = data.get(accept_end..)?.as_chunks();
        if !accept_tail.is_empty() {
            return None;
        }
        Some(Self {
            mask,
            accepts,
            edges: edges.get(..usize::from(edge_count))?,
        })
    }

    fn words(&self) -> usize {
        3 + self.accepts.len() * 2 + self.edges.len() * 3
    }

    fn edge(&self, unit: Option<u16>) -> Option<usize> {
        let (edges, eof) = if let Some((&[END, END, target], edges)) = self.edges.split_last() {
            (edges, Some(usize::from(target)))
        } else {
            (self.edges, None)
        };
        let Some(unit) = unit else { return eof };
        let after = edges.partition_point(|edge| edge[0] <= unit);
        let &[_, to, target] = edges.get(after.checked_sub(1)?)?;
        (to == 0 || unit < to).then_some(usize::from(target))
    }
}

pub(super) fn validate(data: &[u16], valid_term: impl Fn(u16) -> bool) -> Result<(), usize> {
    if data.is_empty() || data.len() > usize::from(END) + 1 {
        return Err(0);
    }
    let mut starts = vec![false; data.len()];
    let mut pos = 0;
    while pos < data.len() {
        starts[pos] = true;
        let row = Row::read(data, pos).ok_or(pos)?;
        for &[term, mask] in row.accepts {
            if !valid_term(term) || mask == 0 || mask & !row.mask != 0 {
                return Err(pos);
            }
        }
        let mut previous_end = 0;
        for (i, &[from, to, _]) in row.edges.iter().enumerate() {
            if from == END && to == END && i + 1 == row.edges.len() {
                break;
            }
            let end = if to == 0 { 0x10000 } else { u32::from(to) };
            let from = u32::from(from);
            if previous_end > from || from >= end {
                return Err(pos);
            }
            previous_end = end;
        }
        pos += row.words();
    }
    for (pos, _) in starts.iter().enumerate().filter(|&(_, &start)| start) {
        let row = Row::read(data, pos).expect("validated DFA row");
        if row
            .edges
            .iter()
            .any(|edge| starts.get(usize::from(edge[2])) != Some(&true))
        {
            return Err(pos);
        }
    }
    Ok(())
}

pub(super) fn precedence(data: &[u16], start: usize, max_term: u16) -> Result<Box<[u16]>, usize> {
    let mut ranks = vec![END; usize::from(max_term) + 1];
    let mut next_rank = 0;
    for (offset, &term) in data.get(start..).ok_or(start)?.iter().enumerate() {
        if term == END {
            return Ok(ranks.into());
        }
        let slot = ranks.get_mut(usize::from(term)).ok_or(start + offset)?;
        // Published tables can repeat a term. Lezer's findOffset uses the
        // first occurrence. Rank distinct terms to keep the rank below END.
        if *slot == END {
            *slot = next_rank;
            next_rank += 1;
        }
    }
    Err(data.len())
}

struct Cached {
    start: usize,
    mask: u32,
    context: Option<Context>,
    scan: TokenScan,
}

#[derive(Clone, Copy)]
struct TokenScan {
    token: Option<(Token, Option<u16>)>,
    lookahead: usize,
}

pub(super) type Candidate = (Action, Token);

pub(super) struct ActionScan {
    pub(super) main: Option<Token>,
    pub(super) lookahead: usize,
}

#[derive(Clone, Copy)]
pub(super) enum ActionLimit {
    Deterministic,
    Split(usize),
}

/// Cumulative input dependency for a deterministic checkpoint.
#[derive(Clone, Copy, Default)]
pub(super) struct ReadExtent {
    pub(super) end: usize,
    pub(super) eof: bool,
}

impl ReadExtent {
    pub(super) fn observe(reads: Option<&Cell<Self>>, end: usize, eof: bool) {
        if let Some(reads) = reads {
            let old = reads.get();
            reads.set(Self {
                end: old.end.max(end),
                eof: old.eof || eof,
            });
        }
    }
}

pub(super) struct TokenCache<'a> {
    input: Input<'a>,
    slots: Vec<Option<Cached>>,
    pub(super) reduction_gotos: Option<Box<ReductionGotos>>,
    pub(super) reads: Option<Cell<ReadExtent>>,
    recover: bool,
}

impl<'a> TokenCache<'a> {
    pub(super) fn new(source: impl Into<Cow<'a, str>>, parser: &Parser, recover: bool) -> Self {
        Self {
            input: Input::new(source.into()),
            slots: (0..parser.tokenizers.len()).map(|_| None).collect(),
            reduction_gotos: None,
            reads: None,
            recover,
        }
    }

    pub(super) fn source(&self) -> &str {
        self.input.source()
    }

    pub(super) fn prepare_input(&self) {
        self.input.prepare();
    }

    // Return None for a target at or beyond EOF. Round up a target inside a
    // surrogate pair to the next scalar boundary.
    pub(super) fn cutoff_after(&self, start: usize, units: usize) -> Option<usize> {
        let target = self.input.utf16_offset(start).saturating_add(units);
        (target < self.input.len()).then(|| {
            self.input.byte_end(target).unwrap_or_else(|| {
                self.input
                    .byte_end(target + 1)
                    .expect("end of surrogate pair")
            })
        })
    }

    pub(super) fn cursor<'b>(&'b self, start: usize, work: &'b Work) -> TokenInput<'b> {
        self.input.cursor(start, work, self.reads.as_ref())
    }

    pub(super) fn actions(
        &mut self,
        stack: &StackView<'_>,
        state: State,
        limit: ActionLimit,
        found: &mut Vec<Candidate>,
    ) -> Result<ActionScan, ParseError> {
        let (parser, work) = (stack.parser, stack.work);
        let start = stack.byte_offset();
        let mask = state.tokenizer_mask;
        let mut main = None;
        let mut lookahead = 0;
        found.clear();
        let add = |term: u16, end: usize, found: &mut Vec<Candidate>| {
            for (entry, action) in parser.tables.action_entries(state) {
                work.spend(start, 1)?;
                if entry != Some(term) && !(entry.is_none() && found.is_empty()) {
                    continue;
                }
                if matches!(limit, ActionLimit::Split(_)) {
                    work.spend(start, found.len())?;
                }
                // Keep the first token for duplicate packed actions.
                if found.iter().any(|&(prior, _)| prior == action) {
                    continue;
                }
                match limit {
                    ActionLimit::Deterministic if !found.is_empty() => {
                        return Err(ParseError::Ambiguous {
                            offset: start,
                            state: stack.state(),
                            term,
                        });
                    }
                    ActionLimit::Split(limit) if found.len() == limit => {
                        return Err(ParseError::LimitExceeded { offset: start });
                    }
                    _ => found.push((action, Token { term, end })),
                }
            }
            Ok(())
        };
        for i in 0..self.slots.len() {
            if mask & (1 << i) == 0 {
                continue;
            }
            work.spend(start, 1)?;
            let tokenizer = &parser.tokenizers[i];
            let options = match tokenizer {
                Tokenizer::Internal(_) | Tokenizer::Local(_) => Default::default(),
                Tokenizer::External(tokenizer) => tokenizer.options,
            };
            if main.is_some() && !options.fallback {
                continue;
            }
            let context = stack.context_value();
            let scan = match &self.slots[i] {
                Some(cached)
                    if !options.contextual
                        && cached.start == start
                        && cached.mask == mask
                        && cached.context.as_ref() == context =>
                {
                    cached.scan
                }
                _ => {
                    let (token, lookahead) = self.read(parser, tokenizer, stack, work)?;
                    let scan = TokenScan {
                        token: token
                            .map(|token| {
                                parser.specialize(self.source(), start, token, stack, work)
                            })
                            .transpose()?,
                        lookahead,
                    };
                    if !options.contextual {
                        self.slots[i] = Some(Cached {
                            start,
                            mask,
                            context: context.cloned(),
                            scan,
                        });
                    }
                    scan
                }
            };
            // Lezer observes every visited slot, including misses and cache hits.
            lookahead = lookahead.max(scan.lookahead);
            if let Some((token, extended)) = scan.token {
                // Only a non-extending tokenizer that adds a new action ends the scan.
                let previous = found.len();
                if let Some(term) = extended {
                    add(term, token.end, found)?;
                }
                add(token.term, token.end, found)?;
                if !options.extend {
                    main = Some(token);
                    if found.len() > previous {
                        break;
                    }
                }
            }
        }
        if main.is_none() && start == self.source().len() {
            ReadExtent::observe(self.reads.as_ref(), start, true);
            main = Some(Token {
                term: parser.eof,
                end: start,
            });
            add(parser.eof, start, found)?;
        }
        Ok(ActionScan { main, lookahead })
    }

    fn read(
        &mut self,
        parser: &Parser,
        tokenizer: &Tokenizer,
        stack: &StackView<'_>,
        work: &Work,
    ) -> Result<(Option<Token>, usize), ParseError> {
        let start = stack.byte_offset();
        let scan = Scan {
            source: self.source(),
            start,
            work,
            reads: self.reads.as_ref(),
        };
        let (token, observed) = match tokenizer {
            Tokenizer::Internal(group) => scan.dfa(
                start,
                &parser.tables.token_data,
                1 << group,
                &parser.precedence,
            )?,
            Tokenizer::Local(group) => scan.local(group)?,
            Tokenizer::External(tokenizer) => self.read_external(parser, tokenizer, stack, work)?,
        };
        let mut lookahead = 0;
        if self.recover && observed > LOOKAHEAD_MARGIN {
            // Index once per parse, not once per token. An unsuccessful scan
            // uses Lezer's synthetic error end (one UTF-16 unit, clipped at EOF).
            let input = &self.input;
            let from = input.utf16_offset(start);
            let end = token.map_or_else(
                || (from + 1).min(input.len()),
                |token| input.utf16_offset(token.end),
            );
            if from + observed > end + LOOKAHEAD_MARGIN {
                lookahead = from + observed;
            }
        }
        Ok((token, lookahead))
    }

    fn read_external(
        &mut self,
        parser: &Parser,
        tokenizer: &super::ExternalTokenizer,
        stack: &StackView<'_>,
        work: &Work,
    ) -> Result<(Option<Token>, usize), ParseError> {
        let start = stack.byte_offset();
        let recover = self.recover;
        let lookahead = Cell::new(0);
        let mut cursor = self.cursor(start, work);
        let from = cursor.position();
        if recover {
            cursor.track_lookahead(&lookahead);
        }
        let token = (tokenizer.scan)(&mut cursor, stack)?;
        let observed = lookahead.get().saturating_sub(from);
        // A callback can catch errors, but cannot reset the parser's budget.
        work.spend(start, 0)?;
        let Some(token) = token else {
            return Ok((None, observed));
        };
        parser.check_token_term(token.term, start)?;
        let end = self
            .input
            .byte_end(token.end)
            .filter(|&end| end >= start)
            .ok_or(ParseError::InvalidTokenEnd {
                offset: start,
                end: token.end,
            })?;
        // A callback can supply an end without reading those units. Even an
        // unselected token depends on those bytes for validity and specialization.
        ReadExtent::observe(self.reads.as_ref(), end, false);
        Ok((
            Some(Token {
                term: token.term,
                end,
            }),
            observed,
        ))
    }
}

impl Parser {
    fn check_token_term(&self, term: u16, offset: usize) -> Result<(), ParseError> {
        if !self.is_token_term(term) {
            return Err(ParseError::InvalidTokenTerm { offset, term });
        }
        Ok(())
    }

    fn specialize(
        &self,
        source: &str,
        start: usize,
        token: Token,
        stack: &StackView<'_>,
        work: &Work,
    ) -> Result<(Token, Option<u16>), ParseError> {
        for specializer in &self.specializers {
            work.spend(start, 1)?;
            if specializer.term != token.term {
                continue;
            }
            let text = &source[start..token.end];
            work.spend(start, text.len())?;
            let result = specializer.get(text, stack)?;
            work.spend(start, 0)?;
            match result {
                Some(Specialization::Replace(term)) => {
                    self.check_token_term(term, start)?;
                    return Ok((Token { term, ..token }, None));
                }
                Some(Specialization::Extend(term)) => {
                    self.check_token_term(term, start)?;
                    return Ok((token, Some(term)));
                }
                None => {}
            }
        }
        Ok((token, None))
    }
}

struct Scan<'a> {
    source: &'a str,
    start: usize,
    work: &'a Work,
    reads: Option<&'a Cell<ReadExtent>>,
}

impl Scan<'_> {
    fn local(&self, group: &LocalTokenGroup) -> Result<(Option<Token>, usize), ParseError> {
        let scan = |at| self.dfa(at, &group.data, 1, &group.precedence);
        let first = scan(self.start)?;
        let Some(term) = group.else_token.filter(|_| first.0.is_none()) else {
            return Ok(first);
        };
        // Lezer resets token lookahead before accepting @else. Checkpoint
        // reads are cumulative and still include all the failed probes.
        // Probe each later scalar boundary, not the end of the failed match.
        // A new token can overlap the characters read by a failed probe.
        // Do not probe EOF again as part of @else: it consumes only input.
        let suffix = &self.source[self.start..];
        for (offset, ch) in suffix.char_indices() {
            let at = self.start + offset;
            self.work.spend(self.start, 1)?;
            if offset != 0 && scan(at)?.0.is_some() {
                return Ok((Some(Token { term, end: at }), 1));
            }
            // The fallback consumes this scalar even if the DFA had no edges.
            ReadExtent::observe(self.reads, at + ch.len_utf8(), false);
        }
        let end = self.source.len();
        ReadExtent::observe(self.reads, end, true);
        Ok(((end > self.start).then_some(Token { term, end }), 1))
    }

    fn dfa(
        &self,
        at: usize,
        data: &[u16],
        mask: u16,
        precedence: &[u16],
    ) -> Result<(Option<Token>, usize), ParseError> {
        let rank = |term| precedence.get(usize::from(term)).copied().unwrap_or(END);
        let mut state = 0;
        let mut token: Option<Token> = None;
        let mut chars = self.source[at..].chars();
        let mut end = at;
        let mut low_surrogate = None;
        let mut eof_steps = 0;
        // InputStream.reset observes the first unit. Each successful advance
        // can observe one more, even if the target row is outside this group.
        let mut observed = 1;
        loop {
            let row = Row::read(data, state).expect("validated DFA row");
            if row.mask & mask == 0 {
                break;
            }
            self.work.spend(self.start, 1)?;
            // A DFA sees UTF-16 code units. A byte range cannot end between
            // the high and low surrogates of a Rust `char`.
            if low_surrogate.is_none() {
                for &[term, groups] in row.accepts {
                    self.work.spend(self.start, 1)?;
                    if groups & mask != 0
                        && token.is_none_or(|prev| {
                            let old_rank = rank(prev.term);
                            let new_rank = rank(term);
                            // Unlisted terms come first. A term can extend its match.
                            term == prev.term
                                || old_rank == END
                                || new_rank == END
                                || new_rank < old_rank
                        })
                    {
                        token = Some(Token { term, end });
                        break;
                    }
                }
            }
            let unit = if let Some(low) = low_surrogate.take() {
                Some(low)
            } else if let Some(ch) = chars.next() {
                let mut units = [0; 2];
                let units = ch.encode_utf16(&mut units);
                low_surrogate = units.get(1).copied();
                end += ch.len_utf8();
                Some(units[0])
            } else {
                // EOF edges do not advance the input. A finite DFA cannot
                // follow this many consecutive EOF edges without a cycle.
                eof_steps += 1;
                if eof_steps > data.len() / 3 {
                    return Err(ParseError::LimitExceeded { offset: self.start });
                }
                None
            };
            let Some(next) = row.edge(unit) else { break };
            observed += usize::from(end < self.source.len() || low_surrogate.is_some());
            state = next;
        }
        // Include units that did not match an edge, even if the chosen
        // token ended much earlier. A high surrogate covers the full char.
        ReadExtent::observe(self.reads, end, eof_steps != 0);
        Ok((token, observed))
    }
}
