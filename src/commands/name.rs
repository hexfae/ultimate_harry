use crate::{Context, DeferEphemeralOrBroadcast, SendMessageSnafu};
use miette::Report;
use poise::serenity_prelude::UserId;
use snafu::ResultExt;

#[poise::command(slash_command)]
pub async fn name(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Namnet att använda"]
    name: String,
    #[rename = "användare"]
    #[description = "Användaren vars namn ska ändras"]
    user: Option<UserId>,
) -> Result<(), Report> {
    ctx.defer_ephemeral_or_broadcast().await?;
    let user_id = user.unwrap_or_else(|| ctx.author().id);
    ctx.data().db.upsert_user_name(user_id, name).await?;
    ctx.say("done").await.context(SendMessageSnafu)?;
    Ok(())
}
