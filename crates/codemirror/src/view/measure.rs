//! Host-drained view measurements. Request order follows CodeMirror view 6.44.0.
//! See `docs/licenses/codemirror-view.txt`.
//!
//! The host owns the queue and decides when to drain it. The queue does not schedule
//! frames, read layout, run view plugins, or request a layout pass without callbacks.
//! Before each round the host can prepare layout or defer the round. Pending requests
//! are not tied to a document revision. Cancel stale pending requests explicitly. If an
//! earlier write can invalidate a result in the active round, check it in the later write.

use std::{fmt, mem, rc::Rc};

use super::ViewException;

/// Maximum number of read/write rounds in one drain, as in the pinned measure loop.
pub const MAX_MEASURE_ROUNDS: usize = 6;
/// Native limit on pending requests, separate from the active round.
pub const MAX_PENDING_MEASURES: usize = 1024;

/// A host-defined key within one queue.
pub type MeasureKey = u64;

type Write<H> = Box<dyn FnOnce(&mut H, &mut MeasureRequests<H>) -> Result<(), ViewException>>;
type Read<H> = dyn Fn(&H, &mut MeasureRequests<H>) -> Result<Option<Write<H>>, ViewException>;

/// A reusable request for a single host thread. Clones have the same identity.
pub struct MeasureRequest<H> {
    key: Option<MeasureKey>,
    read: Rc<Read<H>>,
}

impl<H> Clone for MeasureRequest<H> {
    fn clone(&self) -> Self {
        Self {
            key: self.key,
            read: Rc::clone(&self.read),
        }
    }
}

impl<H> MeasureRequest<H> {
    /// Pair a read result with its write. The read must not change host layout,
    /// and the write must not perform layout reads. Both can enqueue another request.
    pub fn new<T: 'static>(
        key: Option<MeasureKey>,
        read: impl Fn(&H, &mut MeasureRequests<H>) -> Result<T, ViewException> + 'static,
        write: impl Fn(T, &mut H, &mut MeasureRequests<H>) -> Result<(), ViewException> + 'static,
    ) -> Self {
        let write = Rc::new(write);
        Self {
            key,
            read: Rc::new(move |host, pending| {
                let value = read(host, pending)?;
                let write = Rc::clone(&write);
                Ok(Some(Box::new(move |host, pending| {
                    write(value, host, pending)
                })))
            }),
        }
    }

    /// Request a read without a corresponding write.
    pub fn read_only(
        key: Option<MeasureKey>,
        read: impl Fn(&H, &mut MeasureRequests<H>) -> Result<(), ViewException> + 'static,
    ) -> Self {
        Self {
            key,
            read: Rc::new(move |host, pending| read(host, pending).map(|()| None)),
        }
    }
}

/// A new request could not be added. Existing requests are unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeasureQueueFull;

impl fmt::Display for MeasureQueueFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "measurement queue limit ({MAX_PENDING_MEASURES}) reached"
        )
    }
}

impl std::error::Error for MeasureQueueFull {}

/// Access to pending requests only. Callbacks cannot drain or change the active round.
pub struct MeasureRequests<H> {
    entries: Vec<MeasureRequest<H>>,
}

impl<H> MeasureRequests<H> {
    /// Ignore a duplicate of the same request. Otherwise, replace a request with
    /// the same key in its original position or add the request at the end.
    /// Duplicates and replacements are accepted even at the pending limit.
    pub fn request(&mut self, request: &MeasureRequest<H>) -> Result<(), MeasureQueueFull> {
        if self
            .entries
            .iter()
            .any(|old| Rc::ptr_eq(&old.read, &request.read))
        {
            return Ok(());
        }
        if let Some(key) = request.key
            && let Some(old) = self.entries.iter_mut().find(|old| old.key == Some(key))
        {
            *old = request.clone();
            return Ok(());
        }
        if self.entries.len() == MAX_PENDING_MEASURES {
            return Err(MeasureQueueFull);
        }
        self.entries.push(request.clone());
        Ok(())
    }

    /// Cancel the pending request with this key. An active round is not affected.
    pub fn cancel(&mut self, key: MeasureKey) -> bool {
        let Some(index) = self.entries.iter().position(|entry| entry.key == Some(key)) else {
            return false;
        };
        self.entries.remove(index);
        true
    }

    /// Discard all pending requests, for example when the host replaces its layout.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// The result of an explicit host drain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeasureDrain {
    /// Number of batches taken from the queue.
    pub rounds: usize,
    /// Work remains because the host deferred a round or the drain reached its limit.
    pub pending: bool,
}

/// An ordered measurement queue. Drop it when its host is destroyed.
pub struct MeasureQueue<H> {
    pending: MeasureRequests<H>,
}

impl<H> Default for MeasureQueue<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H> MeasureQueue<H> {
    pub fn new() -> Self {
        Self {
            pending: MeasureRequests {
                entries: Vec::new(),
            },
        }
    }

    /// Submit or cancel pending requests between drains.
    pub fn requests(&mut self) -> &mut MeasureRequests<H> {
        &mut self.pending
    }

    /// Check if the host has work to drain. The host can coalesce wake-ups using this flag.
    pub fn has_pending(&self) -> bool {
        !self.pending.entries.is_empty()
    }

    /// Drain up to [`MAX_MEASURE_ROUNDS`] rounds. The host's `prepare` callback runs
    /// before each nonempty round. Return `false` to leave work queued when layout
    /// is not ready; this drain stops instead of trying to stabilize layout itself.
    ///
    /// Each round takes a snapshot of pending requests, then runs all reads before
    /// any writes. Requests from either phase go into a later round. A read error
    /// skips its write. The queue reports errors to `on_error` in the phase where
    /// they occur. Panics are not caught. The limits do not bound callback run time.
    pub fn drain(
        &mut self,
        host: &mut H,
        mut prepare: impl FnMut(&mut H) -> bool,
        mut on_error: impl FnMut(&H, ViewException),
    ) -> MeasureDrain {
        let mut rounds = 0;
        while rounds < MAX_MEASURE_ROUNDS && self.has_pending() && prepare(host) {
            let entries = mem::take(&mut self.pending.entries);
            let mut writes = Vec::with_capacity(entries.len());
            for request in entries {
                match (request.read)(host, &mut self.pending) {
                    Ok(Some(write)) => writes.push(write),
                    Ok(None) => (),
                    Err(error) => on_error(host, error),
                }
            }
            for write in writes {
                if let Err(error) = write(host, &mut self.pending) {
                    on_error(host, error);
                }
            }
            rounds += 1;
        }
        MeasureDrain {
            rounds,
            pending: self.has_pending(),
        }
    }
}
