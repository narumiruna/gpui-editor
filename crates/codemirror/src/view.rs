//! Native view state, viewport updates, and inline mark decorations.
//! Mark edge rules follow CodeMirror view 6.44.0. See `docs/licenses/codemirror-view.txt`.

use std::{ops::Range as ByteRange, sync::LazyLock};

pub mod bidi;
mod clipboard;
pub mod decorations;
mod exception;
mod focus;
pub mod gutters;
pub mod highlights;
pub mod input;
pub mod keymap;
pub mod matches;
pub mod measure;
pub mod panels;
mod plugins;
mod scroll;
mod scroll_handlers;
pub mod selection_layers;
pub mod tooltips;
mod wast_viewport;

pub use wast_viewport::{WastIdleStep, WastIdleWork, WastViewportError};

pub use clipboard::{
    ClipboardCopy, ClipboardLinewiseMemory, clipboard_input_filter, clipboard_output_filter,
    filter_clipboard_input, filter_clipboard_output, plan_clipboard_copy, plan_clipboard_paste,
    plan_linewise_copy, plan_linewise_copy_projected, plan_linewise_paste,
    plan_linewise_paste_projected,
};
pub use exception::{ViewException, exception_sink, log_exception};
pub use focus::{focus_change_effect, focus_change_transaction};
pub use highlights::{
    ViewHighlight, highlight_active_line, highlight_active_line_for_blocks,
    highlight_trailing_whitespace, highlight_whitespace,
};
pub use matches::{
    MatchContext, MatchDecorator, PlaceholderProjection, SpecialChar, SpecialCharAppearance,
    SpecialCharConfig, SpecialCharRender, highlight_special_chars, placeholder,
    placeholder_extension, placeholder_projection, special_char_appearance,
    special_char_decorations, special_char_extension,
};
use plugins::ConfiguredPlugins;
pub use plugins::{ViewPluginType, update_listener};
pub use scroll::{
    CURSOR_SCROLL_MARGIN, CursorScrollMargin, EndScrollPadding, FixedScroller, SCROLL_INTO_VIEW,
    SCROLL_SNAPSHOT, ScrollBlock, ScrollDelta, ScrollOffset, ScrollRect, ScrollSnapshot,
    ScrollSnapshotGeometry, ScrollStrategy, ScrollTarget, scroll_past_end,
};
pub use scroll_handlers::{
    CoveredMargins, ScrollMargins, ScrollView, covered_margins, scroll_handler, try_scroll_handler,
};

use crate::{
    Assoc, ChangeDesc, ChangeSet, EditError, EditorSelection, EditorState, History, MapMode, Range,
    RangeValue, StateEffect, StateEffectType, Text, Transaction, history::UpdateReport,
};

/// Request that the host announce this plain-text message to the user.
/// The core does not speak or render the message itself.
pub static ANNOUNCE: LazyLock<StateEffectType<String>> = LazyLock::new(StateEffectType::new);

/// A visible document interval and the parts of that interval the host draws.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Viewport {
    interval: ByteRange<usize>,
    visible: Vec<ByteRange<usize>>,
}

impl Viewport {
    pub fn new(
        text: &Text,
        interval: ByteRange<usize>,
        visible: impl IntoIterator<Item = ByteRange<usize>>,
    ) -> Result<Self, EditError> {
        if text.slice(interval.clone()).is_none() {
            return Err(EditError::InvalidRange);
        }
        let mut ranges = Vec::new();
        for range in visible {
            if range.start < interval.start
                || range.end > interval.end
                || text.slice(range.clone()).is_none()
            {
                return Err(EditError::InvalidRange);
            }
            ranges.push(range);
        }
        Ok(Self {
            interval,
            visible: normalize(ranges),
        })
    }

    /// Create an interval around a collection of visible source ranges.
    pub fn around(
        text: &Text,
        ranges: impl IntoIterator<Item = ByteRange<usize>>,
    ) -> Result<Self, EditError> {
        let ranges: Vec<_> = ranges.into_iter().collect();
        let start = ranges.iter().map(|range| range.start).min().unwrap_or(0);
        let end = ranges.iter().map(|range| range.end).max().unwrap_or(0);
        Self::new(text, start..end, ranges)
    }

