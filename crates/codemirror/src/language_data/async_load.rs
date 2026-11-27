//! Shared native futures for the `LanguageDescription.load` lifecycle.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
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
/// Unlike a JavaScript promise, the loader runs only when a caller polls its
/// future. The host owns that scheduling. Dropping a waiter does not cancel a
/// load while its description keeps the attempt.
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

    /// Join the current attempt or create a new one. No asynchronous work runs
    /// until a caller polls the returned future.
    pub fn load(&self) -> LanguageLoadFuture<'a, T, E> {
        let mut state = self.state.lock().expect("language load state not poisoned");
        let attempt = state.attempt.get_or_insert_with(|| self.start_attempt());
        LanguageLoadFuture {
            identity: attempt.identity.clone(),
            load: attempt.load.clone(),
        }
    }

    fn start_attempt(&self) -> Attempt<'a, T, E> {
        let identity = Arc::new(());
        let current = identity.clone();
        let state = Arc::downgrade(&self.state);
        let loader = self.loader.clone();
        let description = self.description;
        let load = async move {
            let result = loader(description).await;
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

/// A waiter for one loading attempt. Like other futures, poll it only until it
/// returns `Ready`. Read the cached outcome through the description after that.
#[must_use = "poll the future to load the language"]
pub struct LanguageLoadFuture<'a, T, E> {
    identity: Arc<()>,
    load: SharedLoad<'a, T, E>,
}

impl<T, E> LanguageLoadFuture<'_, T, E> {
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
