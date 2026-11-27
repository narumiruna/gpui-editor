//! SQL isolation properties from `@codemirror/lang-sql` 6.10.0. See the MIT
//! notice in `docs/licenses/codemirror-lang-sql.txt`.

use std::{ops::Range, sync::Arc};

use crate::{
    EditError, RangeSet,
    language::bidi::isolate_marks,
    view::{MarkDecoration, Viewport},
};

pub use crate::language::BidiIsolateOptions as SqlBidiIsolateOptions;

use super::{Node, SqlDocument, SqlNodeKind};

/// The value of `NodeProp.isolate` on a SQL node that isolates its direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SqlBidiIsolate {
    /// Use the first strong character, or LTR if there is none.
    Auto,
}

impl SqlNodeKind {
    /// Read the pinned grammar's `NodeProp.isolate` value for this kind.
    pub const fn bidi_isolate(self) -> Option<SqlBidiIsolate> {
        match self {
            Self::LineComment | Self::BlockComment | Self::String | Self::QuotedIdentifier => {
                Some(SqlBidiIsolate::Auto)
            }
            _ => None,
        }
    }
}

pub(super) fn isolate_ranges(nodes: &[Node]) -> Vec<Range<usize>> {
    // The tagged SQL terms are nonempty leaves in source order, even in local recovery.
    nodes
        .iter()
        .filter(|node| node.kind.bidi_isolate().is_some())
        .map(|node| node.range.clone())
        .collect()
}

impl SqlDocument {
    /// Build `cm-iso` marks for the isolated nodes that touch the visible ranges.
    /// These ranges select nodes but do not trim their marks. With default options,
    /// only the ranges selected by the pinned RTL line optimization contribute.
    ///
    /// Submit the returned set to [`crate::view::bidi::BIDI_ISOLATED_RANGES`] at
    /// lowest precedence on the same document snapshot. Use the same marks as
    /// outer decorations when drawing the text. The host supplies the viewport,
    /// applies the isolation to rendering, and refreshes the marks after changes
    /// to the document, viewport, or direction. This method installs no extension.
    ///
    /// Return [`EditError::InvalidRange`] for viewport endpoints that are outside
    /// this document or not UTF-8 boundaries.
    pub fn bidi_isolate_marks(
        &self,
        viewport: &Viewport,
        options: SqlBidiIsolateOptions,
    ) -> Result<Arc<RangeSet<MarkDecoration>>, EditError> {
        isolate_marks(
            &self.text,
            &self.text,
            &self.isolate_ranges,
            viewport,
            options,
        )
    }
}
