use crate::{Text, language_data::NativeLanguage};

use super::{
    super::lezer::{LrParseJob, NodeTypes, ParseError},
    lr::{PARSER, stock_node_types},
};

pub use super::super::lezer::{LrParseOutput as LezerParseOutput, LrParseStep as LezerParseStep};

/// An owned, bounded Lezer grammar parse. It does not publish output,
/// retain edit checkpoints, or fall back to another parser.
pub struct LezerParseJob(pub(crate) LrParseJob);

impl LezerParseJob {
    /// Project LF, CRLF, and CR to one logical line break. Setup can scan and
    /// copy the entire text. Output ranges use the original bytes.
    pub fn new(text: Text) -> Self {
        Self::with_node_types(text, stock_node_types().clone())
    }

    /// Parse the original bytes without editor line normalization.
    pub fn new_raw(text: Text) -> Self {
        Self(LrParseJob::new(
            text,
            &PARSER,
            NativeLanguage::LezerGrammar,
            stock_node_types().clone(),
            true,
        ))
    }

    pub(crate) fn with_config(text: Text, node_types: NodeTypes, strict: bool) -> Self {
        if strict {
            return Self(LrParseJob::new_strict_lezer(text, &PARSER, node_types));
        }
        Self::with_node_types(text, node_types)
    }

    fn with_node_types(text: Text, node_types: NodeTypes) -> Self {
        Self(LrParseJob::new(
            text,
            &PARSER,
            NativeLanguage::LezerGrammar,
            node_types,
            false,
        ))
    }

    /// Run at most `rounds` live position rounds through `upto`, in original
    /// UTF-8 bytes. Clamp to EOF and round up inside a scalar or projected
    /// CRLF. One round can scan a whole token.
    ///
    /// A zero budget can still build output for a reached cutoff. Staging
    /// uses a separate budget and does not change the live frontier. A live
    /// error ends the job.
    ///
    /// # Panics
    /// Panics if the adjusted cutoff is lower than a previous request.
    pub fn advance(&mut self, upto: usize, rounds: usize) -> Result<LezerParseStep, ParseError> {
        self.0.advance(upto, rounds)
    }
}
