//! The registry of in-flight reply streams that the user can stop.
//!
//! A reply streams on one spawned task while the Stop button press arrives on
//! another, so the two are bridged by a shared [`MessageId`]-keyed table of
//! [`CancellationToken`]s. The streaming task registers a token via
//! [`begin`](Cancellations::begin) (holding the returned [`StreamGuard`] for the
//! stream's lifetime, so the entry is removed once streaming ends) and selects
//! on its `cancelled()` future; the Stop handler looks the token up by message
//! ID and [`cancel`](Cancellations::cancel)s it.

use poise::serenity_prelude::MessageId;
use std::collections::HashMap;
use tokio_util::sync::CancellationToken;

// under the `loom` feature the table is guarded by loom's instrumented mutex,
// so a loom model can permute the interleavings of a register against a
// cancel. the rest of the crate uses the standard one
#[cfg(feature = "loom")]
use loom::sync::{Mutex, MutexGuard};
#[cfg(not(feature = "loom"))]
use std::sync::{Mutex, MutexGuard};
// loom re-exports `LockResult` but not `PoisonError`, and both mutexes poison
// through the same standard-library type
use std::sync::PoisonError;

/// A registry mapping a streaming reply's Discord message ID to the token that
/// stops it.
#[derive(Debug, Default)]
pub struct Cancellations {
    /// The live stream tokens, keyed by the streamed reply's message ID.
    tokens: Mutex<HashMap<MessageId, CancellationToken>>,
}

impl Cancellations {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a fresh token for `id` and returns a guard that removes the
    /// entry when dropped. The guard's [`token`](StreamGuard::token) is the one
    /// the streaming loop selects on.
    pub fn begin(&self, id: MessageId) -> StreamGuard<'_> {
        let token = CancellationToken::new();
        self.lock().insert(id, token.clone());
        StreamGuard {
            registry: self,
            id,
            token,
        }
    }

    /// Cancels the in-flight stream for `id`, if one is registered.
    pub fn cancel(&self, id: MessageId) {
        if let Some(token) = self.lock().get(&id) {
            token.cancel();
        }
    }

    /// Cancels every in-flight stream. Used at shutdown so each reply breaks at
    /// its next select and persists what it has, rather than being aborted
    /// mid-write when the runtime is dropped.
    pub fn cancel_all(&self) {
        let tokens: Vec<CancellationToken> = self.lock().values().cloned().collect();
        for token in tokens {
            token.cancel();
        }
    }

    /// Removes the entry for `id`; called by [`StreamGuard`] on drop.
    fn remove(&self, id: MessageId) {
        self.lock().remove(&id);
    }

    /// Locks the token table, recovering the guard if a previous holder
    /// panicked. The guarded map operations cannot themselves panic, so the map
    /// is always consistent and a poisoned lock is safe to keep using; silently
    /// giving up here would instead permanently disable the Stop button.
    fn lock(&self) -> MutexGuard<'_, HashMap<MessageId, CancellationToken>> {
        self.tokens.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Removes its registry entry when dropped, so a finished or failed stream
/// leaves no token behind. Hands the streaming loop its [`CancellationToken`].
#[derive(Debug)]
pub struct StreamGuard<'a> {
    /// The registry to remove the entry from on drop.
    registry: &'a Cancellations,
    /// The message ID whose entry this guard owns.
    id: MessageId,
    /// The token the streaming loop awaits to learn it was stopped.
    token: CancellationToken,
}

impl StreamGuard<'_> {
    /// The token the streaming loop awaits to learn the reply was stopped.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }
}

impl Drop for StreamGuard<'_> {
    fn drop(&mut self) {
        self.registry.remove(self.id);
    }
}

/// Tests for the stop-token registry.
#[cfg(test)]
mod tests {
    use super::Cancellations;
    // the single-threaded tests below are the only users of `MessageId` when the
    // `loom` feature is off, since the loom models import their own
    #[cfg(not(feature = "loom"))]
    use poise::serenity_prelude::MessageId;

    /// Cancelling a registered stream cancels the token the loop holds.
    #[cfg(not(feature = "loom"))]
    #[test]
    fn cancel_cancels_the_registered_token() {
        let registry = Cancellations::new();
        let id = MessageId::new(1);
        let guard = registry.begin(id);
        let token = guard.token();
        assert!(
            !token.is_cancelled(),
            "a freshly registered stream token starts uncancelled"
        );
        registry.cancel(id);
        assert!(
            token.is_cancelled(),
            "cancelling the stream cancels the token the loop holds"
        );
    }

