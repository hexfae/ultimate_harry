//! The bot's Discord slash command for setting users' emoji.

use crate::{
    AppResult, Context, components::reaction_from, error::SendMessageSnafu,
    traits::SayEphemeral as _,
};
use snafu::ResultExt as _;

/// Ställer in din emoji.
#[poise::command(slash_command)]
pub async fn emoji(
    ctx: Context<'_>,
    #[description = "Emojin att använda"] emoji: String,
) -> AppResult {
    let Some(found_emoji) = reaction_from(&emoji) else {
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
