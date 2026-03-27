//! The bot's Discord slash command for creating chats.

use crate::{
    Context,
    commands::autocomplete,
    models::{character::Character, history::History},
    phrases::no_character,
    traits::SayEphemeral as _,
};
use miette::{Diagnostic, Result};
use poise::serenity_prelude::MessageId;
use snafu::{ResultExt as _, Snafu};

#[poise::command(slash_command, rename = "prata")]
pub async fn chat(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: Option<String>,
) -> Result<()> {
    let db = &ctx.data().db;

    let characters: Vec<Character> = if let Some(character_name) = name {
        db.characters_by_similarity(character_name).await?
    } else {
        db.random_characters().await?
    };

    let Some(character) = characters.first() else {
        ctx.say_ephemeral(no_character())
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };

    let id = MessageId::new(1);
    let mut history = History::from((character, id, ctx.author().id));
    history.has_finished(true);

    let msg = ctx
        .send(history.to_response(character, id, &ctx.data().db).await)
        .await
        .context(SendMessageSnafu)?;
    let actual_id = msg.message().await.context(RetrieveMessageSnafu)?.id;

    history.set_id(actual_id);
    db.upsert_history(history).await?;

    Ok(())
}

/// All errors that can happen when starting a new chat.
#[derive(Debug, Snafu, Diagnostic)]
enum ChatError {
    /// Sending a message failed.
    #[snafu(display("Kunde inte skicka meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(commands::chat::send_message)
    )]
    SendMessage {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Retrieving a message failed.
    #[snafu(display("Kunde inte hämta meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att meddelandet fortfarande finns"),
        code(commands::chat::retrieve_message)
    )]
    RetrieveMessage {
        /// The source of the error.
        source: serenity::Error,
    },
}
