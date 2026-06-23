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
use std::sync::Mutex;
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
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.insert(id, token.clone());
        }
        StreamGuard {
            registry: self,
            id,
            token,
        }
    }

    /// Cancels the in-flight stream for `id`, if one is registered.
    pub fn cancel(&self, id: MessageId) {
        let Ok(tokens) = self.tokens.lock() else {
            return;
        };
        if let Some(token) = tokens.get(&id) {
            token.cancel();
        }
    }

    /// Removes the entry for `id`; called by [`StreamGuard`] on drop.
    fn remove(&self, id: MessageId) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(&id);
        }
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

    /// Cancelling an unknown ID is a no-op rather than a panic.
    #[test]
    fn cancel_unknown_id_is_a_noop() {
        let registry = Cancellations::new();
        registry.cancel(MessageId::new(999));
    }
}
