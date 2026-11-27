use std::{
    cell::RefCell,
    collections::BTreeMap,
    future::{Future, poll_fn},
    ops::Range,
    pin::Pin,
    rc::Rc,
    task::Poll,
};

use gpui::{BackgroundExecutor, Context, Task};
use gpui_editor_codemirror::{
    language::json::json_editor_linter,
    lint::{self, LintConfiguration, LintRun, LintSession, LintSource, LintState, force_linting},
    view::{ViewException, ViewPlugin, ViewUpdate, log_exception},
};

use super::Editor;

#[derive(Clone, Eq, PartialEq)]
pub(super) struct Scope {
    revision: u64,
    configuration: LintConfiguration,
    config_epoch: u64,
    pub(super) range: Range<usize>,
    pub(super) stock_json: bool,
}

#[derive(Default)]
struct Observation {
    attached: bool,
    config_epoch: u64,
    session: Option<LintSession>,
}

struct LintUpdates(Rc<RefCell<Observation>>, BackgroundExecutor);

impl ViewPlugin for LintUpdates {
    fn update(&mut self, update: &ViewUpdate<'_>) {
        let mut observation = self.0.borrow_mut();
        if LintConfiguration::get(update.start_state) != LintConfiguration::get(update.state) {
            // A compartment can change and then restore its old registrations before render.
            observation.config_epoch += 1;
            observation.session = None;
        } else if let Some(session) = &mut observation.session {
            session.observe_update(update, self.1.now());
        }
    }

    fn destroy(&mut self) {
        *self.0.borrow_mut() = Observation::default();
    }
}

#[derive(Default)]
pub(super) struct Runner {
    scope: Option<Scope>,
    observation: Rc<RefCell<Observation>>,
    wake: Option<Task<()>>,
    jobs: BTreeMap<u64, Task<()>>,
    next_job: u64,
}

impl Runner {
    pub(super) fn pending(&self) -> bool {
        self.wake.is_some() || !self.jobs.is_empty()
    }
}

impl Editor {
    pub(super) fn live_diagnostic_scope(&self) -> Option<Scope> {
        if !self.input_enabled || self.is_composing() || LintState::get(self.view.state()).is_none()
        {
            return None;
        }
        let fence = self.source_fence()?;
        let configuration = LintConfiguration::get(self.view.state());
        let configured = configuration.has_sources();
        let stock_json = !configured && fence.stock_json;
        (configured || stock_json).then(|| Scope {
            revision: self.view.revision(),
            config_epoch: self.diagnostics.runner.observation.borrow().config_epoch,
            configuration,
            range: fence.body,
            stock_json,
        })
    }

    pub(super) fn current_diagnostic_scope(&self) -> Option<&Scope> {
        self.diagnostics
            .runner
            .scope
            .as_ref()
            .filter(|scope| self.live_diagnostic_scope().as_ref() == Some(*scope))
    }

    pub(in crate::editor) fn sync_source_diagnostics(&mut self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        let observation = self.diagnostics.runner.observation.clone();
        if !observation.borrow().attached {
            self.view
                .add_plugin(LintUpdates(observation.clone(), executor.clone()));
            observation.borrow_mut().attached = true;
        }
        let scope = self.live_diagnostic_scope();
        if self.diagnostics.runner.scope != scope {
            self.dismiss_keyboard_diagnostic_hover();
            let runner = &mut self.diagnostics.runner;
            runner.wake = None;
            runner.jobs.clear();
            observation.borrow_mut().session = None;
            runner.scope = scope;
            // Do not keep marks mapped from a prior draft or configuration.
            if let Some(run) = LintRun::start(self.view.state()) {
                self.apply_diagnostic_update(run.discard());
            }
            if let Some(scope) = &self.diagnostics.runner.scope {
                let mut session = if scope.stock_json {
                    LintSession::with_fallback(json_source(scope.range.clone()))
                } else {
                    LintSession::default()
                };
                session.observe(self.view.state(), executor.now());
                observation.borrow_mut().session = Some(session);
            } else if let Some(close) = lint::close_lint_panel(self.view.state()) {
                self.apply_diagnostic_update(close);
            }
            self.restore_diagnostic_focus(cx);
        }
        self.schedule_diagnostic_wake(cx);
    }

