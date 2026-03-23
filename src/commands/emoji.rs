use crate::{Context, DeferSnafu, SendMessageSnafu};
use miette::Report;
use poise::serenity_prelude::ReactionType;
use snafu::ResultExt;

#[poise::command(slash_command)]
pub async fn emoji(ctx: Context<'_>, emoji: String) -> Result<(), Report> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;
    let Ok(emoji) = ReactionType::try_from(emoji) else {
        ctx.say("Det där var ingen emoji… (eller så gick någonting fel!)")
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };
    ctx.data()
        .db
        .upsert_user_emoji(ctx.author(), emoji.clone())
        .await?;
    ctx.say(emoji.to_string()).await.context(SendMessageSnafu)?;
    Ok(())
}
