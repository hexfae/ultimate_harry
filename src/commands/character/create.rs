//! The bot's Discord slash command for creating characters.

use crate::{
    AppResult, ApplicationContext, Context,
    components::single_button_row,
    constants::TRANSIENT_LINGER,
    error::{DeleteMessageSnafu, SendMessageSnafu, ShowModalSnafu},
    models::character::Character,
    phrases::{click_below, click_me, created},
    traits::SayEphemeral as _,
};
use alloc::borrow::Cow;
use poise::{
    CreateReply, Modal, ReplyHandle, execute_modal, execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt as _;
use tokio::time::sleep;

/// Skapar en ny gubbe.
#[poise::command(slash_command, rename = "skapa")]
pub async fn create(ctx: ApplicationContext<'_>) -> AppResult {
    let Some(first_modal) = execute_modal(ctx, None, None)
        .await
        .context(ShowModalSnafu)?
    else {
        return Ok(());
    };
    let tempting_message = ctx
        .send(create_reply_with_tempting_button(ctx.id().to_string()))
        .await
        .context(SendMessageSnafu)?;
    let Some(second_modal) = show_modal_on_button_press(ctx, tempting_message).await? else {
        return Ok(());
    };

    let character = Character::from((first_modal, second_modal, ctx.author().id));

    let success_message = ctx
        .say_ephemeral(created(&character))
        .await
        .context(SendMessageSnafu)?;
    sleep(TRANSIENT_LINGER).await;
    success_message
        .delete(Context::Application(ctx))
        .await
        .context(DeleteMessageSnafu)?;

    ctx.data().db.insert_character(character).await?;

    Ok(())
}

/// Create a collector that listens for a button press with the context's id.
/// The tempting-button message is ephemeral, so only the author can press it.
#[must_use]
async fn create_collector(ctx: ApplicationContext<'_>) -> Option<ComponentInteraction> {
    ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(FixedArray::from_vec_trunc(vec![
            FixedString::from_string_trunc(ctx.id().to_string()),
        ]))
        .await
}

/// Waits for the user to press the tempting button and shows them the modal.
async fn show_modal_on_button_press<M: Modal>(
    ctx: ApplicationContext<'_>,
    msg: ReplyHandle<'_>,
) -> AppResult<Option<M>> {
    if let Some(interaction) = create_collector(ctx).await {
        msg.delete(Context::Application(ctx))
            .await
            .context(DeleteMessageSnafu)?;
        execute_modal_on_component_interaction(ctx.serenity_context(), interaction, None, None)
            .await
            .context(ShowModalSnafu)
    } else {
        Ok(None)
    }
}

/// Sends a message that attempts to tempt the user into pressing it.
#[must_use]
fn create_reply_with_tempting_button<'a>(id: impl Into<Cow<'a, str>>) -> CreateReply<'a> {
    CreateReply::default()
        .content(click_below())
        .components(single_button_row(id, click_me()))
        .ephemeral(true)
}
