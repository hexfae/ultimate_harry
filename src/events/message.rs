//! Handler for a new message being sent.
//!
//! This module handles the event when a new message is sent in Discord,
//! processing it and generating AI responses for character chats.

use crate::{
    constants::CHARACTER_LIMIT,
    database::{Database, DatabaseError},
    llm::LlmManager,
    models::{character::Character, history::History},
};
use core::time::Duration;
use miette::{Diagnostic, Result};
use poise::serenity_prelude::{Context, Message};
use rig::{
    agent::{MultiTurnStreamItem, StreamingError},
    streaming::StreamedAssistantContent,
};
use serenity::{all::MessageId, futures::StreamExt as _};
use snafu::{ResultExt as _, Snafu};
use std::time::Instant;

/// Handle a new message being sent.
pub async fn message(ctx: &Context, user_message: &Message, db: &Database) -> Result<()> {
    react_to_mentions_and_replies(ctx, user_message, db).await?;

    let Some((mut history, character)) =
        history_and_character_of_replied_to(user_message, db).await?
    else {
        return Ok(());
    };

    let author = db.substitute_name(&user_message.author).await;
    history.push(history.chosen_choice_message().to_owned());
    history.push((user_message, author));
    history.reset_choices();
    history.has_finished(false);

    let placeholder = history.to_placeholder_message(&character, user_message);

    let mut bot_message = user_message
        .channel_id
        .send_message(&ctx.http, placeholder)
        .await
        .context(SendMessageSnafu)?;

    let requester = LlmManager::new(
        character
            .model_settings()
            .unwrap_or(db.model_settings().await),
    );

    let mut total = String::new();
    let now = Instant::now();
    let mut time_since_last_edit = now;
    let mut stream = requester.request_stream(&history, None).await?;

    while let Some(result) = stream.next().await {
        if let MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(delta)) =
            result.context(StreamingSnafu)?
        {
            if total.len() >= CHARACTER_LIMIT {
                break;
            }
            total += delta.text();
            if time_since_last_edit.elapsed() >= Duration::from_secs(1) {
                time_since_last_edit = Instant::now();
                history.set_choices((character.clone(), total.clone(), now.elapsed()));
                let edit = history.to_edit_response(&character, &bot_message, db).await;

                bot_message
                    .edit(ctx, edit)
                    .await
                    .context(EditMessageSnafu)?;
            }
        }
    }

    history.set_choices((character.clone(), total, now.elapsed()));
    history.set_id(&bot_message);
    history.has_finished(true);

    let edit = history.to_edit_response(&character, &bot_message, db).await;

    bot_message
        .edit(ctx, edit)
        .await
        .context(EditMessageSnafu)?;

    db.upsert_history(history).await?;

    Ok(())
}

/// Checks the mentions/replied to message of the new message, and reacts with the corresponding user emoji.
async fn react_to_mentions_and_replies(
    ctx: &Context,
    new_message: &Message,
    db: &Database,
) -> Result<(), NewMessageError> {
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
            .find(|emoji| emoji.user_id == replied_to.author.id)
    {
        new_message
            .react(&ctx.http, user_emoji.emoji.clone())
            .await
            .context(ReactSnafu)?;
    }
    for mention in &new_message.mentions {
        if let Some(user_emoji) = all_emoji.iter().find(|emoji| emoji.user_id == mention.id) {
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
) -> Result<Option<(History, Character)>> {
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
) -> Result<Option<(History, Character)>> {
    let Some(history) = db.history(message).await? else {
        return Ok(None);
    };
    let Some(character) = db.character(history.character()).await? else {
        return Ok(None);
    };
    Ok(Some((history, character)))
}

/// All errors that can happen when handling a new message.
#[derive(Debug, Snafu, Diagnostic)]
enum NewMessageError {
    /// Sending a message failed.
    #[snafu(display("Kunde inte skicka meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(events::message::send_message)
    )]
    SendMessage {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Editing a message failed.
    #[snafu(display("Kunde inte redigera meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att meddelandet fortfarande finns"),
        code(events::message::edit_message)
    )]
    EditMessage {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Reacting to a message failed.
    #[snafu(display("Kunde inte reagera på meddelande: {source}"))]
    #[diagnostic(help("Försök igen"), code(events::message::react))]
    React {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Interacting with the database failed.
    #[snafu(transparent)]
    #[diagnostic(transparent)]
    Database {
        /// The source of the error.
        source: DatabaseError,
    },
    /// Streaming the AI model response failed.
    #[snafu(display("Strömning misslyckades: {source}"))]
    #[diagnostic(
        help(
            "Försök igen eller kontrollera att modellen är tillgänglig och att nätverket fungerar"
        ),
        code(events::message::streaming)
    )]
    Streaming {
        ///The source of the error.
        source: StreamingError,
    },
}
