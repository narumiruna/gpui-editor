//! Per-description native loading, based on CodeMirror language 6.13.1.

use std::sync::{Arc, Mutex, OnceLock};

use super::LanguageDescription;

type Outcome<T, E> = Result<Arc<T>, Arc<E>>;
type Attempt<T, E> = Arc<OnceLock<Outcome<T, E>>>;
type Loader<'a, T, E> = dyn Fn(&LanguageDescription) -> Result<T, E> + Send + Sync + 'a;

/// The loading state for one language description. Keep one handle per
/// description to share its in-flight load and cache the result. Two handles
/// do not share state even if their upstream load recipes are identical.
///
/// Native loads are synchronous. Other callers wait for the current attempt;
/// a successful result is shared, and a failed attempt can be retried. A
/// loader must not call `load` recursively on its own handle.
pub struct NativeLanguageLoad<'a, T, E> {
    description: &'a LanguageDescription,
    loader: Box<Loader<'a, T, E>>,
    attempt: Mutex<Option<Attempt<T, E>>>,
}

impl<'a, T, E> NativeLanguageLoad<'a, T, E> {
    pub fn new(
        description: &'a LanguageDescription,
        loader: impl Fn(&LanguageDescription) -> Result<T, E> + Send + Sync + 'a,
    ) -> Self {
        Self {
            description,
            loader: Box::new(loader),
            attempt: Mutex::new(None),
        }
    }

    pub fn description(&self) -> &'a LanguageDescription {
        self.description
    }

    /// Return the cached support without loading or waiting for a pending load.
    pub fn support(&self) -> Option<Arc<T>> {
        let attempt = self
            .attempt
            .lock()
            .expect("language load state not poisoned");
        attempt.as_ref()?.get()?.as_ref().ok().cloned()
    }

    /// Run the loader at most once per attempt. Return the same success or
    /// error allocation to every caller that joined this attempt.
    pub fn load(&self) -> Outcome<T, E> {
        let attempt = self
            .attempt
            .lock()
            .expect("language load state not poisoned")
            .get_or_insert_with(|| Arc::new(OnceLock::new()))
            .clone();
        let result = attempt.get_or_init(|| {
            (self.loader)(self.description)
                .map(Arc::new)
                .map_err(Arc::new)
        });
        if result.is_err() {
            let mut pending = self
                .attempt
                .lock()
                .expect("language load state not poisoned");
            // A late waiter for a failed attempt must not erase a retry.
            if pending
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &attempt))
            {
                *pending = None;
            }
        }
        result.clone()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant},
    };

    use super::*;
    use crate::language_data::LANGUAGES;

    #[test]
    fn concurrent_waiters_share_the_error_and_support_can_retry() {
        let entered = Barrier::new(2);
        let release = Barrier::new(2);
        let calls = AtomicUsize::new(0);
        let handle = NativeLanguageLoad::new(&LANGUAGES[0], |_| {
            if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                entered.wait();
                release.wait();
                Err("first")
            } else {
                Ok("ready")
            }
        });
        assert!(handle.support().is_none());
        std::thread::scope(|scope| {
            let first = scope.spawn(|| handle.load().unwrap_err());
            entered.wait();
            let no_pending_support = handle.support().is_none();
            let waiting = scope.spawn(|| handle.load());
            let deadline = Instant::now() + Duration::from_secs(5);
            let joined = loop {
                let count = handle
                    .attempt
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(Arc::strong_count)
                    .unwrap();
                if count >= 3 || Instant::now() >= deadline {
                    break count >= 3;
                }
                std::thread::yield_now();
            };
            release.wait();
            let first = first.join().unwrap();
            let waiting = waiting.join().unwrap();
            assert!(no_pending_support);
            assert!(joined, "second caller did not join the first attempt");
            assert!(Arc::ptr_eq(&first, &waiting.unwrap_err()));
        });
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(handle.support().is_none());
        let cached = handle.load().unwrap();
        assert_eq!(*cached, "ready");
        assert!(Arc::ptr_eq(&cached, &handle.support().unwrap()));
        assert!(Arc::ptr_eq(&cached, &handle.load().unwrap()));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