    pub fn complete(text: &Text) -> Self {
        let interval = 0..text.len();
        Self {
            interval: interval.clone(),
            visible: std::iter::once(interval).collect(),
        }
    }

    pub fn interval(&self) -> ByteRange<usize> {
        self.interval.clone()
    }
    pub fn visible_ranges(&self) -> &[ByteRange<usize>] {
        &self.visible
    }

    fn map(&self, changes: &ChangeDesc) -> Self {
        let map = |range: &ByteRange<usize>| {
            let from = changes
                .map_pos(range.start, Assoc::Before, MapMode::Simple)
                .expect("viewport start was in the old document");
            let to = changes
                .map_pos(range.end, Assoc::After, MapMode::Simple)
                .expect("viewport end was in the old document");
            from..to
        };
        Self {
            interval: map(&self.interval),
            visible: normalize(self.visible.iter().map(map).collect()),
        }
    }
}

fn normalize(mut ranges: Vec<ByteRange<usize>>) -> Vec<ByteRange<usize>> {
    ranges.sort_by_key(|range| (range.start, range.end));
    let mut result: Vec<ByteRange<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = result.last_mut()
            && range.start <= last.end
        {
            last.end = last.end.max(range.end);
        } else {
            result.push(range);
        }
    }
    result
}

/// An inline mark over a nonempty range of document text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarkDecoration {
    class: String,
    inclusive_start: bool,
    inclusive_end: bool,
    bidi_direction: Option<bidi::Direction>,
}

impl MarkDecoration {
    pub fn new(class: impl Into<String>) -> Self {
        Self {
            class: class.into(),
            inclusive_start: false,
            inclusive_end: false,
            bidi_direction: None,
        }
    }

    pub fn with_inclusive_edges(mut self, start: bool, end: bool) -> Self {
        self.inclusive_start = start;
        self.inclusive_end = end;
        self
    }

    /// Set the direction used when this mark is in `BIDI_ISOLATED_RANGES`.
    /// `None` uses the first strong character in the isolated text.
    pub fn with_bidi_direction(mut self, direction: Option<bidi::Direction>) -> Self {
        self.bidi_direction = direction;
        self
    }

    pub fn bidi_direction(&self) -> Option<bidi::Direction> {
        self.bidi_direction
    }

    pub fn class(&self) -> &str {
        &self.class
    }

    pub fn range(self, range: ByteRange<usize>) -> Result<Range<Self>, EditError> {
        if range.is_empty() {
            return Err(EditError::InvalidRange);
        }
        Ok(Range::new(range.start, range.end, self))
    }
}

impl RangeValue for MarkDecoration {
    fn start_side(&self) -> i32 {
        if self.inclusive_start {
            -1
        } else {
            500_000_000
        }
    }
    fn end_side(&self) -> i32 {
        if self.inclusive_end { 1 } else { -600_000_000 }
    }

    fn eq(&self, other: &Self) -> bool {
        self.class == other.class && self.bidi_direction == other.bidi_direction
    }
}

/// A native editor update. Coordinates in `changes` link the two states.
pub struct ViewUpdate<'a> {
    pub start_state: &'a EditorState,
    pub state: &'a EditorState,
    pub changes: &'a ChangeDesc,
    /// Effects accepted with the dispatched transaction or replay. Geometry
    /// reports, direct selection reports, and WAST publications have none.
    pub effects: &'a [StateEffect],
    pub start_viewport: &'a Viewport,
    pub viewport: &'a Viewport,
    pub selection_set: bool,
    pub viewport_changed: bool,
    pub focus_changed: bool,
    pub focused: bool,
}

impl ViewUpdate<'_> {
    pub fn doc_changed(&self) -> bool {
        !self.changes.is_empty()
    }

    /// Plain-text announcements for the host, in dispatch order.
    pub fn announcements(&self) -> impl Iterator<Item = &str> {
        self.effects
            .iter()
            .filter_map(|effect| effect.get(&ANNOUNCE).map(String::as_str))
    }
}

