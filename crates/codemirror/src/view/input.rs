//! Input interception from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt`.
//!
//! The host detects input and decides whether to offer it to handlers. This module
//! does not detect DOM changes, selection-only events, atomic ranges, or IME changes.
//! The single-edit planner does not resolve or dispatch transactions. The opt-in
//! headless path dispatches a replacement or a host default with optional completion.

use std::{
    cell::{Cell, OnceCell},
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Change, ChangeSet, EditError, EditorState, Extension, Facet, History, SelectionRange,
    StateEffect, Transaction, history::UpdateReport,
};

use super::{EditorView, Transition};

/// One host-detected text replacement and the state before it. Positions are UTF-8 bytes.
#[derive(Clone, Copy, Debug)]
pub struct InputChange<'a> {
    state: &'a EditorState,
    from: usize,
    to: usize,
    text: &'a str,
}

impl<'a> InputChange<'a> {
    /// Check the replacement range against the old document. Empty text and
    /// empty ranges are valid. The host must first decide whether input is allowed,
    /// for example when the state is read-only.
    pub fn new(
        state: &'a EditorState,
        range: Range<usize>,
        text: &'a str,
    ) -> Result<Self, EditError> {
        if !state.text().is_valid_range(range.clone()) {
            return Err(EditError::InvalidRange);
        }
        Ok(Self {
            state,
            from: range.start,
            to: range.end,
            text,
        })
    }

    pub fn state(self) -> &'a EditorState {
        self.state
    }

    pub fn range(self) -> Range<usize> {
        self.from..self.to
    }

    pub fn text(self) -> &'a str {
        self.text
    }
}

/// The default transaction for this operation, built only on first use.
/// All calls return the same transaction or error. Reading it does not apply it.
pub type DefaultInput<'a> = dyn Fn() -> Result<&'a Transaction, EditError> + 'a;

/// A handler can pass, consume the input without an edit, or propose a transaction.
#[derive(Debug)]
pub enum InputHandling {
    Pass,
    Handled(Option<Transaction>),
}

type Handler =
    Arc<dyn for<'a> Fn(InputChange<'a>, &'a DefaultInput<'a>) -> InputHandling + Send + Sync>;
static INPUT_HANDLERS: LazyLock<Facet<Handler, Vec<Handler>>> = LazyLock::new(Facet::collect);

/// Register a handler in extension precedence order. The first `Handled`
/// result stops the chain, including when it contains no transaction.
pub fn input_handler(
    handler: impl for<'a> Fn(InputChange<'a>, &'a DefaultInput<'a>) -> InputHandling
    + Send
    + Sync
    + 'static,
) -> Extension {
    INPUT_HANDLERS.of(Arc::new(handler))
}

/// Offer the checked change to handlers. If all handlers pass, return `Pass`.
/// The host can then continue its own input path. The default factory is only
/// called if a handler asks for it. Use the supplied state as its start state.
/// A factory error is returned to the requesting handler, which can still pass.
/// The factory and handlers must not apply edits. The returned transaction, if
/// any, has checked change and selection coordinates. The host still owns
/// validation of effects, transaction filtering, and dispatch.
pub fn run_input_handlers(
    change: InputChange<'_>,
    default_factory: impl FnOnce() -> Result<Transaction, EditError>,
) -> Result<InputHandling, EditError> {
    process(change, default_factory, false)
}

/// An accepted host input and the native rule to use for its default edit.
#[derive(Clone, Debug)]
pub enum InputProposal<'a> {
    /// Report the primary selection as the replacement range. The default
    /// replaces every current selection and puts each caret after its insert.
    ReplaceSelection(&'a str),
    /// Replace this exact old-document range. An optional primary selection
    /// uses post-edit coordinates. When it is absent, dispatch maps the old
    /// selections. When present, other ranges keep their coordinates. An invalid
    /// resulting selection is an error, before any handlers run.
    Replace {
        range: Range<usize>,
        text: &'a str,
        selection: Option<SelectionRange>,
    },
}