    /// Dropping the guard removes the entry, so a later stream on the same ID
    /// gets an independent token and the dropped one is never cancelled.
    #[cfg(not(feature = "loom"))]
    #[test]
    fn dropping_the_guard_removes_the_entry() {
        let registry = Cancellations::new();
        let id = MessageId::new(1);
        let first = registry.begin(id);
        let old_token = first.token();
        drop(first);
        let second = registry.begin(id);
        let new_token = second.token();
        registry.cancel(id);
        assert!(
            new_token.is_cancelled(),
            "the current stream's token is the one cancelled"
        );
        assert!(
            !old_token.is_cancelled(),
            "the dropped stream's token is left untouched"
        );
    }

    /// Cancelling an unknown ID leaves an unrelated registered stream untouched
    /// rather than cancelling it (or panicking).
    #[cfg(not(feature = "loom"))]
    #[test]
    fn cancel_unknown_id_leaves_other_streams_alone() {
        let registry = Cancellations::new();
        let guard = registry.begin(MessageId::new(1));
        let token = guard.token();
        registry.cancel(MessageId::new(999));
        assert!(
            !token.is_cancelled(),
            "cancelling an unknown id does not touch a registered stream"
        );
    }

    /// `cancel_all` cancels every registered stream, as at shutdown.
    #[cfg(not(feature = "loom"))]
    #[test]
    fn cancel_all_cancels_every_registered_stream() {
        let registry = Cancellations::new();
        let first = registry.begin(MessageId::new(1));
        let second = registry.begin(MessageId::new(2));
        let first_token = first.token();
        let second_token = second.token();
        registry.cancel_all();
        assert!(
            first_token.is_cancelled(),
            "the first stream is cancelled at shutdown"
        );
        assert!(
            second_token.is_cancelled(),
            "the second stream is cancelled at shutdown"
        );
    }

    /// The interleavings a register and a cancel can reach against one shared
    /// table, permuted exhaustively by loom. The stream side runs on the task
    /// that owns the reply and the stop side on the task that pressed the
    /// button, so the only thing ordering them is the table's lock.
    ///
    /// Loom replaces the table's mutex under the `loom` feature (see the
    /// imports), so these models explore schedules the single-threaded tests
    /// above cannot reach. The tokens themselves stay real, since loom only
    /// instruments the primitives it is handed.
    ///
    /// Each model leaks its registry so a guard taken on one loom thread can be
    /// parked for the whole model, which is what a stream still in flight looks
    /// like. Releasing a guard unregisters its entry, so dropping it early would
    /// leave the stop press nothing to cancel, which is the guard's purpose and
    /// not a race worth modelling.
    #[cfg(feature = "loom")]
    mod loom {
        use super::Cancellations;
        use loom::sync::atomic::{AtomicBool, Ordering};
        use loom::sync::{Arc, Mutex};
        use loom::thread;
        use poise::serenity_prelude::MessageId;
        use tokio_util::sync::CancellationToken;

        /// A registry that outlives every loom thread, so a guard one thread
        /// registers can be read by another.
        fn leaked_registry() -> &'static Cancellations {
            Box::leak(Box::new(Cancellations::new()))
        }

