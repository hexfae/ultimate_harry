//! The button that shows the previous revision of a message.

use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::ResultExt as _;

use crate::{
    AppResult, database::Database, error::SendResponseSnafu,
    events::message::history_and_character_of,
};

/// Show the previous revision of this message.
pub async fn undo(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> AppResult {
    let Some((mut history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    history.undo();

    let response = history.to_interaction(&character, id, db).await;

    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    db.upsert_history(history).await?;

    Ok(())
}
