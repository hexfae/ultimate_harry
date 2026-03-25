use miette::{Diagnostic, Report};
use poise::serenity_prelude::{Context, Message};
use rig::{agent::MultiTurnStreamItem, streaming::StreamedAssistantContent};
use serenity::{all::MessageId, futures::StreamExt};
use snafu::{ResultExt, Snafu};
use std::time::{Duration, Instant};

use crate::{
    constants::CHARACTER_LIMIT,
    db::Database,
    llm::LlmManager,
    models::{character::Character, history::History},
};

#[derive(Debug, Snafu, Diagnostic)]
enum NewMessageError {
    #[snafu(display("Kunde inte skicka meddelandet: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(events::message::send_message)
    )]
    SendMessage { source: serenity::Error },
    #[snafu(display("Kunde inte redigera meddelandet: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att meddelandet fortfarande finns"),
        code(events::message::edit_message)
    )]
    EditMessage { source: serenity::Error },
    #[snafu(display("Kunde inte reagera på meddelandet: {source}"))]
    #[diagnostic(help("Försök igen"), code(events::message::react))]
    React { source: serenity::Error },
    #[snafu(transparent)]
    #[diagnostic(transparent)]
    Database { source: crate::db::DatabaseError },
    #[snafu(display("Strömning misslyckades: {source}"))]
    #[diagnostic(
        help(
            "Försök igen eller kontrollera att modellen är tillgänglig och att nätverket fungerar"
        ),
        code(events::message::streaming)
    )]
    Streaming { source: rig::agent::StreamingError },
}

pub async fn message(ctx: &Context, user_message: &Message, db: &Database) -> Result<(), Report> {
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

    db.insert_history(history).await?;

    Ok(())
}

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
    if let Some(replied_to) = &new_message.referenced_message
        && let Some(user_emoji) = all_emoji.iter().find(|e| e.user_id == replied_to.author.id)
    {
        new_message
            .react(&ctx.http, user_emoji.emoji.clone())
            .await
            .context(ReactSnafu)?;
    }
    for mention in &new_message.mentions {
        if let Some(user_emoji) = all_emoji.iter().find(|e| e.user_id == mention.id) {
            new_message
                .react(&ctx.http, user_emoji.emoji.clone())
                .await
                .context(ReactSnafu)?;
        }
    }
    Ok(())
}

pub async fn history_and_character_of_replied_to(
    message: &Message,
    db: &Database,
) -> Result<Option<(History, Character)>, Report> {
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

pub async fn history_and_character_of(
    message: MessageId,
    db: &Database,
) -> Result<Option<(History, Character)>, Report> {
    let Some(history) = db.history(message).await? else {
        return Ok(None);
    };
    let Some(character) = db.character(history.character()).await? else {
        return Ok(None);
    };
    Ok(Some((history, character)))
}