        /// Locks a loom mutex, which cannot be poisoned inside a model because
        /// a panicking thread aborts the whole model rather than unwinding.
        fn lock<T>(mutex: &Mutex<T>) -> loom::sync::MutexGuard<'_, T> {
            mutex
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        }

        /// A stop press that runs after a registration has begun reaches the token
        /// that registration left in the table.
        ///
        /// The two tasks are deliberately unordered, so loom also explores the
        /// schedule where the stop wins the race and finds nothing to cancel.
        /// The release/acquire pair is what makes the interesting half of that
        /// space assertable: when the stop's own load observed the flag, the
        /// registration is known to have stored its entry, so the stop must
        /// cancel that exact token.
        #[test]
        fn cancel_racing_registration_reaches_the_live_token() {
            loom::model(|| {
                let registry = leaked_registry();
                let id = MessageId::new(1);
                let slots: &Mutex<Vec<CancellationToken>> =
                    Box::leak(Box::new(Mutex::new(Vec::new())));
                let registered = Arc::new(AtomicBool::new(false));
                let stopped = Arc::new(AtomicBool::new(false));

                let streaming = {
                    let slots = &*slots;
                    let registered = Arc::clone(&registered);
                    thread::spawn(move || {
                        let guard = registry.begin(id);
                        lock(slots).push(guard.token());
                        registered.store(true, Ordering::Release);
                        // the guard is leaked on purpose: the stream is still in
                        // flight, so its entry has to stay in the table
                        Box::leak(Box::new(guard));
                    })
                };
                let stopping = {
                    let registered = Arc::clone(&registered);
                    let stopped = Arc::clone(&stopped);
                    thread::spawn(move || {
                        if registered.load(Ordering::Acquire) {
                            registry.cancel(id);
                            stopped.store(true, Ordering::Release);
                        }
                    })
                };

                streaming.join().ok().expect("the streaming task finishes");
                stopping.join().ok().expect("the stop task finishes");

                let tokens = lock(slots);
                assert_eq!(
                    tokens.len(),
                    1,
                    "the streaming task registered exactly one token"
                );
                // when the stop acted, the release/acquire pair guarantees the
                // entry was already stored, so the token is cancelled; when it
                // did not, the stop declined and the token is untouched. either
                // way only the stream's own token is involved
                assert_eq!(
                    tokens.first().is_some_and(CancellationToken::is_cancelled),
                    stopped.load(Ordering::Acquire),
                    "the stop cancels the stream's own token exactly when it ran after registration"
                );
            });
        }

        /// Cancelling an id no stream has claimed leaves a live stream's token
        /// alone, even when the two run concurrently.
        #[test]
        fn cancel_of_an_unclaimed_id_spares_a_live_stream() {
            loom::model(|| {
                let registry = leaked_registry();
                let claimed = MessageId::new(1);
                let stranger = MessageId::new(999);
                let slots: &Mutex<Vec<CancellationToken>> =
                    Box::leak(Box::new(Mutex::new(Vec::new())));

                let streaming = {
                    let slots = &*slots;
                    thread::spawn(move || {
                        let guard = registry.begin(claimed);
                        lock(slots).push(guard.token());
                        Box::leak(Box::new(guard));
                    })
                };
                let stopping = thread::spawn(move || registry.cancel(stranger));

                streaming.join().ok().expect("the streaming task finishes");
                stopping.join().ok().expect("the stop task finishes");

                let tokens = lock(slots);
                assert!(
                    tokens.first().is_some_and(|token| !token.is_cancelled()),
                    "a stop for an id nobody claimed leaves the live stream running"
                );
            });
        }

        /// Shutdown cancels every stream that registered before the sweep began.
        ///
        /// Each stream publishes its own registration flag, and the sweep
        /// publishes one of its own, so "the sweep ran" is read from the sweep
        /// rather than inferred from a flag a different thread wrote.
        #[test]
        fn cancel_all_reaches_every_registered_stream() {
            loom::model(|| {
                let registry = leaked_registry();
                let slots: &Mutex<Vec<CancellationToken>> =
                    Box::leak(Box::new(Mutex::new(Vec::new())));
                let first_stored = Arc::new(AtomicBool::new(false));
                let second_stored = Arc::new(AtomicBool::new(false));
                let swept = Arc::new(AtomicBool::new(false));

                let first = {
                    let slots = &*slots;
                    let first_stored = Arc::clone(&first_stored);
                    thread::spawn(move || {
                        let guard = registry.begin(MessageId::new(1));
                        lock(slots).push(guard.token());
                        first_stored.store(true, Ordering::Release);
                        Box::leak(Box::new(guard));
                    })
                };
                let second = {
                    let slots = &*slots;
                    let second_stored = Arc::clone(&second_stored);
                    thread::spawn(move || {
                        let guard = registry.begin(MessageId::new(2));
                        lock(slots).push(guard.token());
                        second_stored.store(true, Ordering::Release);
                        Box::leak(Box::new(guard));
                    })
                };
                let shutdown = {
                    let first_stored = Arc::clone(&first_stored);
                    let second_stored = Arc::clone(&second_stored);
                    let swept = Arc::clone(&swept);
                    thread::spawn(move || {
                        // only sweep once both entries are known to be stored,
                        // so "cancels everything" has something to be true about
                        if first_stored.load(Ordering::Acquire)
                            && second_stored.load(Ordering::Acquire)
                        {
                            registry.cancel_all();
                            swept.store(true, Ordering::Release);
                        }
                    })
                };

                first.join().ok().expect("the first stream finishes");
                second.join().ok().expect("the second stream finishes");
                shutdown.join().ok().expect("the shutdown sweep finishes");

                let tokens = lock(slots);
                assert_eq!(tokens.len(), 2, "both streaming tasks registered a token");
                // a sweep that observed both flags saw both entries in the
                // table, so both tokens are cancelled; a sweep that declined
                // leaves both running
                assert_eq!(
                    tokens.iter().all(CancellationToken::is_cancelled),
                    swept.load(Ordering::Acquire),
                    "the sweep cancels every stream registered before it ran"
                );
            });
        }
    }
}
