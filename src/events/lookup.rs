//! Shared lookup resolving a conversation and its character from a message ID,
//! used by both the message and interaction handlers.

use crate::{
    AppResult,
    database::Database,
    models::{character::Character, history::History},
};
use serenity::all::MessageId;
use tracing::warn;

/// Returns the history and character associated with the given message, if it's a character reply.
pub async fn history_and_character_of(
    message: MessageId,
    db: &Database,
) -> AppResult<Option<(History, Character)>> {
    let Some(history) = db.history(message).await? else {
        return Ok(None);
    };
    let Some(character) = db.character(history.character()).await? else {
        warn!(
            "history {} references a missing character {}",
            history.id(),
            history.character()
        );
        return Ok(None);
    };
    Ok(Some((history, character)))
}
