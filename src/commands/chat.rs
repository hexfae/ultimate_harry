//! The bot's Discord slash command for creating chats.

use crate::{
    AppResult, Context,
    commands::{autocomplete, notify_no_character},
    error::{EditMessageSnafu, RetrieveMessageSnafu, SendMessageSnafu},
    models::{character::Character, history::History},
    util::report_error,
};
use poise::serenity_prelude::MessageId;
use snafu::ResultExt as _;
use tracing::warn;

/// Startar en chatt med en gubbe.
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
        notify_no_character(ctx).await?;
        return Ok(());
    };

    // the real ID only exists once the message is sent, so the buttons go out
    // disabled and the edit below re-renders them keyed on it
    let mut history = History::from((character, MessageId::new(1)));
    history.set_finished(true);

    let options = db.character_menu_options().await?;

    let msg = ctx
        .send(history.to_pending_response(character, db, &options).await)
        .await
        .context(SendMessageSnafu)?;

    let actual_id = msg.message().await.context(RetrieveMessageSnafu)?.id;
    history.set_id(actual_id);

    msg.edit(
        ctx,
        history
            .to_response(character, actual_id, db, &options)
            .await,
    )
    .await
    .context(EditMessageSnafu)?;
    db.upsert_history(history).await?;

    // best-effort: the chat is already created, so a stats-write blip must not
    // fail the command and show the user an error
    if let Err(why) = db
        .record_character_spawn(character.id(), ctx.author().id)
        .await
    {
        warn!("failed to record spawn stats, keeping the chat");
        report_error(why);
    }

    Ok(())
}
