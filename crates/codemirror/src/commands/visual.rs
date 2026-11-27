//! Headless counterparts to selected `@codemirror/commands` 6.11.2 view commands.
//!
//! The caller supplies geometry for the current document snapshot. This module
//! does not measure text, find wraps, resolve bidirectional order, or scroll a
//! viewport. See `docs/licenses/codemirror-commands.txt` and
//! `docs/licenses/codemirror-view.txt` for the upstream MIT notices.

use std::{cmp::Ordering, collections::HashSet, ops::Range};

use crate::{
    Assoc, CharCategory, EditError, EditorSelection, EditorState, SelectionRange, Transaction,
    language::lezer::{SyntaxNode, SyntaxTree},
    view::{SCROLL_INTO_VIEW, ScrollStrategy, ScrollTarget},
};

use super::{CommandResult, caret, include_crlf, replace_range, select, space, unchanged};

/// Backward means up or toward the visual start. Forward means down or toward
/// the visual end. A horizontal step can move against document order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VisualDirection {
    Backward,
    Forward,
}

use VisualDirection::{Backward, Forward};

impl VisualDirection {
    fn edge(self, range: Range<usize>) -> usize {
        match self {
            Backward => range.start,
            Forward => range.end,
        }
    }
}

/// Base direction of text at a position, not the direction of a bidi run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextDirection {
    LeftToRight,
    RightToLeft,
}

/// The old caret and effective edges of the editor's own vertical scroller.
/// All coordinates use the same origin and units. The scroll edges already
/// exclude the host's top and bottom scroll margins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageScrollGeometry {
    pub scroll_top: f64,
    pub scroll_bottom: f64,
    pub caret_top: f64,
    pub caret_bottom: f64,
}

/// The requested vertical distance. The host resolves it against its layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerticalDistance {
    Line,
    Page,
}

/// A selection head in UTF-8 bytes, with optional visual affinity and goal column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisualCursor {
    pub head: usize,
    /// The side of the head used when a position has two visual locations.
    pub assoc: Option<Assoc>,
    /// The bidi level at the head, when reported by the host.
    pub bidi_level: Option<u8>,
    /// The retained horizontal coordinate in host units, not a byte offset.
    pub goal_column: Option<f64>,
}

impl VisualCursor {
    pub const fn new(head: usize) -> Self {
        Self {
            head,
            assoc: None,
            bidi_level: None,
            goal_column: None,
        }
    }

    pub fn from_range(range: SelectionRange) -> Self {
        Self {
            head: range.head(),
            assoc: range.assoc(),
            bidi_level: range.bidi_level(),
            goal_column: range.goal_column(),
        }
    }

    pub fn as_caret(self) -> SelectionRange {
        EditorSelection::cursor(self.head, self.assoc, self.bidi_level, self.goal_column)
    }

    fn directed(mut self, range: SelectionRange) -> Self {
        self.bidi_level = self.bidi_level.or(range.bidi_level());
        self.assoc = self.assoc.or(range.assoc());
        if self.assoc.is_none() {
            self.assoc = match range.head().cmp(&range.anchor()) {
                Ordering::Less => Some(Assoc::After),
                Ordering::Greater => Some(Assoc::Before),
                Ordering::Equal => None,
            };
        }
        self
    }

    fn has_valid_goal(self) -> bool {
        self.goal_column.is_none_or(f64::is_finite)
    }
}

/// Raw layout queries for one editor snapshot. Do not apply atomic ranges in
/// these queries. This module applies the command-specific atomic range rules.
/// All positions must be UTF-8 byte boundaries in the snapshot.
pub trait VisualGeometry {
    /// Move from the given visual side of the head. Use the retained goal if
    /// present, and return the goal to use on a subsequent vertical move.
    /// `Line` means one displayed row. `Page` uses the host's effective page
    /// distance, including any viewport margins. Return an unchanged head if
    /// there is no further move in this direction. If movement stops at a
    /// document edge, the host can clear the goal.
    fn move_vertical(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
        distance: VerticalDistance,
    ) -> VisualCursor;

    /// Find the start or end of the displayed row at this head and affinity.
    fn line_boundary(&self, start: VisualCursor, direction: VisualDirection) -> VisualCursor;

