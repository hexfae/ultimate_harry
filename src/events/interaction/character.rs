//! The select menu that makes the bot respond as a different character.

use crate::{
    AppResult,
    database::Database,
    error::{SendMessageSnafu, SendResponseSnafu},
    events::streaming::{MessageSink, ReplySink as _, stream_into},
    llm::LlmManager,
    models::{history::History, message::Message},
};
use poise::serenity_prelude::{ComponentInteraction, Context, CreateInteractionResponse};
use serenity::all::ComponentInteractionDataKind;
use snafu::ResultExt as _;
use std::time::Instant;

/// Respond to the history of this message as the given character.
pub async fn character(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    mut history: History,
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

    history.begin_handoff(
        new_character.id().to_owned(),
        Message::new_user("System", format!("Svara nu som {new_character}.")),
    );

    interaction
        .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
        .await
        .context(SendResponseSnafu)?;

    let options = db.character_menu_options().await?;

    let placeholder =
        history.to_placeholder_message(&new_character, &interaction.message, &options);

    let mut response_message = interaction
        .message
        .channel_id
        .send_message(&ctx.http, placeholder)
        .await
        .context(SendMessageSnafu)?;

    let requester = LlmManager::new(db.resolved_model_settings(&new_character).await);

    let now = Instant::now();
    let context = db.build_context(&history, &new_character).await?;
    let prompt = format!("Fortsätt rollspelet som {new_character}.");

    let mut sink = MessageSink {
        ctx,
        history: &mut history,
        character: &new_character,
        message: &mut response_message,
        db,
        options: &options,
    };
    let reply = stream_into(&requester, &context, Some(prompt), now, &mut sink).await?;
    sink.finalize(reply, now.elapsed()).await?;

    db.record_character_spawn(new_character.id(), interaction.user.id)
        .await?;

    Ok(())
}