/// A subscriber to edits, selection updates, viewport changes, and focus changes.
pub trait ViewPlugin {
    fn update(&mut self, _update: &ViewUpdate<'_>) {}

    /// A returned error disables the plugin without undoing the accepted edit.
    /// Rust panics are not intercepted.
    fn try_update(&mut self, update: &ViewUpdate<'_>) -> Result<(), ViewException> {
        self.update(update);
        Ok(())
    }

    fn destroy(&mut self) {}
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PluginId(usize);

/// A request to reveal a range or restore the editor's own scroll position.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScrollRequest {
    Reveal(ScrollTarget),
    Restore(ScrollSnapshot),
}

impl ScrollRequest {
    fn map(self, changes: &ChangeDesc) -> Option<Self> {
        match self {
            Self::Reveal(target) => target.map(changes).map(Self::Reveal),
            Self::Restore(snapshot) => snapshot.map(changes).map(Self::Restore),
        }
    }

    fn clip(self, text: &Text) -> Option<Self> {
        match self {
            Self::Reveal(target) => target.clip(text).map(Self::Reveal),
            Self::Restore(snapshot) => snapshot.clip(text).map(Self::Restore),
        }
    }
}

/// Editor state and history with the lifecycle needed by a native view host.
pub struct EditorView<M> {
    history: History<M>,
    viewport: Viewport,
    wast_viewport: wast_viewport::WastViewport,
    scroll_request: Option<ScrollRequest>,
    focused: bool,
    revision: u64,
    next_plugin: usize,
    plugins: Vec<(PluginId, Box<dyn ViewPlugin>)>,
    configured: ConfiguredPlugins,
}

impl<M: Clone> EditorView<M> {
    pub fn new(state: EditorState) -> Self {
        Self::create(state, None)
    }

    /// Create a view with an initial scroll request in the given state's coordinates.
    pub fn new_with_scroll(state: EditorState, request: ScrollRequest) -> Self {
        Self::create(state, Some(request))
    }

    fn create(state: EditorState, request: Option<ScrollRequest>) -> Self {
        let viewport = Viewport::complete(state.text());
        let scroll_request = request.and_then(|request| request.clip(state.text()));
        let mut history = History::new(state);
        let mut wast_viewport = wast_viewport::WastViewport::default();
        wast_viewport.sync(&mut history, None, None, 0);
        let mut configured = ConfiguredPlugins::default();
        configured.refresh(history.state(), None);
        Self {
            history,
            viewport,
            wast_viewport,
            scroll_request,
            focused: false,
            revision: 0,
            next_plugin: 0,
            plugins: Vec::new(),
            configured,
        }
    }

    pub fn state(&self) -> &EditorState {
        self.history.state()
    }
    pub fn viewport(&self) -> &Viewport {
        &self.viewport
    }
    pub fn focused(&self) -> bool {
        self.focused
    }

    /// Replace the state and reset history, viewport and pending scroll. Drop
    /// the old plugins and construct the new state's configured plugins. The
    /// host retains focus and must report geometry for the replacement text.
    /// An incoming WAST publication is retained, but the old job and tickets
    /// are discarded.
    pub fn set_state(&mut self, state: EditorState) {
        self.configured.clear();
        for (_, mut plugin) in self.plugins.drain(..) {
            plugin.destroy();
        }
        self.viewport = Viewport::complete(state.text());
        self.wast_viewport = wast_viewport::WastViewport::default();
        self.scroll_request = None;
        self.revision = self.revision.wrapping_add(1);
        self.history = History::new(state);
        self.wast_viewport.sync(&mut self.history, None, None, 0);
        self.configured.refresh(self.history.state(), None);
    }

    /// Read the typed instance installed by an extension. A missing or failed
    /// plugin has no instance.
    pub fn plugin<A, P: ViewPlugin + 'static>(&self, kind: &ViewPluginType<A, P>) -> Option<&P> {
        self.configured.get(kind)
    }

    /// Take the last accepted scroll request at its current document position.
    /// The host consumes it when it has a layout for the current document.
    pub fn take_scroll_request(&mut self) -> Option<ScrollRequest> {
        self.scroll_request.take()
    }

