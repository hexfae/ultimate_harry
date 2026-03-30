//! The select menu that makes the bot respond as a different character.

use crate::{
    AppResult,
    constants::CHARACTER_LIMIT,
    database::Database,
    error::{EditMessageSnafu, SendMessageSnafu, SendResponseSnafu, StreamingSnafu},
    events::message::history_and_character_of,
    llm::LlmManager,
    models::message::Message,
};
use core::time::Duration;
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, MessageId,
};
use rig::{agent::MultiTurnStreamItem, streaming::StreamedAssistantContent};
use serenity::{all::ComponentInteractionDataKind, futures::StreamExt as _};
use snafu::ResultExt as _;
use std::time::Instant;
use tokio::time::{MissedTickBehavior, interval};

/// Respond to the history of this message as the given character.
pub async fn character(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> AppResult {
    let maybe_selected_char_id = match interaction.data.kind {
        ComponentInteractionDataKind::StringSelect { ref values } => values.into_iter().next(),
        _ => None,
    };

    let Some(selected_char_id) = maybe_selected_char_id else {
        return Ok(());
    };

    let Ok(selected_record_id) = selected_char_id.parse::<surrealdb::RecordId>() else {
        return Ok(());
    };

    let Some(new_character) = db.character(&selected_record_id).await? else {
        return Ok(());
    };

    let Some((mut history, _old_character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    history.push(history.chosen_choice_message().to_owned());

    history.push(Message::new_system(format!(
        "Användaren byter karaktär till {new_character}."
    )));

    history.reset_choices();

    let user_id = interaction.message.author.id;
    history.replace_setup_with(&new_character, user_id);

    interaction
        .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
        .await
        .context(SendResponseSnafu)?;

    let placeholder = history.to_placeholder_message(&new_character, &interaction.message);

    let mut response_message = interaction
        .message
        .channel_id
        .send_message(&ctx.http, placeholder)
        .await
        .context(SendMessageSnafu)?;

    let requester = LlmManager::new(
        new_character
            .model_settings()
            .unwrap_or(db.model_settings().await),
    );

    let mut total = String::new();
    let now = Instant::now();
    let mut stream = requester.request_stream(&history, None).await?;
    let mut interval = interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            result = stream.next() => {
                match result.transpose().context(StreamingSnafu)? {
                    Some(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(delta))) => {
                        if total.len() >= CHARACTER_LIMIT {
                            break;
                        }
                        total += delta.text();
                    }
                    Some(_) => {},
                    None => break,
                }
            }
            _ = interval.tick() => {
                if total.is_empty() {
                    let placeholder_edit = history.to_placeholder_message_edit(&new_character, now.elapsed());
                    response_message.edit(ctx, placeholder_edit).await.context(EditMessageSnafu)?;
                } else {
                    history.set_choices((new_character.clone(), total.clone(), now.elapsed()));
                    let edit = history.to_edit_response(&new_character, &response_message, db).await;
                    response_message.edit(ctx, edit).await.context(EditMessageSnafu)?;
                }
            }
        }
    }

    history.set_choices((new_character.clone(), total, now.elapsed()));
    history.set_id(&response_message);
    history.has_finished(true);

    let edit = history
        .to_edit_response(&new_character, &response_message, db)
        .await;

    response_message
        .edit(ctx, edit)
        .await
        .context(EditMessageSnafu)?;

    db.upsert_history(history.clone()).await?;

    Ok(())
}
