//! Input interception from `@codemirror/view` 6.44.0.
//! See `docs/licenses/codemirror-view.txt`.
//!
//! The host detects input and decides whether to offer it to handlers. This module
//! does not detect DOM changes, selection-only events, atomic ranges, or IME changes.
//! It does not apply transactions, run transaction filters, or record history.

use std::{
    cell::{Cell, OnceCell},
    ops::Range,
    sync::{Arc, LazyLock},
};

use crate::{
    Change, ChangeSet, EditError, EditorState, Extension, Facet, SelectionRange, Transaction,
};

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
