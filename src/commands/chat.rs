//! The bot's Discord slash command for creating chats.

use crate::{
    AppResult, Context,
    commands::autocomplete,
    error::{EditMessageSnafu, RetrieveMessageSnafu, SendMessageSnafu},
    models::{character::Character, history::History},
    phrases::no_character,
    traits::SayEphemeral as _,
};
use poise::serenity_prelude::MessageId;
use snafu::ResultExt as _;

#[poise::command(slash_command, rename = "prata")]
pub async fn chat(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: Option<String>,
) -> AppResult {
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

    msg.edit(ctx, history.to_response(character, actual_id, db).await)
        .await
        .context(EditMessageSnafu)?;
    db.upsert_history(history).await?;

    Ok(())
}
