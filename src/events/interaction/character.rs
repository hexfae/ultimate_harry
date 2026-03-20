use crate::{
    EditMessageSnafu, SendMessageSnafu, SendResponseSnafu, db::Database,
    events::interaction::HistoryCharacter, llm::LlmManager, models::message::Message,
};
use miette::Report;
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, EditMessage, MessageId,
};
use snafu::ResultExt;
use std::time::Instant;

pub async fn character(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let selected_char_id = match &interaction.data.kind {
        poise::serenity_prelude::ComponentInteractionDataKind::StringSelect { values } => {
            values.first().cloned()
        }
        _ => None,
    };

    let Some(selected_char_id) = selected_char_id else {
        return Ok(());
    };

    let Ok(selected_record_id) = selected_char_id.parse::<surrealdb::RecordId>() else {
        return Ok(());
    };

    let Some(new_character) = db.character(&selected_record_id).await? else {
        return Ok(());
    };

    let Some((mut history, _old_character)) = id.history_character(db).await? else {
        return Ok(());
    };

    history.push(history.chosen_choice_message().to_owned());

    let user_name = db.substitute_name(interaction.message.author.id).await;
    let discord_msg = (*interaction.message).clone();
    history.push(Message::from((discord_msg, user_name)));

    history.reset_choices();

    let user_id = interaction.message.author.id;
    history.replace_setup_with(&new_character, user_id);

    interaction
        .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
        .await
        .context(SendResponseSnafu)?;

    let placeholder = history
        .to_placeholder(&new_character)
        .to_prefix((&*interaction.message).into());

    let mut response_message = interaction
        .message
        .channel_id
        .send_message(&ctx.http, placeholder)
        .await
        .context(SendMessageSnafu)?;

    let requester = LlmManager::new(
        new_character
            .model_settings()
            .unwrap_or(db.model_settings().await),
    );

    let now = Instant::now();
    let response = requester.request(&history).await?;

    history.set_choices((new_character.clone(), response, now.elapsed()));

    let edit = history
        .to_response(&new_character, response_message.id, db)
        .await
        .to_prefix_edit(EditMessage::new());

    response_message
        .edit(ctx, edit)
        .await
        .context(EditMessageSnafu)?;

    history.set_id(response_message.id);

    db.insert_history(history.clone()).await?;

    Ok(())
}
