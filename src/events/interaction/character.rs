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
    let ComponentInteractionDataKind::StringSelect { ref values } = interaction.data.kind else {
        return Ok(());
    };

    let Some(selected_char_id) = values.first() else {
        return Ok(());
    };
    let character_id: &str = selected_char_id;

    let Some(new_character) = db.character(character_id).await? else {
        return Ok(());
    };

    let Some((mut history, _old_character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    let user_id = interaction.message.author.id;
    history.push(history.chosen_message().to_owned());
    history.reset_choices();
    history.set_character(new_character.id().to_owned());
    history.push(Message::new_user(
        "System",
        format!("Svara nu som {new_character}."),
        user_id,
    ));
    history.set_finished(false);

    interaction
        .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
        .await
        .context(SendResponseSnafu)?;

    let placeholder = history
        .to_placeholder_message(&new_character, &interaction.message, db)
        .await;

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
    let context = db.build_context(&history, &new_character, user_id).await?;
    let mut stream = requester
        .request_stream(
            &context,
            Some(format!("Fortsätt rollspelet som {new_character}.")),
        )
        .await?;
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
                    if now.elapsed() >= Duration::from_secs(30) {
                        total += "30 sekunder har gått utan ett svar. Jag ger upp.";
                        break;
                    }
                    let placeholder_edit = history.to_placeholder_message_edit(&new_character, now.elapsed(), db).await;
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
    history.set_finished(true);
    db.upsert_history(history.clone()).await?;

    let edit = history
        .to_edit_response(&new_character, &response_message, db)
        .await;

    response_message
        .edit(ctx, edit)
        .await
        .context(EditMessageSnafu)?;

    Ok(())
}
