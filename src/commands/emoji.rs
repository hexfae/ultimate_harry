//! The bot's Discord slash command for setting users' emoji.

use crate::{AppResult, Context, error::SendMessageSnafu, traits::SayEphemeral as _};
use poise::serenity_prelude::ReactionType;
use snafu::ResultExt as _;

/// The bot's Discord slash command for setting a user's emoji.
#[poise::command(slash_command)]
pub async fn emoji(ctx: Context<'_>, emoji: String) -> AppResult {
    let Ok(found_emoji) = ReactionType::try_from(emoji) else {
        ctx.say_ephemeral("Det där var ingen emoji… (eller så gick någonting fel!)")
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };
    ctx.data()
        .db
        .upsert_user_emoji(ctx.author(), found_emoji.clone())
        .await?;
    ctx.say_ephemeral(found_emoji.to_string())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
