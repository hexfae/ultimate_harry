use std::fmt::Display;

use crate::{
    Context, DeferEphemeralOrBroadcast, EditMessageSnafu, FIVE_SECONDS, ONE_MINUTE, Result,
    SendMessageSnafu,
    traits::{DeleteSelfAndInvokingMessageIfPrefix, EditWith, ShowModal},
};
use miette::Report;
use poise::{
    CreateReply, Modal, ReplyHandle,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, CreateActionRow, CreateButton,
    },
};
use snafu::ResultExt;
use tokio::time::sleep;
use ultimate_character::{CHARACTERS, Character};
use ultimate_modals::{CreateCharacterModal, SecondCreateCharacterModal};
use ultimate_phrases::{
    CLICK_BELOW_PHRASES, CLICK_ME_PHRASES, CREATED_PHRASES, TIMEOUT_PHRASES, sample, sample_name,
};
use ultimate_statistics::STATISTICS;

#[poise::command(slash_command, prefix_command, rename = "skapa")]
pub async fn create(ctx: Context<'_>) -> Result<(), Report> {
    let msg = send_initial_message(ctx).await?;
    let Some(first_modal): Option<CreateCharacterModal> = show_modal_button(ctx, &msg).await?
    else {
        return Ok(());
    };
    edit_message(ctx, &msg).await?;
    let Some(second_modal): Option<SecondCreateCharacterModal> =
        show_modal_button(ctx, &msg).await?
    else {
        return Ok(());
    };

    let character = Character::from((first_modal, second_modal, ctx.author().id));

    let response = sample_name(CREATED_PHRASES, character.name());
    msg.edit_with(ctx, response).await?;

    CHARACTERS.write().insert(character);
    STATISTICS.write().character_created_by(ctx.author());

    sleep(FIVE_SECONDS).await;
    msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
    Ok(())
}

async fn send_initial_message(ctx: Context<'_>) -> Result<ReplyHandle<'_>> {
    ctx.defer_ephemeral_or_broadcast().await?;
    ctx.send(create_reply_with_tempting_button(ctx.id()))
        .await
        .context(SendMessageSnafu)
}

async fn edit_message(ctx: Context<'_>, msg: &ReplyHandle<'_>) -> Result<()> {
    msg.edit(ctx, create_reply_with_tempting_button(ctx.id()))
        .await
        .context(EditMessageSnafu)
}

#[must_use]
async fn await_button_interaction(ctx: Context<'_>) -> Option<ComponentInteraction> {
    ComponentInteractionCollector::new(ctx)
        .author_id(ctx.author().id)
        .custom_ids(vec![ctx.id().to_string()])
        .timeout(ONE_MINUTE)
        .await
}

async fn show_modal_button<M: Modal>(ctx: Context<'_>, msg: &ReplyHandle<'_>) -> Result<Option<M>> {
    ctx.defer_ephemeral_or_broadcast().await?;

    if let Some(interaction) = await_button_interaction(ctx).await {
        ctx.show_modal(interaction).await
    } else {
        let response = sample(TIMEOUT_PHRASES);
        msg.edit_with(ctx, response).await?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        Ok(None)
    }
}

#[must_use]
fn create_reply_with_tempting_button(id: impl Display) -> CreateReply {
    let id = id.to_string();
    let click_me = sample(CLICK_ME_PHRASES);
    let click_below = sample(CLICK_BELOW_PHRASES);
    let button = vec![CreateButton::new(id).label(click_me)];
    let component = vec![CreateActionRow::Buttons(button)];

    CreateReply::default()
        .content(click_below)
        .components(component)
}
