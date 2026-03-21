use miette::Report;
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
    MessageId,
};
use snafu::ResultExt;

use crate::{SendResponseSnafu, db::Database, events::message::HistoryCharacter};

pub async fn previous(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let Some((mut history, character)) = id.history_character(db).await? else {
        return Ok(());
    };
    history.previous();

    db.update_history(history.clone()).await?;

    let response = CreateInteractionResponse::UpdateMessage(
        history
            .to_response(&character, id, db)
            .await
            .to_slash_initial_response(CreateInteractionResponseMessage::new()),
    );

    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    Ok(())
}
