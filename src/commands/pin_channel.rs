//! The bot's Discord slash command for setting the pin channel.

use crate::{AppResult, Context, error::SendMessageSnafu, traits::SayEphemeral as _};
use serenity::all::GuildChannel;
use snafu::ResultExt as _;

#[poise::command(slash_command)]
pub async fn pin_channel(ctx: Context<'_>, channel: GuildChannel) -> AppResult {
    ctx.data().db.upsert_pin_channel(channel.id).await?;
    ctx.say_ephemeral("done").await.context(SendMessageSnafu)?;
    Ok(())
}
