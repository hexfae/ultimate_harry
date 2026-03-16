use crate::{
    Context, DeferEphemeralOrBroadcast, FIVE_SECONDS, SendMessageSnafu,
    traits::{DeleteSelfAndInvokingMessageIfPrefix, SayWith},
};
use miette::Report;
use poise::serenity_prelude::Message;
use snafu::ResultExt;
use tokio::time::sleep;
use ultimate_config::CONFIG;
use ultimate_database::DB;
use ultimate_phrases::{NO_CHARACTER_PHRASES, NO_HISTORY_PHRASES, sample};

#[poise::command(context_menu_command = "Pinna meddelandet")]
pub async fn pin(ctx: Context<'_>, message: Message) -> Result<(), Report> {
    ctx.defer_ephemeral_or_broadcast().await?;

    let Some(history) = DB.history(&message).await? else {
        let response = sample(NO_HISTORY_PHRASES);
        let msg = ctx.say_with(response).await?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    };

    let Some(character) = DB.character(history.character()).await? else {
        let response = sample(NO_CHARACTER_PHRASES);
        let msg = ctx.say_with(response).await?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    };

    let reply = history.into_bare_response(&character, message.link().to_string());

    let channel_id = CONFIG.read().pins_channel_id();
    let pin = channel_id
        .widen()
        .send_message(ctx.http(), reply.to_prefix((&message).into()))
        .await
        .context(SendMessageSnafu)?;

    ctx.say(pin.link().to_string())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
