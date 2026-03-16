use crate::{SendResponseSnafu, event_handler::HistoryCharacter};
use miette::Report;
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
    EditInteractionResponse, MessageId,
};
use snafu::ResultExt;
use std::time::Instant;
use ultimate_config::CONFIG;
use ultimate_database::DB;
use ultimate_message::Message as UltimateMessage;
use ultimate_requester::Requester;

pub async fn next(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
) -> Result<(), Report> {
    let Some((mut history, character)) = id.history_character().await? else {
        return Ok(());
    };

    if history.current_choice() + 1 < history.choices_len() {
        interaction
            .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
            .await
            .context(SendResponseSnafu)?;

        history.next();
    } else {
        let placeholder = CreateInteractionResponse::UpdateMessage(
            history
                .to_placeholder(&character)
                .to_slash_initial_response(CreateInteractionResponseMessage::new()),
        );

        interaction
            .create_response(&ctx.http, placeholder)
            .await
            .context(SendResponseSnafu)?;

        let requester = Requester::new(
            character
                .model_settings()
                .unwrap_or_else(|| CONFIG.read().model_settings()),
        );

        let now = Instant::now();
        let response = requester.request(history.clone()).await?;

        // current choice is set in this function
        history.push_choice(UltimateMessage::try_from((
            character.clone(),
            response,
            now.elapsed(),
        ))?);
    }

    DB.update_history(history.clone()).await?;

    let response = history
        .to_response(&character, id)
        .to_slash_initial_response_edit(EditInteractionResponse::new());

    interaction
        .edit_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    Ok(())
}
