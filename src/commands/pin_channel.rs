//! The bot's Discord slash command for setting the pin channel.

use crate::{Context, database::DatabaseError, traits::SayEphemeral as _};
use miette::{Diagnostic, Result};
use serenity::all::GuildChannel;
use snafu::{ResultExt as _, Snafu};

#[poise::command(slash_command)]
pub async fn pin_channel(ctx: Context<'_>, channel: GuildChannel) -> Result<()> {
    ctx.data()
        .db
        .upsert_pin_channel(channel.id)
        .await
        .context(SetPinChannelSnafu)?;
    ctx.say_ephemeral("done").await.context(SendMessageSnafu)?;
    Ok(())
}

/// All errors that can happen when setting the pin channel.
#[derive(Debug, Snafu, Diagnostic)]
enum PinChannelError {
    /// Sending a message failed.
    #[snafu(display("Kunde inte skicka meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(commands::pin_channel::send_message)
    )]
    SendMessage {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Saving the pin channel to the database failed.
    #[snafu(display("Kunde inte spara pinkanal: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att inställningarna är giltiga"),
        code(commands::pin_channel::set_pin_channel)
    )]
    SetPinChannel {
        /// The source of the error.
        source: DatabaseError,
    },
}
