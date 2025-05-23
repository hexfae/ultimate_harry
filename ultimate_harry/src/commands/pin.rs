use crate::{
    Context, DeferEphemeralOrBroadcast, FIVE_SECONDS, SendMessageSnafu,
    traits::{DeleteSelfAndInvokingMessageIfPrefix, SayWith},
};
use miette::Report;
use poise::serenity_prelude::{ChannelId, Message};
use snafu::ResultExt;
use tokio::time::sleep;
use ultimate_database::DB;
use ultimate_phrases::{NO_CHARACTER_PHRASES, NO_HISTORY_PHRASES, sample};

#[allow(clippy::unreadable_literal)] // doesn't make sense for a guild id
const PIN_CHANNEL: ChannelId = ChannelId::new(1373963823974715412);

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

    let reply = character.to_bare_response(
        message.link(),
        history.chosen_choice().chosen_revision().head().content(),
        history.chosen_choice().current_editor(),
    );

    let pin = PIN_CHANNEL
        .send_message(ctx, reply.to_prefix((&message).into()))
        .await
        .context(SendMessageSnafu)?;

    ctx.say(pin.link()).await.context(SendMessageSnafu)?;
    Ok(())
}
