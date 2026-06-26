//! The shared two-step modal prompt used by the character create and edit
//! commands.
//!
//! The first modal opens immediately as the command's response; the second opens
//! behind a tempting button, since Discord refuses to open a second modal without
//! a fresh interaction to hang it on.

use crate::{
    AppResult, ApplicationContext, Context,
    error::{DeleteMessageSnafu, SendMessageSnafu, ShowModalSnafu},
    phrases::{click_below, click_me},
};
use alloc::borrow::Cow;
use poise::{
    CreateReply, Modal, ReplyHandle, execute_modal, execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, CreateActionRow, CreateButton,
        CreateComponent,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt as _;

/// Shows two modals back to back and returns both once submitted.
///
/// The first modal opens immediately as the response to the command, pre-filled
/// from `first`. The second opens behind a tempting button (Discord refuses to
/// open a second modal without a fresh interaction), pre-filled from `second`.
/// Returns `None` if the user dismisses either modal.
#[expect(
    clippy::module_name_repetitions,
    reason = "this is the two-modal prompt, so the shared suffix is meaningful"
)]
pub async fn prompt_two_modals<M1: Modal, M2: Modal>(
    ctx: ApplicationContext<'_>,
    first: Option<M1>,
    second: Option<M2>,
) -> AppResult<Option<(M1, M2)>> {
    let Some(first_modal) = execute_modal(ctx, first, None)
        .await
        .context(ShowModalSnafu)?
    else {
        return Ok(None);
    };
    let tempting_message = ctx
        .send(tempting_button_reply(ctx.id().to_string()))
        .await
        .context(SendMessageSnafu)?;
    let Some(second_modal) = show_modal_on_button_press(ctx, tempting_message, second).await? else {
        return Ok(None);
    };
    Ok(Some((first_modal, second_modal)))
}

/// Waits for the user to press the tempting button and shows them the second
/// modal, pre-filled from `defaults`, deleting the tempting message first.
async fn show_modal_on_button_press<M: Modal>(
    ctx: ApplicationContext<'_>,
    msg: ReplyHandle<'_>,
    defaults: Option<M>,
) -> AppResult<Option<M>> {
    if let Some(interaction) = button_collector(ctx).await {
        msg.delete(Context::Application(ctx))
            .await
            .context(DeleteMessageSnafu)?;
        execute_modal_on_component_interaction(ctx.serenity_context(), interaction, defaults, None)
            .await
            .context(ShowModalSnafu)
    } else {
        Ok(None)
    }
}

/// Creates a collector that listens for the tempting button keyed on the
/// context's id. The tempting message is ephemeral, so only the author can press
/// it.
#[must_use]
async fn button_collector(ctx: ApplicationContext<'_>) -> Option<ComponentInteraction> {
    ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(FixedArray::from_vec_trunc(vec![
            FixedString::from_string_trunc(ctx.id().to_string()),
        ]))
        .await
}

/// Builds the ephemeral message whose button tempts the user into opening the
/// second modal.
#[must_use]
fn tempting_button_reply<'a>(id: impl Into<Cow<'a, str>>) -> CreateReply<'a> {
    CreateReply::default()
        .content(click_below())
        .components(single_button_row(id, click_me()))
        .ephemeral(true)
}

/// Builds a single-button action row labelled `label` and keyed on `custom_id`.
fn single_button_row<'a, I, L>(custom_id: I, label: L) -> Vec<CreateComponent<'a>>
where
    I: Into<Cow<'a, str>>,
    L: Into<Cow<'a, str>>,
{
    let button = vec![CreateButton::new(custom_id).label(label)].into();
    vec![CreateComponent::ActionRow(CreateActionRow::Buttons(button))]
}
