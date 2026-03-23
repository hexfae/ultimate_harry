use crate::{
    SendResponseSnafu, db::Database, events::message::history_and_character_of, llm::LlmManager,
};
use miette::Report;
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, MessageId,
};
use snafu::ResultExt;
use std::time::Instant;

pub async fn next(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let Some((mut history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    if !history.is_on_last_choice() {
        history.next();

        let response = history.to_interaction(&character, id, db).await;

        interaction
            .create_response(&ctx.http, response)
            .await
            .context(SendResponseSnafu)?;
    } else {
        let placeholder = CreateInteractionResponse::UpdateMessage(
            history.to_placeholder_interaction(&character),
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

        let response = history.to_edit_interaction(&character, id, db).await;

        interaction
            .edit_response(&ctx.http, response)
            .await
            .context(SendResponseSnafu)?;
    }

    db.update_history(history).await?;

    Ok(())
}
