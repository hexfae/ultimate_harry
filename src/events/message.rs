//! Handler for a new message being sent.
//!
//! This module handles the event when a new message is sent in Discord,
//! processing it and generating AI responses for character chats.

use crate::{
    AppResult,
    constants::CHARACTER_LIMIT,
    database::Database,
    error::{EditMessageSnafu, ReactSnafu, SendMessageSnafu, StreamingSnafu},
    llm::LlmManager,
    models::{character::Character, history::History},
};
use core::time::Duration;
use poise::serenity_prelude::{Context, Message};
use rig::{agent::MultiTurnStreamItem, streaming::StreamedAssistantContent};
use serenity::{all::MessageId, futures::StreamExt as _};
use snafu::ResultExt as _;
use std::time::Instant;
use tokio::time::{MissedTickBehavior, interval};

/// Handle a new message being sent.
pub async fn message(ctx: &Context, user_message: &Message, db: &Database) -> AppResult {
    react_to_mentions_and_replies(ctx, user_message, db).await?;
    if user_message.author.bot() {
        return Ok(());
    }

    let Some((mut history, character)) =
        history_and_character_of_replied_to(user_message, db).await?
    else {
        return Ok(());
    };

    let author = db.substitute_name(&user_message.author).await;
    history.push(history.chosen_message().to_owned());
    history.push((user_message, author));
    history.reset_choices();
    history.set_finished(false);

    let placeholder_message = history
        .to_placeholder_message(&character, user_message, db)
        .await;

    let mut bot_message = user_message
        .channel_id
        .send_message(&ctx.http, placeholder_message)
        .await
        .context(SendMessageSnafu)?;

    let requester = LlmManager::new(
        character
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
                    if now.elapsed() >= Duration::from_secs(30) {
                        total += "30 sekunder har gått utan ett svar. Jag ger upp.";
                        break;
                    }
                    let placeholder_edit = history.to_placeholder_message_edit(&character, now.elapsed(), db).await;
                    bot_message.edit(ctx, placeholder_edit).await.context(EditMessageSnafu)?;
                } else {
                    history.set_choices((character.clone(), total.clone(), now.elapsed()));
                    let edit = history.to_edit_response(&character, &bot_message, db).await;
                    bot_message.edit(ctx, edit).await.context(EditMessageSnafu)?;
                }
            }
        }
    }

    history.set_choices((character.clone(), total, now.elapsed()));
    history.set_id(&bot_message);
    history.set_finished(true);
    db.upsert_history(history.clone()).await?;

    let edit = history.to_edit_response(&character, &bot_message, db).await;

    bot_message
        .edit(ctx, edit)
        .await
        .context(EditMessageSnafu)?;

    Ok(())
}

/// Checks the mentions/replied to message of the new message, and reacts with the corresponding user emoji.
async fn react_to_mentions_and_replies(
    ctx: &Context,
    new_message: &Message,
    db: &Database,
) -> AppResult {
    let all_emoji = db.user_emoji().await?;
    if new_message.mention_everyone() {
        for user_emoji in &all_emoji {
            new_message
                .react(&ctx.http, user_emoji.emoji.clone())
                .await
                .context(ReactSnafu)?;
        }
        return Ok(());
    }
    if new_message.author.bot() {
        return Ok(());
    }
    if let Some(ref replied_to) = new_message.referenced_message
        && let Some(user_emoji) = all_emoji
            .iter()
            .find(|emoji| emoji.user_id == replied_to.author.id.to_string())
    {
        new_message
            .react(&ctx.http, user_emoji.emoji.clone())
            .await
            .context(ReactSnafu)?;
    }
    for mention in &new_message.mentions {
        if let Some(user_emoji) = all_emoji
            .iter()
            .find(|emoji| emoji.user_id == mention.id.to_string())
        {
            new_message
                .react(&ctx.http, user_emoji.emoji.clone())
                .await
                .context(ReactSnafu)?;
        }
    }
    Ok(())
}

/// Returns the history and character associated with the message that the given message replied to,
/// if it's a character reply.
pub async fn history_and_character_of_replied_to(
    message: &Message,
    db: &Database,
) -> AppResult<Option<(History, Character)>> {
    let Some(replied_to) = message.referenced_message.as_deref() else {
        return Ok(None);
    };
    let Some(history) = db.history(replied_to).await? else {
        return Ok(None);
    };
    let Some(character) = db.character(history.character()).await? else {
        return Ok(None);
    };
    Ok(Some((history, character)))
}

/// Returns the history and character associated with the given message, if it's a character reply.
pub async fn history_and_character_of(
    message: MessageId,
    db: &Database,
) -> AppResult<Option<(History, Character)>> {
    let Some(history) = db.history(message).await? else {
        return Ok(None);
    };
    let Some(character) = db.character(history.character()).await? else {
        return Ok(None);
    };
    Ok(Some((history, character)))
}