/// Offer the input to handlers and return the default if they all pass.
/// The host chooses the default rule. In CodeMirror the DOM diff, resulting
/// selection, atomic ranges, and composition state determine that rule.
/// For example, an ordinary edit within the primary selection can extend to
/// the full selection and repeat at other selections. The browser can also
/// use the composition node to decide which other cursors to edit. This
/// planner does not infer either case or label IME changes. Use
/// [`run_input_handlers`] to supply a host-specific default.
///
/// The built-in defaults preserve the supplied text bytes, request scrolling,
/// and carry `input.type`. Dispatch against the same state to run filters and
/// apply the accepted transaction with the host's history rules.
pub fn plan_input(
    state: &EditorState,
    proposal: InputProposal<'_>,
) -> Result<Option<Transaction>, EditError> {
    let outcome = match proposal {
        InputProposal::ReplaceSelection(text) => {
            let change = InputChange::new(state, state.selection().main().range(), text)?;
            process(change, || state.replace_selection(text).map(typed), true)?
        }
        InputProposal::Replace {
            range,
            text,
            selection,
        } => {
            let change = InputChange::new(state, range, text)?;
            let changes = ChangeSet::single(Change::new(change.range(), text))?;
            let selection = selection
                .map(|range| {
                    let boundary = |pos| changes.is_boundary_after(state.text(), pos);
                    if !boundary(range.anchor()) || !boundary(range.head()) {
                        return Err(EditError::InvalidSelection);
                    }
                    let old = state.selection();
                    let selection = old.replace_range(old.main_index(), range)?;
                    selection.check(boundary)?;
                    Ok(selection)
                })
                .transpose()?;
            process(
                change,
                move || {
                    let mut transaction = Transaction::new(changes);
                    if let Some(selection) = selection {
                        transaction = transaction.with_selection(selection);
                    }
                    Ok(typed(transaction))
                },
                true,
            )?
        }
    };
    match outcome {
        InputHandling::Handled(transaction) => Ok(transaction),
        InputHandling::Pass => unreachable!("the planner uses a default when all handlers pass"),
    }
}

/// One resolved step of a headless input. Changes use `start_state` positions.
/// The explicit selection and effects use positions in `state`.
#[derive(Debug)]
pub struct InputStep {
    pub start_state: EditorState,
    pub state: EditorState,
    pub transaction: Transaction,
}

/// The resolved host default and its resulting state, built on first use.
/// All calls return the same step or error.
pub type DefaultInputState<'a> = dyn Fn() -> Result<&'a InputStep, EditError> + 'a;

type ReplacementHandler =
    Arc<dyn for<'a> Fn(InputChange<'a>) -> Result<Option<Transaction>, EditError> + Send + Sync>;
type CompletionHandler = Arc<
    dyn for<'a> Fn(
            InputChange<'a>,
            &'a DefaultInputState<'a>,
        ) -> Result<Option<Transaction>, EditError>
        + Send
        + Sync,
>;
#[derive(Clone)]
enum HeadlessHandler {
    Replacement(ReplacementHandler),
    Completion(CompletionHandler),
}
static HEADLESS_HANDLERS: LazyLock<Facet<HeadlessHandler, Vec<HeadlessHandler>>> =
    LazyLock::new(Facet::collect);

/// Register a headless replacement handler in the same precedence order as
/// completion handlers. `Ok(None)` passes. `Ok(Some(transaction))` consumes the
/// input and uses that proposal against the original state instead of the host
/// default, even if filters remove its changes. No user event is added to the
/// proposal. `Err` aborts the input. This extension does not affect the
/// single-edit planner.
pub fn headless_input_handler(
    handler: impl for<'a> Fn(InputChange<'a>) -> Result<Option<Transaction>, EditError>
    + Send
    + Sync
    + 'static,
) -> Extension {
    HEADLESS_HANDLERS.of(HeadlessHandler::Replacement(Arc::new(handler)))
}

/// Register a headless completion handler in extension precedence order.
/// A handler can pass with `Ok(None)` or return a proposal in the default's
/// resulting document. The first proposal stops the chain, even if filters
/// remove its changes. This extension does not affect the single-edit planner.
pub fn input_completion_handler(
    handler: impl for<'a> Fn(
        InputChange<'a>,
        &'a DefaultInputState<'a>,
    ) -> Result<Option<Transaction>, EditError>
    + Send
    + Sync
    + 'static,
) -> Extension {
    HEADLESS_HANDLERS.of(HeadlessHandler::Completion(Arc::new(handler)))
}

impl<M: Clone> EditorView<M> {
    /// Dispatch a headless replacement or the host default with an optional
    /// second `input.complete` step. The host supplies the default proposal and
    /// decides whether input is allowed. Only headless handlers run on this path.
    ///
    /// Resolve each step against its predecessor and publish one view update.
    /// History groups the steps with its ordinary rules and the same metadata.
    /// Replay infers the opposite selection from each recorded change.
    /// An error does not change the view or history. Side effects of user
    /// callbacks that ran while resolving the steps are not rolled back.
    pub fn apply_headless_input(
        &mut self,
        range: Range<usize>,
        text: &str,
        default_factory: impl FnOnce(&EditorState) -> Result<Transaction, EditError>,
        metadata: M,
    ) -> Result<(), EditError> {
        let change = InputChange::new(self.state(), range, text)?;
        let handlers = change.state().context().facet(&HEADLESS_HANDLERS)?;
        let (steps, report) = if handlers.is_empty() {
            let proposal = default_factory(self.state())?;
            let (step, report) = apply_step(&mut self.history, proposal, &metadata)?;
            (vec![step], report)
        } else {
            let StagedInput {
                history,
                steps,
                report,
            } = stage_input(&self.history, change, &handlers, default_factory, &metadata)?;
            self.history = history;
            (steps, report)
        };
        let start = steps[0].start_state.clone();
        let length = start.text().len();
        self.report_transition(Some(start), report, length, Transition::Input(&steps));
        Ok(())
    }
}