    /// Take a range reveal request. Leave a pending snapshot in the view.
    pub fn take_scroll_target(&mut self) -> Option<ScrollTarget> {
        let Some(ScrollRequest::Reveal(target)) = self.scroll_request else {
            return None;
        };
        self.scroll_request = None;
        Some(target)
    }

    /// Take a scroll restoration request. Leave a pending range reveal in the view.
    pub fn take_scroll_snapshot(&mut self) -> Option<ScrollSnapshot> {
        let Some(ScrollRequest::Restore(snapshot)) = self.scroll_request else {
            return None;
        };
        self.scroll_request = None;
        Some(snapshot)
    }
    /// A serial number that changes when an edit changes the document or the view replaces its state.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn add_plugin(&mut self, plugin: impl ViewPlugin + 'static) -> PluginId {
        let id = PluginId(self.next_plugin);
        self.next_plugin = self
            .next_plugin
            .checked_add(1)
            .expect("view plugin IDs exhausted");
        self.plugins.push((id, Box::new(plugin)));
        id
    }

    /// Create a plugin from the current view. Report a construction error to
    /// the configured sink and leave the plugin list unchanged.
    pub fn try_add_plugin<P: ViewPlugin + 'static>(
        &mut self,
        create: impl FnOnce(&Self) -> Result<P, ViewException>,
    ) -> Option<PluginId> {
        match create(self) {
            Ok(plugin) => Some(self.add_plugin(plugin)),
            Err(exception) => {
                log_exception(
                    self.state(),
                    &exception.with_context("view plugin construction"),
                );
                None
            }
        }
    }

    pub fn remove_plugin(&mut self, id: PluginId) -> bool {
        let Some(index) = self.plugins.iter().position(|(key, _)| *key == id) else {
            return false;
        };
        let (_, mut plugin) = self.plugins.remove(index);
        plugin.destroy();
        true
    }

    /// Report geometry without changing the last reported focus. Hosts using
    /// [`Self::set_focus`] can call this from layout without dispatching effects.
    pub fn set_viewport_geometry(&mut self, viewport: Viewport) -> Result<(), EditError> {
        self.set_viewport(viewport, self.focused)
    }

    /// Report layout and focus without dispatching a focus transaction. Hosts
    /// that use focus effect providers should report focus through [`Self::set_focus`].
    pub fn set_viewport(&mut self, viewport: Viewport, focused: bool) -> Result<(), EditError> {
        self.report_viewport(viewport, focused, true)
    }

    fn report_viewport(
        &mut self,
        viewport: Viewport,
        focused: bool,
        measured: bool,
    ) -> Result<(), EditError> {
        let text = self.state().text();
        if text.slice(viewport.interval()).is_none()
            || viewport
                .visible_ranges()
                .iter()
                .any(|range| text.slice(range.clone()).is_none())
        {
            return Err(EditError::InvalidRange);
        }
        self.wast_viewport.has_geometry |= measured;
        let focus_changed = focused != self.focused;
        if !focus_changed && self.viewport == viewport {
            return Ok(());
        }
        let previous = std::mem::replace(&mut self.viewport, viewport);
        self.focused = focused;
        if let Some(start_state) = self.previous_state() {
            let changes = ChangeSet::default().desc(start_state.text().len())?;
            self.notify(&start_state, &changes, &previous, false, focus_changed, &[]);
        }
        Ok(())
    }

    /// Dispatch the configured effects for a change in native focus, before
    /// publishing one focus update. Call outside paint with the current host
    /// metadata. Filters and history apply as for other transactions. An error
    /// leaves the reported focus and editor state unchanged.
    pub fn set_focus(&mut self, focused: bool, metadata: M) -> Result<M, EditError> {
        self.set_focus_checked(focused, metadata, |_| Ok(()))
    }

    /// As [`Self::set_focus`], with a host check after filters and extenders.
    /// For example, a Markdown host can reject a filter that adds a text edit.
    /// The check runs only when a focus transaction exists. A rejected focus
    /// can still be reported without effects by [`Self::set_viewport`].
    pub fn set_focus_checked(
        &mut self,
        focused: bool,
        metadata: M,
        check: impl FnOnce(&Transaction) -> Result<(), EditError>,
    ) -> Result<M, EditError> {
        if self.focused == focused {
            return Ok(metadata);
        }
        let Some(transaction) = focus_change_transaction(self.state(), self.focused, focused)?
        else {
            self.report_viewport(self.viewport.clone(), focused, false)?;
            return Ok(metadata);
        };
        let previous = self.state().clone();
        let transaction = previous.filter_transaction(transaction)?;
        check(&transaction)?;
        let length = previous.text().len();
        let margin = CursorScrollMargin::configured(&previous);
        let (result, accepted) = self.history.update_resolved_reporting(
            transaction,
            metadata.clone(),
            |_| (None, metadata),
            true,
        )?;
        self.focused = focused;
        self.report_transition(Some(previous), accepted, length, margin, true);
        Ok(result)
    }

    pub fn update(
        &mut self,
        transaction: Transaction,
        metadata: M,
        finish: impl FnOnce(&EditorState) -> (Option<EditorSelection>, M),
    ) -> Result<M, EditError> {
        self.update_checked(transaction, metadata, |_| Ok(()), finish)
    }

    /// Apply filters and extenders once, then check the resolved transaction
    /// against host rules. A rejected check does not apply edits, effects,
    /// selection, or history and does not notify view plugins.
    pub fn update_checked(
        &mut self,
        transaction: Transaction,
        metadata: M,
        check: impl FnOnce(&Transaction) -> Result<(), EditError>,
        finish: impl FnOnce(&EditorState) -> (Option<EditorSelection>, M),
    ) -> Result<M, EditError> {
        let transaction = self.state().filter_transaction(transaction)?;
        check(&transaction)?;
        // An accepted transaction can install a listener through a filter,
        // even if neither the old state nor the requested effects list it.
        let previous = Some(self.state().clone());
        let length = self.state().text().len();
        let margin = CursorScrollMargin::configured(self.state());
        let (result, accepted) =
            self.history
                .update_resolved_reporting(transaction, metadata, finish, true)?;
        self.report_transition(previous, accepted, length, margin, false);
        Ok(result)
    }

    pub fn set_selection(&mut self, selection: EditorSelection) -> Result<(), EditError> {
        let previous = self.previous_state();
        self.history.set_selection(selection)?;
        self.report_transition(
            previous,
            UpdateReport {
                selection_set: true,
                ..Default::default()
            },
            self.state().text().len(),
            CursorScrollMargin::default(),
            false,
        );
        Ok(())
    }

    pub fn start_group(&mut self) {
        self.history.start_group();
    }
    pub fn end_group(&mut self) {
        self.history.end_group();
    }
    pub fn undo_depth(&self) -> usize {
        self.history.undo_depth()
    }
    pub fn redo_depth(&self) -> usize {
        self.history.redo_depth()
    }
    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
    pub fn undo(&mut self) -> Option<M> {
        self.replay(false, None)
    }
    pub fn redo(&mut self) -> Option<M> {
        self.replay(true, None)
    }

    pub fn can_undo_selection(&self) -> bool {
        self.history.can_undo_selection()
    }
    pub fn can_redo_selection(&self) -> bool {
        self.history.can_redo_selection()
    }

    /// Replay a selection step, or replay an edit if there is no selection step.
    /// A selection-only step returns the caller's current metadata.
    pub fn undo_selection(&mut self, metadata: M) -> Option<M> {
        self.replay(false, Some(metadata))
    }

    pub fn redo_selection(&mut self, metadata: M) -> Option<M> {
        self.replay(true, Some(metadata))
    }

    fn replay(&mut self, redo: bool, selection: Option<M>) -> Option<M> {
        let previous = if self.state().wast_publication.is_some() {
            Some(self.state().clone())
        } else {
            self.previous_state()
        };
        let length = self.state().text().len();
        let margin = CursorScrollMargin::configured(self.state());
        let (metadata, accepted) = match (redo, selection) {
            (false, Some(current)) => self.history.undo_selection_reporting(current, true),
            (true, Some(current)) => self.history.redo_selection_reporting(current, true),
            (false, None) => self.history.undo_reporting(true),
            (true, None) => self.history.redo_reporting(true),
        }?;
        self.report_transition(previous, accepted, length, margin, false);
        Some(metadata)
    }

    fn previous_state(&self) -> Option<EditorState> {
        (!self.plugins.is_empty()
            || !self.configured.is_empty()
            || plugins::has_listeners(self.state()))
        .then(|| self.state().clone())
    }

    fn report_transition(
        &mut self,
        start: Option<EditorState>,
        accepted: UpdateReport,
        length: usize,
        margin: CursorScrollMargin,
        focus_changed: bool,
    ) {
        let changes = accepted.changes.as_ref().map(|changes| {
            changes
                .desc(length)
                .expect("accepted changes have valid lengths")
        });
        self.wast_viewport.sync(
            &mut self.history,
            start.as_ref(),
            changes.as_ref().filter(|changes| !changes.is_empty()),
            self.viewport.interval().end,
        );
        if start.is_none() {
            self.configured.refresh(self.history.state(), None);
        }
        let explicit_scroll = accepted.effects.iter().rev().find_map(|effect| {
            effect
                .get(&SCROLL_INTO_VIEW)
                .copied()
                .map(ScrollRequest::Reveal)
                .or_else(|| {
                    effect
                        .get(&SCROLL_SNAPSHOT)
                        .copied()
                        .map(ScrollRequest::Restore)
                })
        });
        if start.is_none()
            && accepted.changes.is_none()
            && explicit_scroll.is_none()
            && !accepted.scroll_into_view
        {
            return;
        }
        let changes = changes.unwrap_or_else(|| {
            ChangeSet::default()
                .desc(length)
                .expect("unchanged document length")
        });
        let previous = self.viewport.clone();
        if !changes.is_empty() {
            self.viewport = self.viewport.map(&changes);
            self.scroll_request = self
                .scroll_request
                .and_then(|request| request.map(&changes));
            self.revision = self.revision.wrapping_add(1);
        }
        if accepted.scroll_into_view {
            let main = self.state().selection().main();
            let range = if main.is_empty() {
                main
            } else {
                let side = if main.head() > main.anchor() {
                    Assoc::Before
                } else {
                    Assoc::After
                };
                EditorSelection::cursor(main.head(), Some(side), None, None)
            };
            self.scroll_request = Some(ScrollRequest::Reveal(ScrollTarget {
                x_margin: margin.x,
                y_margin: margin.y,
                ..ScrollTarget::new(range)
            }));
        }
        if let Some(request) = explicit_scroll.and_then(|request| request.clip(self.state().text()))
        {
            self.scroll_request = Some(request);
        }
        if let Some(start) = start {
            self.notify(
                &start,
                &changes,
                &previous,
                accepted.selection_set,
                focus_changed,
                &accepted.effects,
            );
        }
    }

    fn notify(
        &mut self,
        previous: &EditorState,
        changes: &ChangeDesc,
        viewport: &Viewport,
        selection_set: bool,
        focus_changed: bool,
        effects: &[StateEffect],
    ) {
        let update = ViewUpdate {
            start_state: previous,
            state: self.history.state(),
            changes,
            effects,
            start_viewport: viewport,
            viewport: &self.viewport,
            selection_set,
            viewport_changed: viewport != &self.viewport,
            focus_changed,
            focused: self.focused,
        };
        self.configured.refresh(update.state, Some(&update));
        self.plugins
            .retain_mut(|(_, plugin)| match plugin.try_update(&update) {
                Ok(()) => true,
                Err(exception) => {
                    log_exception(update.state, &exception.with_context("view plugin update"));
                    plugin.destroy();
                    false
                }
            });
        plugins::notify_listeners(&update);
    }
}

impl<M> Drop for EditorView<M> {
    fn drop(&mut self) {
        for (_, plugin) in &mut self.plugins {
            plugin.destroy();
        }
    }
}
