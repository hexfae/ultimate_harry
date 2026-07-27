//! Handler for a new message being sent.
//!
//! This module handles the event when a new message is sent in Discord,
//! processing it and generating AI responses for character chats.

use crate::{
    AppResult,
    cancellation::Cancellations,
    database::Database,
    error::{AppError, SendMessageSnafu},
    error_display::error_message,
    events::lookup::history_and_character_of_replied_to,
    events::streaming::{MessageSink, report_reply_failure, stream_and_finalize},
    models::{
        character::{Character, CharacterOption},
        history::History,
    },
    util::report_error,
};
use alloc::collections::BTreeMap;
use poise::serenity_prelude::{Context, Message};
use serenity::all::{CreateAllowedMentions, ReactionType};
use snafu::ResultExt as _;
use tracing::warn;

/// Handle a new message being sent.
pub async fn message(
    ctx: &Context,
    user_message: &Message,
    db: &Database,
    cancellations: &Cancellations,
) -> AppResult {
    react_to_mentions_and_replies(ctx, user_message, db).await;
    if user_message.author.bot() {
        return Ok(());
    }

    let begun = match begin_reply(ctx, user_message, db).await {
        Ok(begun) => begun,
        Err(why) => {
            report_setup_failure(ctx, user_message, &why).await;
            return Err(why);
        }
    };
    let Some((mut history, character, options, mut bot_message)) = begun else {
        return Ok(());
    };

    // the placeholder is now live, so a later failure must replace it with an
    // error notice rather than leaving the user staring at a frozen placeholder.
    if let Err(why) = reply_into(
        ctx,
        db,
        cancellations,
        &character,
        &mut history,
        &mut bot_message,
        &options,
    )
    .await
    {
        report_reply_failure(ctx, &mut bot_message, &why).await;
        return Err(why);
    }

    Ok(())
}

/// Resolves the conversation the user replied to and posts the placeholder the
/// reply will stream into, or `None` if the message is not a character reply.
async fn begin_reply(
    ctx: &Context,
    user_message: &Message,
    db: &Database,
) -> AppResult<Option<(History, Character, Vec<CharacterOption>, Message)>> {
    let Some((mut history, character)) =
        history_and_character_of_replied_to(user_message, db).await?
    else {
        return Ok(None);
    };

    let author = db.substitute_name(&user_message.author).await;
    history.begin_new_turn((user_message, author));

    let options = db.character_menu_options().await?;

    let placeholder_message = history
        .to_placeholder_message(&character, user_message, db, &options)
        .await;

    let bot_message = user_message
        .channel_id
        .send_message(&ctx.http, placeholder_message)
        .await
        .context(SendMessageSnafu)?;

    Ok(Some((history, character, options, bot_message)))
}

/// Shows the user the red error notice for a failure that happened before the
/// placeholder went live, so a reply never fails silently.
async fn report_setup_failure(ctx: &Context, user_message: &Message, why: &AppError) {
    let notice = error_message(why.user_message())
        .reference_message(user_message)
        .allowed_mentions(CreateAllowedMentions::new());
    if let Err(report_why) = user_message
        .channel_id
        .send_message(&ctx.http, notice)
        .await
    {
        warn!(
            message_id = %user_message.id,
            "failed to show the error notice: {report_why}"
        );
    }
}

/// Streams the character's reply into the already-posted placeholder message.
async fn reply_into(
    ctx: &Context,
    db: &Database,
    cancellations: &Cancellations,
    character: &Character,
    history: &mut History,
    bot_message: &mut Message,
    options: &[CharacterOption],
) -> AppResult {
    let sink = MessageSink {
        ctx,
        history,
        character,
        message: bot_message,
        db,
        options,
    };
    stream_and_finalize(None, cancellations, sink).await
}

/// Checks the mentions/replied to message of the new message, and reacts with the corresponding user emoji.
///
/// Reactions are cosmetic and best-effort, so this never fails the surrounding reply.
async fn react_to_mentions_and_replies(ctx: &Context, new_message: &Message, db: &Database) {
    if new_message.author.bot() {
        return;
    }
    // a failure to load the emoji must never abort the reply that follows: log it
    // and skip reacting.
    let all_emoji: BTreeMap<String, ReactionType> = match db.user_emoji().await {
        Ok(emoji) => emoji.into_iter().collect(),
        Err(why) => {
            warn!("failed to load reaction emoji, skipping reactions");
            report_error(why);
            return;
        }
    };
    if new_message.mention_everyone() {
        for emoji in all_emoji.values() {
            react(ctx, new_message, emoji.clone()).await;
        }
        return;
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
}

/// Reacts to a message with an emoji, logging instead of failing so a missed
/// reaction never aborts the surrounding reply pipeline.
async fn react(ctx: &Context, message: &Message, emoji: ReactionType) {
    if let Err(why) = message.react(&ctx.http, emoji).await {
        warn!(message_id = %message.id, "failed to react: {why}");
    }
}
