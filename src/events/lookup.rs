//! Shared lookup resolving a conversation and its character from a message ID,
//! used by both the message and interaction handlers.

use crate::{
    AppResult,
    database::Database,
    models::{character::Character, history::History},
};
use poise::serenity_prelude::Message;
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

/// Returns the history and character associated with the message that the given message replied to,
/// if it's a character reply.
pub async fn history_and_character_of_replied_to(
    message: &Message,
    db: &Database,
) -> AppResult<Option<(History, Character)>> {
    let Some(replied_to) = message.referenced_message.as_deref() else {
        return Ok(None);
    };
    history_and_character_of(replied_to.id, db).await
}
