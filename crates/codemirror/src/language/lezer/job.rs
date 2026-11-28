use crate::{Text, language_data::NativeLanguage};

use super::{NodeTypes, ParseError, Parser, SyntaxTree, session::RecoveringSession};
use crate::language::line_projection::{NormalizedInput, NormalizedOffsets, splits_crlf};

/// Work and output for one call to an owned LR parse job.
#[derive(Debug)]
pub struct LrParseStep {
    /// Live position rounds used in this call.
    pub rounds: usize,
    /// Work to build and project the output, separate from live rounds.
    pub output_work: usize,
    /// No tree means more rounds are necessary or the job has ended.
    pub output: Option<LrParseOutput>,
}

/// An owned tree with configured grammar rows and original UTF-8 byte ranges.
#[derive(Debug)]
pub struct LrParseOutput {
    pub tree: SyntaxTree,
    /// The requested prefix coverage. A token can take the root past it.
    pub parsed_to: usize,
    /// True only after the live parse finishes at real EOF.
    pub complete: bool,
    language: NativeLanguage,
}

impl LrParseOutput {
    pub fn language(&self) -> NativeLanguage {
        self.language
    }

    pub(crate) fn covers(&self, at: usize, len: usize) -> bool {
        self.parsed_to >= at && (at < len || self.complete)
    }
}

pub(crate) struct LrParseJob {
    text: Text,
    offsets: Option<NormalizedOffsets>,
    session: RecoveringSession,
    node_types: NodeTypes,
    language: NativeLanguage,
    cutoff: usize,
    in_flight: bool,
}

impl LrParseJob {
    pub(in crate::language) fn new(
        text: Text,
        parser: &'static Parser,
        language: NativeLanguage,
        node_types: NodeTypes,
        raw: bool,
    ) -> Self {
        let session = match language {
            NativeLanguage::Json => RecoveringSession::new_json,
            NativeLanguage::LezerGrammar => RecoveringSession::new_lezer,
            _ => unreachable!("only JSON and Lezer grammar use the LR viewport job"),
        };
        Self::with_session(text, language, node_types, raw, |input| {
            session(parser, input)
        })
    }

    pub(in crate::language) fn new_strict_lezer(
        text: Text,
        parser: &'static Parser,
        node_types: NodeTypes,
    ) -> Self {
        Self::with_session(
            text,
            NativeLanguage::LezerGrammar,
            node_types,
            false,
            |input| RecoveringSession::new_strict_lezer(parser, input),
        )
    }

    fn with_session(
        text: Text,
        language: NativeLanguage,
        node_types: NodeTypes,
        raw: bool,
        session: impl FnOnce(String) -> RecoveringSession,
    ) -> Self {
        let (input, offsets) = if raw {
            (text.as_str().to_owned(), None)
        } else {
            let projected = NormalizedInput::new(text.as_str());
            (projected.text.into_owned(), Some(projected.offsets))
        };
        Self {
            text,
            offsets,
            session: session(input),
            node_types,
            language,
            cutoff: 0,
            in_flight: false,
        }
    }

    pub(crate) fn cutoff_after(&self, start: usize, units: usize) -> Option<usize> {
        let offsets = self.offsets.as_ref()?;
        let start = self.adjusted_cutoff(start);
        self.session
            .cutoff_after(offsets.normalized_byte(start), units)
            .map(|end| offsets.original_byte(end))
    }

    /// Raw coverage of unfinished work, excluding token overrun from a stage.
    pub(crate) fn in_flight_progress(&self) -> Option<usize> {
        if !self.in_flight {
            return None;
        }
        let position = self.session.position()?;
        Some(
            self.offsets
                .as_ref()
                .map_or(position, |offsets| offsets.original_byte(position)),
        )
    }

    pub(crate) fn advance(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<LrParseStep, ParseError> {
        let cutoff = self.adjusted_cutoff(upto);
        assert!(
            cutoff >= self.cutoff,
            "cannot lower the cutoff of an LR job"
        );
        self.cutoff = cutoff;
        self.advance_at(cutoff, rounds)
    }

    pub(crate) fn advance_viewport(
        &mut self,
        upto: usize,
        rounds: usize,
    ) -> Result<LrParseStep, ParseError> {
        self.advance_at(self.adjusted_cutoff(upto), rounds)
    }

    fn adjusted_cutoff(&self, upto: usize) -> usize {
        let mut cutoff = upto.min(self.text.len());
        while !self.text.is_char_boundary(cutoff) {
            cutoff += 1;
        }
        if self.offsets.is_some() && splits_crlf(self.text.as_str(), cutoff) {
            cutoff += 1;
        }
        cutoff
    }

    fn advance_at(&mut self, cutoff: usize, rounds: usize) -> Result<LrParseStep, ParseError> {
        let offsets = self.offsets.as_ref();
        let stop = offsets.map_or(cutoff, |offsets| offsets.normalized_byte(cutoff));
        let original_byte = |at| offsets.map_or(at, |offsets| offsets.original_byte(at));
        let advance = self
            .session
            .advance(stop, rounds)
            .map_err(|error| error.map_offset(original_byte))?;
        if advance.output.is_some() {
            self.in_flight = false;
        } else if advance.rounds != 0 {
            self.in_flight = true;
        }
        let mut output_work = advance.output_work;
        let output = advance.output.map(|(mut tree, complete)| {
            if offsets.is_some() {
                output_work += tree.project_glr_ranges(original_byte);
            }
            LrParseOutput {
                tree: tree.with_node_types(self.language, self.node_types.clone()),
                parsed_to: if complete { self.text.len() } else { cutoff },
                complete,
                language: self.language,
            }
        });
        Ok(LrParseStep {
            rounds: advance.rounds,
            output_work,
            output,
        })
    }
}
