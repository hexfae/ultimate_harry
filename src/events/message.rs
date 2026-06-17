//! Handler for a new message being sent.
//!
//! This module handles the event when a new message is sent in Discord,
//! processing it and generating AI responses for character chats.

use crate::{
    AppResult,
    database::Database,
    error::SendMessageSnafu,
    events::streaming::{MessageSink, ReplySink as _, stream_into},
    llm::LlmManager,
    models::{character::Character, history::History},
};
use poise::serenity_prelude::{Context, Message};
use serenity::all::{MessageId, ReactionType};
use snafu::ResultExt as _;
use alloc::collections::BTreeMap;
use std::time::Instant;
use tracing::warn;

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

    let options = db.character_menu_options().await?;

    let placeholder_message =
        history.to_placeholder_message(&character, user_message, &options);

    let mut bot_message = user_message
        .channel_id
        .send_message(&ctx.http, placeholder_message)
        .await
        .context(SendMessageSnafu)?;

    let requester = LlmManager::new(db.resolved_model_settings(&character).await);

    let now = Instant::now();
    let context = db.build_context(&history, &character).await?;

    let mut sink = MessageSink {
        ctx,
        history: &mut history,
        character: &character,
        message: &mut bot_message,
        db,
        options: &options,
    };
    let reply = stream_into(&requester, &context, None, now, &mut sink).await?;
    sink.finalize(reply, now.elapsed()).await?;

    Ok(())
}

/// Checks the mentions/replied to message of the new message, and reacts with the corresponding user emoji.
async fn react_to_mentions_and_replies(
    ctx: &Context,
    new_message: &Message,
    db: &Database,
) -> AppResult {
    let all_emoji: BTreeMap<String, ReactionType> = db
        .user_emoji()
        .await?
        .into_iter()
        .map(|user_emoji| (user_emoji.user_id, user_emoji.emoji))
        .collect();
    if new_message.mention_everyone() {
        for emoji in all_emoji.values() {
            react(ctx, new_message, emoji.clone()).await;
        }
        return Ok(());
    }
    if new_message.author.bot() {
        return Ok(());
    }
    if let Some(ref replied_to) = new_message.referenced_message
        && let Some(emoji) = all_emoji.get(&replied_to.author.id.to_string())
    {
        react(ctx, new_message, emoji.clone()).await;
    }
    for mention in &new_message.mentions {
        if let Some(emoji) = all_emoji.get(&mention.id.to_string()) {
            react(ctx, new_message, emoji.clone()).await;
        }
    }
    Ok(())
}

/// Reacts to a message with an emoji, logging instead of failing so a missed
/// reaction never aborts the surrounding reply pipeline.
async fn react(ctx: &Context, message: &Message, emoji: ReactionType) {
    if let Err(why) = message.react(&ctx.http, emoji).await {
        warn!(message_id = %message.id, "failed to react: {why}");
    }
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
    history_and_character_of(replied_to.id, db).await
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
        warn!(
            "history {} references a missing character {}",
            history.id(),
            history.character()
        );
        return Ok(None);
    };
    Ok(Some((history, character)))
}
