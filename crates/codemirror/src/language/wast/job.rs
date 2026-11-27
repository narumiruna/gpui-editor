use crate::Text;

use super::{
    super::{
        lezer::{ParseError, SyntaxTree, session::RecoveringSession},
        line_projection::{NormalizedInput, NormalizedOffsets, splits_crlf},
    },
    WastNodeTypes,
    parser::PARSER,
};

#[cfg(test)]
mod tests;

/// An owned, cold WAST recovery parse of one editor text snapshot.
///
/// This job does not publish output or supply edit checkpoints. It returns
/// native recovery errors without the synchronous WAST parser's strict-first
/// path or local fallback. It does not replace the configured language API.
pub struct WastParseJob {
    text: Text,
    offsets: NormalizedOffsets,
    session: RecoveringSession,
    node_types: WastNodeTypes,
}

/// Result and work counts for one call to [`WastParseJob::advance`].
#[derive(Debug)]
pub struct WastParseStep {
    /// Number of live position rounds run in this call.
    pub rounds: usize,
    /// Work units spent on output, separate from live rounds. Includes staged
    /// branch selection, fork and closure, traversal and copying, or EOF
    /// finalization; also counts one unit per projected byte endpoint.
    pub output_work: usize,
    /// An owned result for the caller to keep or publish. No result means
    /// more rounds are needed, or the job has already ended.
    pub output: Option<WastParseOutput>,
}

/// A WAST tree with node-type rows and original UTF-8 ranges. The tree has
/// no incremental edit state.
#[derive(Debug)]
pub struct WastParseOutput {
    pub tree: SyntaxTree,
    /// Prefix coverage to publish. A token can take the root past this byte.
    pub parsed_to: usize,
    /// True only after the live parse has finished at real EOF.
    pub complete: bool,
}

impl WastParseJob {
    /// Normalize and index the input once, before the first work call. This
    /// setup can scan and copy the entire document.
    pub fn new(text: Text) -> Self {
        Self::with_node_types(text, WastNodeTypes::stock())
    }

    pub(crate) fn with_node_types(text: Text, node_types: WastNodeTypes) -> Self {
        let NormalizedInput {
            text: normalized,
            offsets,
        } = NormalizedInput::new(text.as_str());
        let session = RecoveringSession::new_wast(&PARSER, normalized.into_owned());
        Self {
            text,
            offsets,
            session,
            node_types,
        }
    }

    /// Add normalized UTF-16 units to a raw byte position and map back to
    /// raw bytes. Return `None` at or beyond EOF or after the session ends.
    pub(crate) fn cutoff_after(&self, start: usize, units: usize) -> Option<usize> {
        let start = self.offsets.normalized_byte(self.round_up(start));
        self.session
            .cutoff_after(start, units)
            .map(|end| self.offsets.original_byte(end))
    }

    fn round_up(&self, upto: usize) -> usize {
        let mut upto = upto.min(self.text.len());
        while !self.text.is_char_boundary(upto) {
            upto += 1;
        }
        upto + usize::from(splits_crlf(self.text.as_str(), upto))
    }

    /// Parse through `upto` in at most `rounds` live position rounds. Clamp
    /// `upto` to EOF and round up within a UTF-8 scalar or CRLF pair. Tokens
    /// can cross the cutoff. A round can scan a long token under the native
    /// work limit.
    ///
    /// A zero round budget can still materialize a reached cutoff. Staging
    /// uses a separate copy of the remaining work budget and leaves the live
    /// frontier unchanged, even on a staging error. An error from a live
    /// round or EOF finalization ends the job. Error offsets use raw bytes.
    pub fn advance(&mut self, upto: usize, rounds: usize) -> Result<WastParseStep, ParseError> {
        let upto = self.round_up(upto);
        let original_byte = |at| self.offsets.original_byte(at);
        let advance = self
            .session
            .advance(self.offsets.normalized_byte(upto), rounds)
            .map_err(|error| error.map_offset(original_byte))?;
        let mut output_work = advance.output_work;
        let output = advance.output.map(|(mut tree, complete)| {
            output_work += tree.project_glr_ranges(original_byte);
            let parsed_to = if complete { self.text.len() } else { upto };
            WastParseOutput {
                tree: tree.with_metadata(self.node_types.clone()),
                parsed_to,
                complete,
            }
        });
        Ok(WastParseStep {
            rounds: advance.rounds,
            output_work,
            output,
        })
    }
}
