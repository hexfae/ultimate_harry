//! The select menu that makes the bot respond as a different character.

use crate::{
    AppResult,
    database::Database,
    error::{EditMessageSnafu, SendMessageSnafu, SendResponseSnafu},
    events::{
        message::history_and_character_of,
        streaming::{MessageSink, stream_into},
    },
    llm::LlmManager,
    models::message::Message,
};
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, MessageId,
};
use serenity::all::ComponentInteractionDataKind;
use snafu::ResultExt as _;
use std::time::Instant;

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

    let requester = LlmManager::new(db.resolved_model_settings(&new_character).await);

    let now = Instant::now();
    let context = db.build_context(&history, &new_character, user_id).await?;
    let prompt = format!("Fortsätt rollspelet som {new_character}.");

    let mut sink = MessageSink {
        ctx,
        history: &mut history,
        character: &new_character,
        message: &mut response_message,
        db,
    };
    let total = stream_into(&requester, &context, Some(prompt), now, &mut sink).await?;

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