struct StagedInput<M> {
    history: History<M>,
    steps: Vec<InputStep>,
    report: UpdateReport,
}

fn stage_input<M: Clone>(
    history: &History<M>,
    change: InputChange<'_>,
    handlers: &[HeadlessHandler],
    build: impl FnOnce(&EditorState) -> Result<Transaction, EditError>,
    metadata: &M,
) -> Result<StagedInput<M>, EditError> {
    let stage_first = |proposal| -> Result<_, EditError> {
        let mut staged = history.staging_copy();
        let (step, report) = apply_step(&mut staged, proposal, metadata)?;
        Ok((staged, step, report))
    };
    let factory = Cell::new(Some(build));
    let default = OnceCell::new();
    let default_insert = || {
        default
            .get_or_init(|| {
                let proposal = factory.take().expect("one default per operation")(change.state())?;
                stage_first(proposal)
            })
            .as_ref()
            .map(|(_, step, _)| step)
            .map_err(|error| *error)
    };
    let mut proposal = None;
    for handler in handlers {
        match handler {
            HeadlessHandler::Replacement(handler) => {
                if let Some(replacement) = handler(change)? {
                    // Start from the original history even if a completion handler
                    // requested the default before passing.
                    let (history, step, report) = stage_first(replacement)?;
                    return Ok(StagedInput {
                        history,
                        steps: vec![step],
                        report,
                    });
                }
            }
            HeadlessHandler::Completion(handler) => {
                if let Some(completion) = handler(change, &default_insert)? {
                    proposal = Some(completion);
                    break;
                }
            }
        }
    }
    default_insert()?;
    let (mut history, first, mut report) = default.into_inner().expect("default was built")?;
    let length = first.start_state.text().len();
    let mut steps = vec![first];
    if let Some(mut proposal) = proposal {
        proposal.annotations.insert(
            0,
            Transaction::user_event_annotation().of("input.complete".to_owned()),
        );
        let (step, completion) = apply_step(&mut history, proposal, metadata)?;
        if let Some(changes) = &completion.changes {
            let desc = changes.desc(step.start_state.text().len())?;
            report.effects = StateEffect::map_all(&report.effects, &desc);
        }
        report.effects.extend(completion.effects);
        report.selection_set |= completion.selection_set;
        report.scroll_into_view |= completion.scroll_into_view;
        report.changes = match (report.changes, completion.changes) {
            (Some(first), Some(second)) => Some(first.compose(&second, length)?),
            (first, second) => first.or(second),
        };
        steps.push(step);
    }
    Ok(StagedInput {
        history,
        steps,
        report,
    })
}

fn apply_step<M: Clone>(
    history: &mut History<M>,
    proposal: Transaction,
    metadata: &M,
) -> Result<(InputStep, UpdateReport), EditError> {
    let start_state = history.state().clone();
    let mut transaction = start_state.filter_transaction(proposal)?;
    let report = history.update_resolved_input_step(transaction.clone(), metadata)?;
    transaction.changes = report.changes.clone().unwrap_or_default();
    Ok((
        InputStep {
            start_state,
            state: history.state().clone(),
            transaction,
        },
        report,
    ))
}

fn typed(transaction: Transaction) -> Transaction {
    transaction
        .with_user_event("input.type")
        .with_scroll_into_view()
}

fn process(
    change: InputChange<'_>,
    build: impl FnOnce() -> Result<Transaction, EditError>,
    use_default: bool,
) -> Result<InputHandling, EditError> {
    let factory = Cell::new(Some(build));
    let default = OnceCell::new();
    let default_insert = || {
        default
            .get_or_init(|| {
                let transaction = factory.take().expect("one default per operation")()?;
                check_transaction(change.state(), &transaction)?;
                Ok(transaction)
            })
            .as_ref()
            .map_err(|error| *error)
    };
    for handler in change.state().context().facet(&INPUT_HANDLERS)?.iter() {
        if let InputHandling::Handled(transaction) = handler(change, &default_insert) {
            if let Some(transaction) = &transaction {
                check_transaction(change.state(), transaction)?;
            }
            return Ok(InputHandling::Handled(transaction));
        }
    }
    if !use_default {
        return Ok(InputHandling::Pass);
    }
    default_insert()?;
    let transaction = default.into_inner().expect("default was built")?;
    Ok(InputHandling::Handled(Some(transaction)))
}

fn check_transaction(state: &EditorState, transaction: &Transaction) -> Result<(), EditError> {
    let changes = transaction.changes();
    changes.check(state.text())?;
    if let Some(selection) = transaction.selection() {
        selection.check(|pos| changes.is_boundary_after(state.text(), pos))?;
    }
    Ok(())
}
