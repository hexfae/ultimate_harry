//! The bot's Discord slash command for setting users' names.

use crate::{AppResult, Context, error::SendMessageSnafu, traits::SayEphemeral as _};
use poise::serenity_prelude::UserId;
use snafu::ResultExt as _;

/// Ställer in en användares namn.
#[poise::command(slash_command, rename = "namn")]
pub async fn name(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Namnet att använda"]
    name: String,
    #[rename = "användare"]
    #[description = "Användaren vars namn ska ändras"]
    user: Option<UserId>,
) -> AppResult {
    let user_id = user.unwrap_or_else(|| ctx.author().id);
    ctx.data().db.upsert_user_name(user_id, name).await?;

    ctx.say_ephemeral("Klart!").await.context(SendMessageSnafu)?;
    Ok(())
}
