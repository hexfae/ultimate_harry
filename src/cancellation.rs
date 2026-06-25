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
use std::sync::{Mutex, MutexGuard, PoisonError};
use tokio_util::sync::CancellationToken;

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
    use poise::serenity_prelude::MessageId;

    /// Cancelling a registered stream cancels the token the loop holds.
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
}
