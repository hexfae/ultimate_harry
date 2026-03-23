use std::{borrow::Cow, fmt::Display};

use crate::{
    Context, DeferSnafu, EditMessageSnafu, Result, SendMessageSnafu,
    constants::{
        CLICK_BELOW_PHRASES, CLICK_ME_PHRASES, CREATED_PHRASES, TIMEOUT_PHRASES, sample,
        sample_name,
    },
    models::{
        character::Character,
        modals::{CreateCharacterModal, SecondCreateCharacterModal},
    },
    traits::{EditWith, ShowModal},
};
use miette::Report;
use poise::{
    CreateReply, Modal, ReplyHandle,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, CreateActionRow, CreateButton,
        CreateComponent,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt;

#[poise::command(slash_command, rename = "skapa")]
pub async fn create(ctx: Context<'_>) -> Result<(), Report> {
    let msg = send_initial_message(ctx).await?;
    let Some(first_modal): Option<CreateCharacterModal> = show_modal_button(ctx, &msg).await?
    else {
        return Ok(());
    };
    msg.edit(ctx, create_reply_with_tempting_button(ctx.id()))
        .await
        .context(EditMessageSnafu)?;
    let Some(second_modal): Option<SecondCreateCharacterModal> =
        show_modal_button(ctx, &msg).await?
    else {
        return Ok(());
    };

    let character = Character::from((first_modal, second_modal, ctx.author().id));

    let response = sample_name(CREATED_PHRASES, character.name());
    msg.edit_with(ctx, response).await?;

    ctx.data().db.insert_character(character).await?;
    ctx.data().stats.character_created_by(ctx.author());

    Ok(())
}

async fn send_initial_message(ctx: Context<'_>) -> Result<ReplyHandle<'_>> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;
    ctx.send(create_reply_with_tempting_button(ctx.id()))
        .await
        .context(SendMessageSnafu)
}

#[must_use]
async fn await_button_interaction(ctx: Context<'_>) -> Option<ComponentInteraction> {
    ComponentInteractionCollector::new(ctx.serenity_context())
        .author_id(ctx.author().id)
        .custom_ids(FixedArray::from_vec_trunc(vec![
            FixedString::from_string_trunc(ctx.id().to_string()),
        ]))
        .await
}

async fn show_modal_button<M: Modal>(ctx: Context<'_>, msg: &ReplyHandle<'_>) -> Result<Option<M>> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;

    if let Some(interaction) = await_button_interaction(ctx).await {
        ctx.serenity_context().show_modal(interaction).await
    } else {
        let response = sample(TIMEOUT_PHRASES);
        msg.edit_with(ctx, response).await?;
        Ok(None)
    }
}

#[must_use]
fn create_reply_with_tempting_button<'a>(id: impl Display) -> CreateReply<'a> {
    let id = id.to_string();
    let click_me = sample(CLICK_ME_PHRASES);
    let click_below = sample(CLICK_BELOW_PHRASES);
    let button = Cow::Owned(vec![CreateButton::new(id).label(click_me)]);
    let component = Cow::Owned(vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        button,
    ))]);

    CreateReply::default()
        .content(click_below)
        .components(component)
}
