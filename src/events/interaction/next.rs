use crate::{
    EditMessageSnafu, SendResponseSnafu, constants::CHARACTER_LIMIT, db::Database,
    events::message::history_and_character_of, llm::LlmManager,
};
use miette::{Diagnostic, Report};
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, MessageId,
};
use rig::{agent::MultiTurnStreamItem, streaming::StreamedAssistantContent};
use serenity::futures::StreamExt;
use snafu::{ResultExt, Snafu};
use std::time::{Duration, Instant};

#[derive(Debug, Snafu, Diagnostic)]
pub struct StreamingError {
    source: rig::agent::StreamingError,
}

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
        history.has_finished(false);

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

        let mut total = String::new();
        let now = Instant::now();
        let mut time_since_last_edit = now;
        let mut response = requester.request_stream(&history, None).await;

        while let Some(delta) = response.next().await {
            if let MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(delta)) =
                delta.context(StreamingSnafu)?
            {
                if total.len() >= CHARACTER_LIMIT {
                    break;
                }
                total += delta.text();
                if time_since_last_edit.elapsed() >= Duration::from_secs(1) {
                    time_since_last_edit = Instant::now();
                    history.set_choices((character.clone(), total.clone(), now.elapsed()));
                    let edit = history.to_edit_interaction(&character, id, db).await;

                    interaction
                        .edit_response(&ctx.http, edit)
                        .await
                        .context(EditMessageSnafu)?;
                }
            }
        }

        // current choice is set in this function
        history.push_choice((character.clone(), total, now.elapsed()));
        history.has_finished(true);

        let response = history.to_edit_interaction(&character, id, db).await;

        interaction
            .edit_response(&ctx.http, response)
            .await
            .context(SendResponseSnafu)?;
    }

    db.update_history(history).await?;

    Ok(())
}
