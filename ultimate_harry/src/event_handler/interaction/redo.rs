use miette::Report;
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
    MessageId,
};
use snafu::ResultExt;
use ultimate_database::DB;

use crate::{SendResponseSnafu, event_handler::HistoryCharacter};

pub async fn redo(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
) -> Result<(), Report> {
    let Some((mut history, character)) = id.history_character().await? else {
        return Ok(());
    };
    history.redo();

    DB.update_history(history.clone()).await?;

    let response = CreateInteractionResponse::UpdateMessage(
        history
            .to_response(&character, id)
            .to_slash_initial_response(CreateInteractionResponseMessage::new()),
    );

    interaction
        .create_response(ctx, response)
        .await
        .context(SendResponseSnafu)?;

    Ok(())
}
