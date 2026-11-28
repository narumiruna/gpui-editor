//! Shared native futures for the `LanguageDescription.load` lifecycle.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Waker},
};

use futures_util::future::{BoxFuture, FutureExt, Shared};

use super::LanguageDescription;

type Outcome<T, E> = Result<Arc<T>, Arc<E>>;
type SharedLoad<'a, T, E> = Shared<BoxFuture<'a, Outcome<T, E>>>;
type Factory<'a, T, E> =
    dyn Fn(&'a LanguageDescription) -> BoxFuture<'a, Outcome<T, E>> + Send + Sync + 'a;

struct Attempt<'a, T, E> {
    identity: Arc<()>,
    load: SharedLoad<'a, T, E>,
}

struct LoadState<'a, T, E> {
    attempt: Option<Attempt<'a, T, E>>,
    support: Option<Arc<T>>,
}

/// Loading state for one description. One attempt can have many waiters.
/// A successful attempt remains cached. Failure permits a new attempt without
/// changing the result for existing waiters.
///
/// Like a JavaScript promise factory, the loader is invoked on the first
/// load request. The host schedules polling of the returned Rust future.
/// Dropping a waiter does not cancel the attempt while the description exists.
pub struct NativeAsyncLanguageLoad<'a, T, E> {
    description: &'a LanguageDescription,
    loader: Arc<Factory<'a, T, E>>,
    state: Arc<Mutex<LoadState<'a, T, E>>>,
}

