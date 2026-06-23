//! The buttons that swipe between a message's replies and edit revisions.

use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::ResultExt as _;

use crate::{
    AppResult, database::Database, error::SendResponseSnafu, models::character::Character,
    models::history::History,
};

/// Apply `mutate` to the history of this message, then re-render it.
///
/// Shared by the previous/undo/redo buttons, which differ only in which
/// cursor-moving call they make on the loaded [`History`].
pub async fn swipe<Mutate: FnOnce(&mut History)>(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
    mut history: History,
    character: Character,
    mutate: Mutate,
) -> AppResult {
    mutate(&mut history);

    let options = db.character_menu_options().await?;

    // persist the moved cursor before rendering it, so a save failure surfaces as
    // an error rather than showing a swipe that was never stored.
    db.upsert_history(history.clone()).await?;

    let response = history.to_interaction(&character, id, db, &options).await;
    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    Ok(())
}
