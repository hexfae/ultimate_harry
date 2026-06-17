//! The button that posts a character's reply in the pin channel.

use crate::{
    AppResult,
    database::Database,
    error::{SendMessageSnafu, SendResponseSnafu},
    models::{character::Character, history::History},
};
use serenity::all::{
    ComponentInteraction, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
};
use snafu::ResultExt as _;

/// Send the reply in the configured pin channel.
pub async fn pin(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    history: History,
    character: Character,
) -> AppResult {
    let reply = history
        .into_bare_response(&character, interaction.message.link().to_string(), db)
        .await;

    let pins_channel_id = db.pins_channel().await;
    let pin = pins_channel_id
        .widen()
        .send_message(&ctx.http, reply.to_prefix((&*interaction.message).into()))
        .await
        .context(SendMessageSnafu)?;

    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new().content(pin.link().to_string()),
    );

    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;
    Ok(())
}
