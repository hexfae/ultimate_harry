//! The button that posts a character's reply in the pin channel.

use crate::{database::Database, events::message::history_and_character_of};
use miette::{Diagnostic, Result};
use serenity::all::{
    ComponentInteraction, Context, CreateInteractionResponse, CreateInteractionResponseMessage,
    MessageId,
};
use snafu::{ResultExt as _, Snafu};

/// Send the reply in the configured pin channel.
pub async fn pin(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<()> {
    let Some((history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

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

/// All errors that can happen when pinning a reply.
#[derive(Debug, Snafu, Diagnostic)]
enum PinReplyError {
    /// Sending a message failed.
    #[snafu(display("Kunde inte skicka meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(events::interaction::pin::send_message)
    )]
    SendMessage {
        ///The source of the error.
        source: serenity::Error,
    },
    /// Sending a response failed.
    #[snafu(display("Kunde inte skicka interaktionssvar: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att interaktionen fortfarande är giltig"),
        code(events::interaction::pin::send_response)
    )]
    SendResponse {
        /// The source of the error.
        source: serenity::Error,
    },
}