    /// Base text direction at `head`. Physical aliases query the main head
    /// once and use that basis for all ranges. `None` makes them unhandled.
    fn text_direction_at(&self, _head: usize) -> Option<TextDirection> {
        None
    }

    /// Walk grapheme clusters in visual order, respecting bidi runs and hard
    /// line breaks. `None` means this query is unsupported. With no predicate,
    /// stop after the first cluster. With a predicate, call it for each cluster
    /// (use `"\n"` for a hard line break) and stop before the first rejection.
    /// Call it only for nonempty clusters. The command always accepts the first.
    /// Return the input cursor if there is no step. Do not skip atomic ranges.
    fn move_by_char(
        &self,
        _start: VisualCursor,
        _direction: VisualDirection,
        _accept: Option<&mut dyn FnMut(&str) -> bool>,
    ) -> Option<VisualCursor> {
        None
    }

    /// The line block that contains `head`. It can differ from a hard line
    /// when block decorations hide or join text. `None` means unsupported.
    fn line_block(&self, _head: usize) -> Option<Range<usize>> {
        None
    }

    /// Like `line_boundary`, but ignore wraps. Use `start.assoc` to resolve
    /// positions shared by blocks. `None` means unsupported.
    fn unwrapped_line_boundary(
        &self,
        _start: VisualCursor,
        _direction: VisualDirection,
    ) -> Option<VisualCursor> {
        None
    }

    /// Return coordinates for the old main head (with the host's default
    /// affinity for position-only queries). Return `None` for a window or
    /// parent scroller, or if the coordinates are not available. The command
    /// requests start alignment only when the old caret is strictly inside
    /// both scroll edges. Otherwise it requests ordinary selection scrolling.
    fn page_scroll_geometry(&self, _head: usize) -> Option<PageScrollGeometry> {
        None
    }

    /// Ordered sets of atomic ranges. Preserve the visit order within each set.
    /// Only the interior of a range is atomic. Sets may overlap.
    fn atomic_ranges(&self) -> &[Vec<Range<usize>>] {
        &[]
    }
}

/// A syntax tree for the current document and optional grammar bracket data.
/// A plain `SyntaxTree` supports traversal without marked-bracket jumps.
/// Supply the bracket queries if the grammar marks opening or closing nodes.
/// A provider can also use them for grammar-specific bracket handles.
pub trait VisualSyntax {
    fn tree(&self) -> &SyntaxTree;

    /// Whether the node has `closedBy` (forward) or `openedBy` (backward).
    fn is_bracket(&self, _node: SyntaxNode<'_>, _direction: VisualDirection) -> bool {
        false
    }

    /// Byte range of the matched partner's handle. Return `None` if unmatched.
    fn matching_bracket(
        &self,
        _node: SyntaxNode<'_>,
        _direction: VisualDirection,
    ) -> Option<Range<usize>> {
        None
    }
}

impl VisualSyntax for SyntaxTree {
    fn tree(&self) -> &SyntaxTree {
        self
    }
}

/// Host-owned visual information for the current selection.
/// Reset it after an unrelated selection or text change.
#[derive(Clone, Debug, PartialEq)]
pub struct VisualSelection {
    selection: EditorSelection,
    cursors: Vec<VisualCursor>,
}

impl VisualSelection {
    /// Start with no retained columns and use the default affinity of each range.
    pub fn from_selection(selection: &EditorSelection) -> Self {
        let cursors = selection
            .ranges()
            .iter()
            .map(|&range| VisualCursor::new(range.head()).directed(range))
            .collect();
        Self {
            selection: selection.clone(),
            cursors,
        }
    }

    /// Supply one visual cursor for each range in normalized selection order.
    pub fn new(
        selection: &EditorSelection,
        mut cursors: Vec<VisualCursor>,
    ) -> Result<Self, EditError> {
        if cursors.len() != selection.ranges().len() {
            return Err(EditError::InvalidSelection);
        }
        for (&range, cursor) in selection.ranges().iter().zip(&mut cursors) {
            if cursor.head != range.head() || !cursor.has_valid_goal() {
                return Err(EditError::InvalidSelection);
            }
            *cursor = cursor.directed(range);
        }
        Ok(Self {
            selection: selection.clone(),
            cursors,
        })
    }

