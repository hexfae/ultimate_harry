use crate::{
    Context, DeferEphemeralOrBroadcast, FIVE_SECONDS, SendMessageSnafu,
    traits::DeleteSelfAndInvokingMessageIfPrefix,
};
use miette::Report;
use poise::serenity_prelude::ReactionType;
use snafu::ResultExt;
use tokio::time::sleep;
use ultimate_database::DB;

#[poise::command(slash_command, prefix_command)]
pub async fn emoji(ctx: Context<'_>, emoji: String) -> Result<(), Report> {
    ctx.defer_ephemeral_or_broadcast().await?;
    let Ok(emoji) = ReactionType::try_from(emoji) else {
        let msg = ctx
            .say("Det där var ingen emoji… (eller så gick någonting fel!)")
            .await
            .context(SendMessageSnafu)?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    };
    DB.upsert_user_emoji(ctx.author(), emoji.clone()).await?;
    ctx.say(emoji.to_string()).await.context(SendMessageSnafu)?;
    Ok(())
}
