use miette::Report;
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::ResultExt;

use crate::{SendResponseSnafu, db::Database, events::message::history_and_character_of};

pub async fn previous(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let Some((mut history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    history.previous();

    let response = history.to_interaction(&character, id, db).await;

    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    db.update_history(history).await?;

    Ok(())
}