    pub fn cursors(&self) -> &[VisualCursor] {
        &self.cursors
    }

    fn normalize(
        mut ranges: Vec<(SelectionRange, VisualCursor)>,
        main: usize,
    ) -> Result<Self, EditError> {
        let selection =
            EditorSelection::new(ranges.iter().map(|(range, _)| *range).collect(), main)?;
        ranges.sort_by_key(|(range, _)| range.range().start);
        let mut cursors = Vec::with_capacity(selection.ranges().len());
        let mut end = None;
        // Follow EditorSelection's merge rule to retain metadata only on
        // unchanged ranges. A merged range starts with default metadata.
        for (range, cursor) in ranges {
            let span = range.range();
            if let Some(previous) = end
                && (span.start < previous || (span.is_empty() && span.start == previous))
            {
                end = Some(previous.max(span.end));
                let merged = selection.ranges()[cursors.len() - 1];
                *cursors.last_mut().expect("there is a previous cursor") =
                    VisualCursor::new(merged.head()).directed(merged);
            } else {
                end = Some(span.end);
                cursors.push(cursor.directed(range));
            }
        }
        Ok(Self { selection, cursors })
    }

    fn eq_for_motion(&self, other: &Self, include_assoc: bool) -> bool {
        self.selection == other.selection
            && self
                .cursors
                .iter()
                .zip(&other.cursors)
                .zip(self.selection.ranges())
                .all(|((a, b), range)| {
                    a.goal_column == b.goal_column
                        && (!include_assoc || !range.range().is_empty() || a.assoc == b.assoc)
                })
    }
}

/// A selection transaction and its proposed host-owned visual information.
#[derive(Debug)]
pub struct VisualUpdate {
    pub transaction: Transaction,
    /// Retain this only if the transaction is accepted with its proposed
    /// selection unchanged. If a filter changes it, reset the metadata.
    pub selection: VisualSelection,
}

/// `None` means no update was needed or the host lacks a required query.
/// A missing query for one range leaves the complete selection unchanged.
pub type VisualCommandResult = Result<Option<VisualUpdate>, EditError>;

struct Geometry<'a> {
    state: &'a EditorState,
    raw: &'a dyn VisualGeometry,
    atoms: &'a [Vec<Range<usize>>],
}

impl<'a> Geometry<'a> {
    fn new(
        state: &'a EditorState,
        raw: &'a dyn VisualGeometry,
        selection: &VisualSelection,
    ) -> Result<Self, EditError> {
        if &selection.selection != state.selection() {
            return Err(EditError::InvalidSelection);
        }
        let atoms = raw.atomic_ranges();
        for range in atoms.iter().flatten() {
            if range.start > range.end
                || !state.text().is_char_boundary(range.start)
                || !state.text().is_char_boundary(range.end)
            {
                return Err(EditError::InvalidRange);
            }
        }
        Ok(Self { state, raw, atoms })
    }

    fn check_cursor(&self, cursor: VisualCursor) -> Result<VisualCursor, EditError> {
        if !cursor.has_valid_goal() || !self.state.text().is_char_boundary(cursor.head) {
            return Err(EditError::InvalidSelection);
        }
        Ok(cursor)
    }

    fn line_boundary(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
    ) -> Result<VisualCursor, EditError> {
        self.check_cursor(self.raw.line_boundary(start, direction))
    }

    fn move_vertical(
        &self,
        start: VisualCursor,
        direction: VisualDirection,
        distance: VerticalDistance,
    ) -> Result<VisualCursor, EditError> {
        let target = self.raw.move_vertical(start, direction, distance);
        self.skip_motion_atoms(start, target)
    }

    fn skip_motion_atoms(
        &self,
        start: VisualCursor,
        target: VisualCursor,
    ) -> Result<VisualCursor, EditError> {
        let target = self.check_cursor(target)?;
        let direction = if start.head > target.head {
            Backward
        } else {
            Forward
        };
        let mut head = target.head;
        loop {
            let next = self.skip_atoms(head, direction, 1);
            if next == head {
                break;
            }
            head = next;
        }
        if head == target.head {
            Ok(target)
        } else {
            // An atomic jump gives a fresh cursor on the near side of the edge.
            let assoc = if head < target.head {
                Assoc::After
            } else {
                Assoc::Before
            };
            Ok(VisualCursor {
                assoc: Some(assoc),
                ..VisualCursor::new(head)
            })
        }
    }

