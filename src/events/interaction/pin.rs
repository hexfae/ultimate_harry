//! The button that posts a character's reply in the pin channel.

use crate::{
    AppResult,
    database::Database,
    error::{NoPinChannelSnafu, SendMessageSnafu, SendResponseSnafu},
    models::{character::Character, history::History},
    phrases,
};
use serenity::all::{
    ChannelId, ComponentInteraction, Context, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use snafu::{ResultExt as _, ensure};

/// Send the reply in the configured pin channel.
pub async fn pin(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    history: History,
    character: Character,
) -> AppResult {
    let pins_channel_id = db.pins_channel().await;
    ensure!(pins_channel_id != ChannelId::default(), NoPinChannelSnafu);

    let reply = history
        .into_bare_response(&character, interaction.message.link().to_string(), db)
        .await;

    let pin = pins_channel_id
        .widen()
        .send_message(&ctx.http, reply.to_prefix((&*interaction.message).into()))
        .await
        .context(SendMessageSnafu)?;

    let response = CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .content(format!("{}\n{}", phrases::pinned(), pin.link())),
    );

    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;
    Ok(())
}
