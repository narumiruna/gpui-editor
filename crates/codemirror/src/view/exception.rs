use std::sync::{Arc, LazyLock};

use crate::{EditorState, Extension, Facet};

/// An error from a native view callback that cannot return it to the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewException {
    pub message: String,
    pub context: Option<&'static str>,
}

impl ViewException {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            context: None,
        }
    }

    pub fn with_context(mut self, context: &'static str) -> Self {
        self.context = Some(context);
        self
    }
}

type Sink = Arc<dyn Fn(&ViewException) + Send + Sync>;
static EXCEPTION_SINKS: LazyLock<Facet<Sink, Vec<Sink>>> = LazyLock::new(Facet::collect);

/// Register a native view error sink. The first configured sink receives
/// errors. Use extension precedence to select a sink.
pub fn exception_sink(sink: impl Fn(&ViewException) + Send + Sync + 'static) -> Extension {
    EXCEPTION_SINKS.of(Arc::new(sink))
}

/// Report a callback failure. Return `false` when no sink is installed.
/// There is no browser or console fallback in the native core.
pub fn log_exception(state: &EditorState, exception: &ViewException) -> bool {
    let Ok(sinks) = state.context().facet(&EXCEPTION_SINKS) else {
        return false;
    };
    let Some(sink) = sinks.first() else {
        return false;
    };
    sink(exception);
    true
}
