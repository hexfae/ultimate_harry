use crate::{
    db::Database, events::message::history_and_character_of, llm::LlmManager,
    models::message::Message,
};
use miette::{Diagnostic, Report};
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, MessageId,
};
use serenity::all::ComponentInteractionDataKind;
use snafu::{ResultExt, Snafu};
use std::time::Instant;

#[derive(Debug, Snafu, Diagnostic)]
enum ViewCharacterError {
    #[snafu(display("Kunde inte skicka meddelandet: {source}"))]
    #[diagnostic(help("Försök igen eller kontrollera att kanalen är tillgänglig"), code(events::interaction::character::send_message))]
    SendMessage { source: serenity::Error },
    #[snafu(display("Kunde inte skicka interaktionssvar: {source}"))]
    #[diagnostic(help("Försök igen eller kontrollera att interaktionen fortfarande är giltig"), code(events::interaction::character::send_response))]
    SendResponse { source: serenity::Error },
    #[snafu(display("Kunde inte redigera meddelandet: {source}"))]
    #[diagnostic(help("Försök igen eller kontrollera att meddelandet fortfarande finns"), code(events::interaction::character::edit_message))]
    EditMessage { source: serenity::Error },
}

pub async fn character(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let selected_char_id = match &interaction.data.kind {
        ComponentInteractionDataKind::StringSelect { values } => values.into_iter().next(),
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

    let Some((mut history, _old_character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    history.push(history.chosen_choice_message().to_owned());

    let user_name = db.substitute_name(interaction.message.author.id).await;
    let discord_msg = (*interaction.message).clone();
    history.push(Message::from((&discord_msg, user_name)));

    history.reset_choices();

    let user_id = interaction.message.author.id;
    history.replace_setup_with(&new_character, user_id);

    interaction
        .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
        .await
        .context(SendResponseSnafu)?;

    let placeholder = history.to_placeholder_message(&new_character, &interaction.message);

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
    let response = requester
        .request(
            &history,
            Some(format!(
                "Du hoppar nu in i rollspelet som {new_character}. Fortsätt rollspelet."
            )),
        )
        .await?;

    history.set_choices((new_character.clone(), response, now.elapsed()));

    let edit = history
        .to_edit_response(&new_character, response_message.id, db)
        .await;

    response_message
        .edit(ctx, edit)
        .await
        .context(EditMessageSnafu)?;

    history.set_id(response_message.id);

    db.insert_history(history.clone()).await?;

    Ok(())
}