    fn line_block(&self, head: usize) -> Result<Option<Range<usize>>, EditError> {
        self.raw
            .line_block(head)
            .map(|block| {
                if block.start <= head
                    && head <= block.end
                    && self.state.text().is_valid_range(block.clone())
                {
                    Ok(block)
                } else {
                    Err(EditError::InvalidRange)
                }
            })
            .transpose()
    }

    // Search around the original position for each set, not around each moved
    // position. Navigation repeats with a one-byte margin; deletion visits
    // each set just once and only looks at ranges that touch the exact point.
    fn skip_atoms(&self, mut head: usize, direction: VisualDirection, margin: usize) -> usize {
        for set in self.atoms {
            let from = head.saturating_sub(margin);
            let to = head.saturating_add(margin);
            for range in set {
                if range.start <= to && from <= range.end && range.start < head && head < range.end
                {
                    head = match direction {
                        Backward => range.start,
                        Forward => range.end,
                    };
                }
            }
        }
        head
    }
}

fn selection_update(selection: VisualSelection) -> VisualUpdate {
    VisualUpdate {
        transaction: select(selection.selection.clone(), true),
        selection,
    }
}

fn move_selection(
    selection: &VisualSelection,
    extend: bool,
    compare_assoc: bool,
    mut target: impl FnMut(SelectionRange, VisualCursor) -> Result<Option<VisualCursor>, EditError>,
) -> Result<Option<VisualSelection>, EditError> {
    let mut ranges = Vec::with_capacity(selection.cursors.len());
    for (&range, &cursor) in selection.selection.ranges().iter().zip(&selection.cursors) {
        let Some(target) = target(range, cursor)? else {
            return Ok(None);
        };
        let range = if extend {
            SelectionRange::new(range.anchor(), target.head)
        } else {
            caret(target.head)
        };
        ranges.push((range, target));
    }
    let next = VisualSelection::normalize(ranges, selection.selection.main_index())?;
    Ok((!next.eq_for_motion(selection, compare_assoc)).then_some(next))
}

fn vertical(
    state: &EditorState,
    raw: &dyn VisualGeometry,
    selection: &VisualSelection,
    direction: VisualDirection,
    distance: VerticalDistance,
    extend: bool,
) -> VisualCommandResult {
    let geometry = Geometry::new(state, raw, selection)?;
    let compare_assoc = !extend && distance == VerticalDistance::Line;
    let next = move_selection(selection, extend, compare_assoc, |range, cursor| {
        let target = if !extend && !range.range().is_empty() {
            VisualCursor::new(direction.edge(range.range()))
        } else {
            let moved = geometry.move_vertical(cursor, direction, distance)?;
            if compare_assoc && moved.head == cursor.head {
                geometry.line_boundary(cursor, direction)?
            } else {
                moved
            }
        };
        Ok(Some(target))
    })?;
    let Some(next) = next else { return Ok(None) };
    let head = next.selection.main().head();
    let mut update = selection_update(next);
    if !extend
        && distance == VerticalDistance::Page
        && let Some(page) = raw.page_scroll_geometry(state.selection().main().head())
    {
        let margin = page.caret_top - page.scroll_top;
        if ![
            page.scroll_top,
            page.scroll_bottom,
            page.caret_top,
            page.caret_bottom,
            margin,
        ]
        .into_iter()
        .all(f64::is_finite)
            || page.caret_top > page.caret_bottom
        {
            return Err(EditError::InvalidRange);
        }
        if page.scroll_top < page.caret_top && page.caret_bottom < page.scroll_bottom {
            update.transaction =
                update
                    .transaction
                    .with_effects([SCROLL_INTO_VIEW.of(ScrollTarget {
                        y: ScrollStrategy::Start,
                        y_margin: margin,
                        ..ScrollTarget::at(head)
                    })]);
        }
    }
    Ok(Some(update))
}

