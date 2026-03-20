use crate::{Context, DeferEphemeralOrBroadcast, SendMessageSnafu};
use miette::Report;
use serenity::all::GuildChannel;
use snafu::ResultExt;

#[poise::command(slash_command)]
pub async fn pin_channel(ctx: Context<'_>, channel: GuildChannel) -> Result<(), Report> {
    ctx.defer_ephemeral_or_broadcast().await?;
    ctx.data().db.upsert_pin_channel(channel.id).await?;
    ctx.say("done").await.context(SendMessageSnafu)?;
    Ok(())
}
