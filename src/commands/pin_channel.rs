//! The bot's Discord slash command for setting the pin channel.

use crate::{AppResult, Context, error::SendMessageSnafu, phrases, traits::SayEphemeral as _};
use serenity::all::GuildChannel;
use snafu::ResultExt as _;

/// Ställer in kanalen där fästa meddelanden hamnar.
#[poise::command(slash_command, rename = "fästkanal")]
pub async fn pin_channel(
    ctx: Context<'_>,
    #[rename = "kanal"]
    #[description = "Kanalen där fästa meddelanden ska hamna"]
    channel: GuildChannel,
) -> AppResult {
    ctx.data().db.upsert_pin_channel(channel.id).await?;
    ctx.say_ephemeral(phrases::done())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