/// Move up one displayed row, or to the row start when no vertical step moves the head.
pub fn cursor_line_up(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    vertical(
        state,
        geometry,
        selection,
        Backward,
        VerticalDistance::Line,
        false,
    )
}

/// Move down one displayed row, or to the row end when no vertical step moves the head.
pub fn cursor_line_down(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    vertical(
        state,
        geometry,
        selection,
        Forward,
        VerticalDistance::Line,
        false,
    )
}

/// Extend each range up by one displayed row.
pub fn select_line_up(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    vertical(
        state,
        geometry,
        selection,
        Backward,
        VerticalDistance::Line,
        true,
    )
}

/// Extend each range down by one displayed row.
pub fn select_line_down(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    vertical(
        state,
        geometry,
        selection,
        Forward,
        VerticalDistance::Line,
        true,
    )
}

/// Move up by one host-defined page, or collapse a nonempty range to its start.
pub fn cursor_page_up(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    vertical(
        state,
        geometry,
        selection,
        Backward,
        VerticalDistance::Page,
        false,
    )
}

/// Move down by one host-defined page, or collapse a nonempty range to its end.
pub fn cursor_page_down(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    vertical(
        state,
        geometry,
        selection,
        Forward,
        VerticalDistance::Page,
        false,
    )
}

/// Extend each range up by one host-defined page, without moving its anchor.
pub fn select_page_up(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    vertical(
        state,
        geometry,
        selection,
        Backward,
        VerticalDistance::Page,
        true,
    )
}

/// Extend each range down by one host-defined page, without moving its anchor.
pub fn select_page_down(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    vertical(
        state,
        geometry,
        selection,
        Forward,
        VerticalDistance::Page,
        true,
    )
}

#[derive(Clone, Copy)]
enum HorizontalDirection {
    Left,
    Right,
    Visual(VisualDirection),
}

use HorizontalDirection::{Left, Right, Visual};

enum HorizontalUnit<'a> {
    Char,
    Group,
    GroupWindows,
    Boundary,
    Line,
    Syntax(Option<&'a dyn VisualSyntax>),
}

fn horizontal(
    state: &EditorState,
    raw: &dyn VisualGeometry,
    selection: &VisualSelection,
    movement: HorizontalDirection,
    unit: HorizontalUnit<'_>,
    extend: bool,
) -> VisualCommandResult {
    use HorizontalUnit::*;

    let geometry = Geometry::new(state, raw, selection)?;
    let direction = match movement {
        Visual(direction) => direction,
        Left | Right => {
            let Some(basis) = raw.text_direction_at(state.selection().main().head()) else {
                return Ok(None);
            };
            match (movement, basis) {
                (Right, TextDirection::LeftToRight) | (Left, TextDirection::RightToLeft) => Forward,
                _ => Backward,
            }
        }
    };
    let next = move_selection(selection, extend, !extend, |range, cursor| {
        let target = match unit {
            Char | Group | GroupWindows => {
                if !extend && !range.range().is_empty() {
                    Some(VisualCursor::new(direction.edge(range.range())))
                } else {
                    let target = match unit {
                        Char => raw.move_by_char(cursor, direction, None),
                        _ => move_by_group(
                            state,
                            raw,
                            cursor,
                            direction,
                            matches!(unit, GroupWindows),
                        ),
                    };
                    target
                        .map(|target| geometry.skip_motion_atoms(cursor, target))
                        .transpose()?
                }
            }
            Boundary => move_to_boundary(&geometry, cursor, direction)?,
            Line if extend => geometry
                .line_block(cursor.head)?
                .map(|block| VisualCursor::new(direction.edge(block))),
            Line => raw.unwrapped_line_boundary(cursor, direction),
            Syntax(syntax) => Some(move_by_syntax(state, syntax, cursor, direction)?),
        };
        target
            .map(|target| geometry.check_cursor(target))
            .transpose()
    })?;
    Ok(next.map(selection_update))
}

