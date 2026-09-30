//! Chat-history records: one self-contained file per bot reply.

use serenity::all::MessageId;
use snafu::ResultExt as _;
use tracing::warn;

use super::store::read_json;
use super::{Database, DatabaseError, GetSnafu, InsertSnafu};
use crate::models::history::{History, StoredHistory};

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the chat-history record operations are split into this child module to separate them from the other record types"
)]
impl Database {
    /// Returns a chat history by its ID, embedding its messages.
    pub async fn history<T: Into<MessageId>>(
        &self,
        id: T,
    ) -> Result<Option<History>, DatabaseError> {
        let path = self.chat_path(&id.into().to_string());
        let Some(stored) = read_json::<StoredHistory>(&path).await.context(GetSnafu)? else {
            return Ok(None);
        };
        let history_id = stored.id.clone();
        let Some(history) = History::hydrate(stored) else {
            warn!("history {history_id} has no resolvable choices");
            return Ok(None);
        };
        Ok(Some(history))
    }

    /// Updates or inserts a chat history, storing the full messages inline in a single file.
    ///
    /// Each handler loads its own [`History`], mutates it, and saves it here separately, so two
    /// near-simultaneous button presses on the same reply race and the last write wins (a lost
    /// swipe/edit). This is accepted: there is no per-conversation lock or version guard, because the
    /// bot serves a handful of users and the worst case is a single dropped mutation, never
    /// corruption (every write is a whole, valid record renamed into place atomically).
    pub async fn upsert_history(&self, history: History) -> Result<(), DatabaseError> {
        let stored = history.into_stored();
        self.write_json(&self.chat_path(&stored.id), &stored)
            .await
            .context(InsertSnafu)
    }
}
