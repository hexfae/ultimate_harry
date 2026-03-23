use crate::{SendResponseSnafu, db::Database, events::message::HistoryCharacter, llm::LlmManager};
use miette::Report;
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
    EditInteractionResponse, MessageId,
};
use snafu::ResultExt;
use std::time::Instant;

pub async fn next(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let Some((mut history, character)) = id.history_character(db).await? else {
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

        let requester = LlmManager::new(
            character
                .model_settings()
                .unwrap_or(db.model_settings().await),
        );

        let now = Instant::now();
        let response = requester.request(&history.clone(), None).await?;

        // current choice is set in this function
        history.push_choice((character.clone(), response, now.elapsed()));
    }

    db.update_history(history.clone()).await?;

    let response = history
        .to_response(&character, id, db, true)
        .await
        .to_slash_initial_response_edit(EditInteractionResponse::new());

    interaction
        .edit_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    Ok(())
}