fn move_by_group(
    state: &EditorState,
    raw: &dyn VisualGeometry,
    start: VisualCursor,
    direction: VisualDirection,
    windows: bool,
) -> Option<VisualCursor> {
    let categorize = state.char_categorizer(start.head);
    let mut category = None;
    raw.move_by_char(
        start,
        direction,
        Some(&mut |cluster| {
            let next = categorize.category(cluster);
            // Regular motion accepts space before a group; Windows accepts it after.
            if (windows && next == CharCategory::Space)
                || (!windows && category == Some(CharCategory::Space))
            {
                category = Some(next);
            }
            next == *category.get_or_insert(next)
        }),
    )
}

fn move_to_boundary(
    geometry: &Geometry<'_>,
    start: VisualCursor,
    direction: VisualDirection,
) -> Result<Option<VisualCursor>, EditError> {
    let Some(block) = geometry.line_block(start.head)? else {
        return Ok(None);
    };
    let block_edge = direction.edge(block.clone());
    let mut target = geometry.line_boundary(start, direction)?;
    if target.head == start.head && target.head != block_edge {
        let Some(unwrapped) = geometry.raw.unwrapped_line_boundary(start, direction) else {
            return Ok(None);
        };
        target = geometry.check_cursor(unwrapped)?;
    }
    if direction == Backward && target.head == block.start {
        let source = geometry.state.text().as_str();
        let indent = source[block.clone()]
            .chars()
            .take(100)
            .take_while(|&ch| space(ch))
            .map(char::len_utf8)
            .sum::<usize>();
        if indent > 0 && start.head != block.start + indent {
            return Ok(Some(VisualCursor::new(block.start + indent)));
        }
    }
    Ok(Some(target))
}

fn move_by_syntax(
    state: &EditorState,
    syntax: Option<&dyn VisualSyntax>,
    start: VisualCursor,
    direction: VisualDirection,
) -> Result<VisualCursor, EditError> {
    let assoc = Some(match direction {
        Backward => Assoc::After,
        Forward => Assoc::Before,
    });
    let make_cursor = |head| VisualCursor {
        assoc,
        ..VisualCursor::new(head)
    };
    let Some(syntax) = syntax else {
        return Ok(make_cursor(0));
    };
    let mut node = syntax.tree().root();
    if node.range() != (0..state.text().len()) {
        return Err(EditError::InvalidRange);
    }
    // Resolve with side zero: at a shared edge, start from the parent.
    while let Some(child) = node.children().find(|child| {
        let span = child.range();
        span.start < start.head && start.head < span.end
    }) {
        node = child;
    }
    let mut at = start.head;
    loop {
        let next = match direction {
            Forward => node.children().find(|child| at < child.range().end),
            Backward => node.children().rfind(|child| child.range().start < at),
        };
        let Some(child) = next else { break };
        let span = child.range();
        let text = state
            .text()
            .slice(span.clone())
            .ok_or(EditError::InvalidRange)?;
        if syntax.is_bracket(child, direction)
            || child.children().len() != 0
            || text.encode_utf16().take(3).count() > 2
            || text
                .chars()
                .any(|ch| !space(ch) && !matches!(ch, ',' | '.' | ';' | ':'))
        {
            node = child;
        } else {
            at = direction.edge(span);
        }
    }
    let head = if syntax.is_bracket(node, direction)
        && let Some(partner) = syntax.matching_bracket(node, direction)
    {
        if !state.text().is_valid_range(partner.clone()) {
            return Err(EditError::InvalidRange);
        }
        direction.edge(partner)
    } else {
        direction.edge(node.range())
    };
    Ok(make_cursor(head))
}