impl<'a, T, E> NativeAsyncLanguageLoad<'a, T, E>
where
    T: Send + Sync + 'a,
    E: Send + Sync + 'a,
{
    pub fn new<F>(
        description: &'a LanguageDescription,
        loader: impl Fn(&'a LanguageDescription) -> F + Send + Sync + 'a,
    ) -> Self
    where
        F: Future<Output = Result<T, E>> + Send + 'a,
    {
        Self::with_loader(description, None, loader)
    }

    /// Wrap already loaded support. `support` returns it before the first poll.
    pub fn from_support(description: &'a LanguageDescription, support: T) -> Self {
        let support = Arc::new(support);
        Self::with_factory(description, Some(support.clone()), move |_| {
            std::future::ready(Ok(support.clone())).boxed()
        })
    }

    /// Keep initial support available until the loader succeeds. A failed
    /// attempt leaves that support in place and permits a retry.
    pub fn with_support<F>(
        description: &'a LanguageDescription,
        support: T,
        loader: impl Fn(&'a LanguageDescription) -> F + Send + Sync + 'a,
    ) -> Self
    where
        F: Future<Output = Result<T, E>> + Send + 'a,
    {
        Self::with_loader(description, Some(Arc::new(support)), loader)
    }

    fn with_loader<F>(
        description: &'a LanguageDescription,
        support: Option<Arc<T>>,
        loader: impl Fn(&'a LanguageDescription) -> F + Send + Sync + 'a,
    ) -> Self
    where
        F: Future<Output = Result<T, E>> + Send + 'a,
    {
        Self::with_factory(description, support, move |d| {
            loader(d)
                .map(|result| result.map(Arc::new).map_err(Arc::new))
                .boxed()
        })
    }

    fn with_factory(
        description: &'a LanguageDescription,
        support: Option<Arc<T>>,
        loader: impl Fn(&'a LanguageDescription) -> BoxFuture<'a, Outcome<T, E>> + Send + Sync + 'a,
    ) -> Self {
        Self {
            description,
            loader: Arc::new(loader),
            state: Arc::new(Mutex::new(LoadState {
                attempt: None,
                support,
            })),
        }
    }

    pub fn description(&self) -> &'a LanguageDescription {
        self.description
    }

    /// Read the initial or loaded support without polling the loader.
    pub fn support(&self) -> Option<Arc<T>> {
        self.state
            .lock()
            .expect("language load state not poisoned")
            .support
            .clone()
    }

    /// Join the current attempt or invoke the loader to create a new one.
    pub fn load(&self) -> LanguageLoadFuture<'a, T, E> {
        let (future, slot) = {
            let mut state = self.state.lock().expect("language load state not poisoned");
            if let Some(attempt) = &state.attempt {
                return LanguageLoadFuture::from_attempt(attempt);
            }
            let slot = Arc::new(Mutex::new(Waiting {
                future: None,
                waker: None,
                panicked: false,
            }));
            let attempt = self.start_attempt(slot.clone());
            let future = LanguageLoadFuture::from_attempt(&attempt);
            state.attempt = Some(attempt);
            (future, slot)
        };
        let mut reset = ResetFactory {
            state: &self.state,
            slot: &slot,
            identity: &future.identity,
            armed: true,
        };
        let load = (self.loader)(self.description);
        reset.armed = false;
        drop(reset);
        let waiter = {
            let mut slot = slot
                .lock()
                .expect("language load factory state not poisoned");
            slot.future = Some(load);
            slot.waker.take()
        };
        if let Some(waiter) = waiter {
            waiter.wake();
        }
        future
    }

    fn start_attempt(&self, slot: Arc<Mutex<Waiting<'a, T, E>>>) -> Attempt<'a, T, E> {
        let identity = Arc::new(());
        let current = identity.clone();
        let state = Arc::downgrade(&self.state);
        let load = async move {
            let future = std::future::poll_fn(|context| {
                let mut slot = slot
                    .lock()
                    .expect("language load factory state not poisoned");
                if slot.panicked {
                    drop(slot);
                    panic!("language load factory panicked");
                }
                if let Some(load) = slot.future.take() {
                    Poll::Ready(load)
                } else {
                    slot.waker = Some(context.waker().clone());
                    Poll::Pending
                }
            })
            .await;
            let result = future.await;
            if let Some(state) = state.upgrade() {
                let replaced = {
                    let mut state = state.lock().expect("language load state not poisoned");
                    match &result {
                        Ok(support) => state.support.replace(support.clone()),
                        Err(_) => {
                            if state
                                .attempt
                                .as_ref()
                                .is_some_and(|attempt| Arc::ptr_eq(&attempt.identity, &current))
                            {
                                state.attempt = None;
                            }
                            None
                        }
                    }
                };
                // Drop caller-supplied support outside the state lock.
                drop(replaced);
            }
            result
        }
        .boxed()
        .shared();
        Attempt { identity, load }
    }
}

// A synchronous factory panic must not prevent the next load request.
struct ResetFactory<'s, 'a, T, E> {
    state: &'s Mutex<LoadState<'a, T, E>>,
    slot: &'s Mutex<Waiting<'a, T, E>>,
    identity: &'s Arc<()>,
    armed: bool,
}

impl<T, E> Drop for ResetFactory<'_, '_, T, E> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        {
            let mut state = self.state.lock().expect("language load state not poisoned");
            if state
                .attempt
                .as_ref()
                .is_some_and(|attempt| Arc::ptr_eq(&attempt.identity, self.identity))
            {
                state.attempt = None;
            }
        }
        let waiter = {
            let mut slot = self
                .slot
                .lock()
                .expect("language load factory state not poisoned");
            slot.panicked = true;
            slot.waker.take()
        };
        if let Some(waiter) = waiter {
            waiter.wake();
        }
    }
}

// Publish an attempt before calling host loader code, so other callers can
// join it without executing that code or waiting under the state lock.
struct Waiting<'a, T, E> {
    future: Option<BoxFuture<'a, Outcome<T, E>>>,
    waker: Option<Waker>,
    panicked: bool,
}

/// A waiter for one loading attempt. Like other futures, poll it only until it
/// returns `Ready`. Read the cached outcome through the description after that.
#[must_use = "poll the future to load the language"]
pub struct LanguageLoadFuture<'a, T, E> {
    identity: Arc<()>,
    load: SharedLoad<'a, T, E>,
}

impl<'a, T, E> LanguageLoadFuture<'a, T, E> {
    fn from_attempt(attempt: &Attempt<'a, T, E>) -> Self {
        Self {
            identity: attempt.identity.clone(),
            load: attempt.load.clone(),
        }
    }

    /// Whether these waiters refer to the same attempt, even after completion.
    pub fn same_attempt(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}

impl<T, E> Future for LanguageLoadFuture<'_, T, E> {
    type Output = Outcome<T, E>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        self.load.poll_unpin(context)
    }
}