    fn schedule_diagnostic_wake(&mut self, cx: &mut Context<Self>) {
        let runner = &mut self.diagnostics.runner;
        if runner.wake.is_some() {
            return;
        }
        let executor = cx.background_executor();
        let wait = runner
            .observation
            .borrow()
            .session
            .as_ref()
            .and_then(|session| session.wait_time(executor.now()));
        if let Some(wait) = wait {
            let timer = executor.timer(wait);
            runner.wake = Some(cx.spawn(async move |editor, cx| {
                timer.await;
                let _ = editor.update(cx, |editor, cx| editor.run_diagnostics(false, cx));
            }));
        }
    }

    pub(super) fn run_diagnostics(&mut self, force: bool, cx: &mut Context<Self>) {
        self.diagnostics.runner.wake = None;
        let Some(scope) = self.current_diagnostic_scope().cloned() else {
            return;
        };
        let executor = cx.background_executor().clone();
        let now = executor.now();
        // Scope gates provider calls and publication, not the callback's view of the state.
        // Native sources take the full immutable document with absolute byte offsets.
        let work = self
            .diagnostics
            .runner
            .observation
            .borrow_mut()
            .session
            .as_mut()
            .and_then(|session| {
                if force {
                    force_linting(session, self.view.state(), now)
                } else {
                    session.take_due(self.view.state(), now)
                }
            });
        self.schedule_diagnostic_wake(cx);
        let Some((mut batch, futures)) = work else {
            return;
        };
        batch.restrict_to(scope.range.clone());
        let runner = &mut self.diagnostics.runner;
        let id = runner.next_job;
        runner.next_job += 1;
        runner.jobs.insert(
            id,
            cx.spawn(async move |editor, cx| {
                let mut workers: Vec<_> = futures
                    .into_iter()
                    .enumerate()
                    .map(|(index, future)| (index, executor.spawn(future)))
                    .collect();
                while !workers.is_empty() || batch.wait_time(executor.now()).is_some() {
                    let mut timer = batch
                        .wait_time(executor.now())
                        .map(|wait| executor.timer(wait));
                    let ready = poll_fn(|cx| {
                        if timer
                            .as_mut()
                            .is_some_and(|timer| Pin::new(timer).poll(cx).is_ready())
                        {
                            return Poll::Ready(None);
                        }
                        for (i, (index, worker)) in workers.iter_mut().enumerate() {
                            if let Poll::Ready(result) = Pin::new(worker).poll(cx) {
                                return Poll::Ready(Some((i, *index, result)));
                            }
                        }
                        Poll::Pending
                    })
                    .await;
                    let now = executor.now();
                    let publication = if let Some((i, index, result)) = ready {
                        drop(workers.remove(i));
                        batch.complete(index, result, now)
                    } else {
                        Ok(batch.take_due(now))
                    };
                    if !editor
                        .update(cx, |editor, cx| {
                            if editor.current_diagnostic_scope() != Some(&scope) {
                                return false;
                            }
                            match publication {
                                Ok(Some(transaction)) => {
                                    editor.apply_diagnostic_update(transaction);
                                    editor.restore_diagnostic_focus(cx);
                                    cx.notify();
                                }
                                Ok(None) => {}
                                Err(error) => {
                                    log_exception(editor.view.state(), &ViewException::new(error));
                                }
                            }
                            true
                        })
                        .unwrap_or(false)
                    {
                        break;
                    }
                }
                let _ = editor.update(cx, |editor, cx| {
                    editor.diagnostics.runner.jobs.remove(&id);
                    cx.notify();
                });
            }),
        );
    }
}

fn json_source(range: Range<usize>) -> LintSource {
    LintSource::asynchronous(move |state| {
        let range = range.clone();
        let text = state.text().clone();
        async move {
            let mut diagnostics = text
                .slice_owned(range.clone())
                .map_or_else(Vec::new, |text| json_editor_linter(&text));
            for diagnostic in &mut diagnostics {
                diagnostic.range =
                    range.start + diagnostic.range.start..range.start + diagnostic.range.end;
                diagnostic.source = Some("JSON".into());
            }
            Ok(diagnostics)
        }
    })
}