macro_rules! horizontal_commands {
    ($(#[doc = $doc:literal] $name:ident, $direction:expr, $unit:ident, $extend:literal;)*) => {
        $(
            #[doc = $doc]
            pub fn $name(
                state: &EditorState,
                geometry: &dyn VisualGeometry,
                selection: &VisualSelection,
            ) -> VisualCommandResult {
                horizontal(state, geometry, selection, $direction, HorizontalUnit::$unit, $extend)
            }
        )*
    };
}

horizontal_commands! {
    /// Move one visual grapheme to the left.
    cursor_char_left, Left, Char, false;
    /// Move one visual grapheme to the right.
    cursor_char_right, Right, Char, false;
    /// Move one visual grapheme forward.
    cursor_char_forward, Visual(Forward), Char, false;
    /// Move one visual grapheme backward.
    cursor_char_backward, Visual(Backward), Char, false;
    /// Extend the selection one visual grapheme to the left.
    select_char_left, Left, Char, true;
    /// Extend the selection one visual grapheme to the right.
    select_char_right, Right, Char, true;
    /// Extend the selection one visual grapheme forward.
    select_char_forward, Visual(Forward), Char, true;
    /// Extend the selection one visual grapheme backward.
    select_char_backward, Visual(Backward), Char, true;
    /// Move one visual group to the left.
    cursor_group_left, Left, Group, false;
    /// Move one visual group to the right.
    cursor_group_right, Right, Group, false;
    /// Move one visual group forward.
    cursor_group_forward, Visual(Forward), Group, false;
    /// Move one visual group backward.
    cursor_group_backward, Visual(Backward), Group, false;
    /// Extend the selection one visual group to the left.
    select_group_left, Left, Group, true;
    /// Extend the selection one visual group to the right.
    select_group_right, Right, Group, true;
    /// Extend the selection one visual group forward.
    select_group_forward, Visual(Forward), Group, true;
    /// Extend the selection one visual group backward.
    select_group_backward, Visual(Backward), Group, true;
    /// Move past one visual group and its trailing space.
    cursor_group_forward_win, Visual(Forward), GroupWindows, false;
    /// Extend the selection past one visual group and its trailing space.
    select_group_forward_win, Visual(Forward), GroupWindows, true;
    /// Move to the displayed row boundary on the left.
    cursor_line_boundary_left, Left, Boundary, false;
    /// Move to the displayed row boundary on the right.
    cursor_line_boundary_right, Right, Boundary, false;
    /// Move to the displayed row end, or the unwrapped end at a wrap edge.
    cursor_line_boundary_forward, Visual(Forward), Boundary, false;
    /// Move to the displayed row start or toggle between indentation and the block start.
    cursor_line_boundary_backward, Visual(Backward), Boundary, false;
    /// Extend the selection to the displayed row boundary on the left.
    select_line_boundary_left, Left, Boundary, true;
    /// Extend the selection to the displayed row boundary on the right.
    select_line_boundary_right, Right, Boundary, true;
    /// Extend the selection to the displayed row end or the unwrapped end.
    select_line_boundary_forward, Visual(Forward), Boundary, true;
    /// Extend the selection to the displayed row start, indentation, or the block start.
    select_line_boundary_backward, Visual(Backward), Boundary, true;
    /// Move to the unwrapped start of the current block.
    cursor_line_start, Visual(Backward), Line, false;
    /// Move to the unwrapped end of the current block.
    cursor_line_end, Visual(Forward), Line, false;
    /// Extend the selection to the line block start, without affinity.
    select_line_start, Visual(Backward), Line, true;
    /// Extend the selection to the line block end, without affinity.
    select_line_end, Visual(Forward), Line, true;
}

macro_rules! syntax_commands {
    ($(#[doc = $doc:literal] $name:ident, $direction:ident, $extend:literal;)*) => {
        $(
            #[doc = $doc]
            pub fn $name(
                state: &EditorState,
                geometry: &dyn VisualGeometry,
                selection: &VisualSelection,
                syntax: Option<&dyn VisualSyntax>,
            ) -> VisualCommandResult {
                horizontal(state, geometry, selection, $direction, HorizontalUnit::Syntax(syntax), $extend)
            }
        )*
    };
}

syntax_commands! {
    /// Move over the next syntactic element to the left. `None` uses the empty syntax tree.
    cursor_syntax_left, Left, false;
    /// Move over the next syntactic element to the right. `None` uses the empty syntax tree.
    cursor_syntax_right, Right, false;
    /// Extend the selection over the next syntactic element to the left. `None` uses the empty syntax tree.
    select_syntax_left, Left, true;
    /// Extend the selection over the next syntactic element to the right. `None` uses the empty syntax tree.
    select_syntax_right, Right, true;
}

fn add_cursor(
    state: &EditorState,
    raw: &dyn VisualGeometry,
    selection: &VisualSelection,
    direction: VisualDirection,
) -> VisualCommandResult {
    let geometry = Geometry::new(state, raw, selection)?;
    let mut ranges: Vec<_> = selection
        .selection
        .ranges()
        .iter()
        .copied()
        .zip(selection.cursors.iter().copied())
        .collect();
    let mut heads: HashSet<_> = selection.cursors.iter().map(|cursor| cursor.head).collect();
    let projection = crate::language::line_projection::command_line_projection(state);
    let lines = projection.as_ref().unwrap_or(state.text());
    for &start in &selection.cursors {
        let line = super::line_at(lines, start.head).range;
        if match direction {
            Backward => line.start == 0,
            Forward => line.end == state.text().len(),
        } {
            continue;
        }
        let mut cursor = start;
        let mut visited = HashSet::from([cursor.head]);
        loop {
            let next = geometry.move_vertical(cursor, direction, VerticalDistance::Line)?;
            if next.head < line.start || next.head > line.end {
                if heads.insert(next.head) {
                    ranges.push((caret(next.head), next));
                }
                break;
            }
            if next.head == cursor.head {
                break;
            }
            if !visited.insert(next.head) {
                return Err(EditError::InvalidSelection);
            }
            cursor = next;
        }
    }
    if ranges.len() == selection.cursors.len() {
        return Ok(None);
    }
    let main = ranges.len() - 1;
    Ok(Some(selection_update(VisualSelection::normalize(
        ranges, main,
    )?)))
}

/// Add a cursor above each head, skipping rows of the same logical line.
pub fn add_cursor_above(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    add_cursor(state, geometry, selection, Backward)
}

/// Add a cursor below each head, skipping rows of the same logical line.
pub fn add_cursor_below(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> VisualCommandResult {
    add_cursor(state, geometry, selection, Forward)
}

fn delete_boundary(
    state: &EditorState,
    raw: &dyn VisualGeometry,
    selection: &VisualSelection,
    direction: VisualDirection,
) -> CommandResult {
    if state.read_only() {
        return Ok(None);
    }
    let geometry = Geometry::new(state, raw, selection)?;
    let source = state.text().as_str();
    let mut cursors = selection.cursors.iter().copied();
    let mut event = "delete.selection";
    let transaction = state.change_by_range(|range| {
        let cursor = cursors.next().expect("one cursor per range");
        let span = range.range();
        let deleted = if span.is_empty() {
            let at = cursor.head;
            let edge = geometry.line_boundary(cursor, direction)?.head;
            // At the wrap edge, delete one scalar in this direction. The
            // browser can delete one UTF-16 unit; Rust cannot split a scalar.
            let target = match direction {
                Backward if edge < at => edge,
                Forward if edge > at => edge,
                Backward if source[..at].ends_with("\r\n") => at - 2,
                Forward if source[at..].starts_with("\r\n") => at + 2,
                Backward => source[..at]
                    .char_indices()
                    .next_back()
                    .map_or(at, |(pos, _)| pos),
                Forward => at + source[at..].chars().next().map_or(0, char::len_utf8),
            };
            event = match target.cmp(&at) {
                Ordering::Less => "delete.backward",
                Ordering::Greater => "delete.forward",
                Ordering::Equal => return Ok(unchanged(range)),
            };
            let target = geometry.skip_atoms(target, direction, 0);
            at.min(target)..at.max(target)
        } else {
            geometry.skip_atoms(span.start, Backward, 0)..geometry.skip_atoms(span.end, Forward, 0)
        };
        let deleted = include_crlf(source, deleted);
        let start = deleted.start;
        replace_range(deleted, "", start)
    })?;
    Ok(super::delete::finish_deletion(state, transaction, event))
}

/// Delete each selection, or delete back to the start of its displayed row.
/// If that does not move backward, delete one scalar or line break instead.
/// Reset retained visual selection information after applying the edit.
pub fn delete_line_boundary_backward(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> CommandResult {
    delete_boundary(state, geometry, selection, Backward)
}

/// Delete each selection, or delete forward to the end of its displayed row.
/// If that does not move forward, delete one scalar or line break instead.
/// Reset retained visual selection information after applying the edit.
pub fn delete_line_boundary_forward(
    state: &EditorState,
    geometry: &dyn VisualGeometry,
    selection: &VisualSelection,
) -> CommandResult {
    delete_boundary(state, geometry, selection, Forward)
}
