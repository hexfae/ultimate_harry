//! The bot's Discord slash command for restoring deleted characters.

use crate::{
    AppResult, Context,
    commands::{
        autocomplete_deleted,
        character::paginate::{notify_cancelled, paginate_deleted},
    },
    constants::{RESTORE, TRANSIENT_LINGER},
    error::{DeleteResponseSnafu, SendMessageSnafu, SendResponseSnafu},
    events::interaction::InteractionKind,
    models::character::Character,
    phrases::{ask_restore, restored},
    traits::RespondToWith as _,
};
use poise::serenity_prelude::{
    ComponentInteraction, ComponentInteractionCollector,
    small_fixed_array::{FixedArray, FixedString},
};
use snafu::ResultExt as _;
use tokio::time::sleep;

/// Återupplivar en gubbe.
#[poise::command(slash_command, rename = "återuppliva")]
pub async fn restore(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete_deleted]
    name: String,
) -> AppResult {
    paginate_deleted(ctx, name, RESTORE, confirm_restoration).await
}

/// Asks the user to confirm restoring `character`, then restores it or cancels.
async fn confirm_restoration(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    character: Character,
) -> AppResult {
    let confirm_id = InteractionKind::Confirm.custom_id(ctx.id());
    let name = character.to_string();

    let Some(response) = ask_for_confirmation(ctx, interaction, &name).await? else {
        return Ok(());
    };

    if response.data.custom_id == confirm_id {
        restore_confirmed(ctx, response, character.id(), &name).await
    } else {
        notify_cancelled(ctx, response).await
    }
}

/// Sends a message asking the user for confirmation on restoring the character.
async fn ask_for_confirmation(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    name: &str,
) -> AppResult<Option<ComponentInteraction>> {
    let reply = Character::to_confirm_interaction_response(ctx.id(), ask_restore(name));
    interaction
        .create_response(ctx.http(), reply)
        .await
        .context(SendResponseSnafu)?;

    let id = ctx.id();
    let confirm_id = InteractionKind::Confirm.custom_id(id);
    let cancel_id = InteractionKind::Cancel.custom_id(id);

    Ok(ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(FixedArray::from_vec_trunc(vec![
            FixedString::from_string_trunc(confirm_id.clone()),
            FixedString::from_string_trunc(cancel_id.clone()),
        ]))
        .await)
}

/// Restores the character, sends a message stating it was restored, and deletes the message 5 seconds later.
async fn restore_confirmed(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    id: &str,
    name: &str,
) -> AppResult {
    ctx.data().db.restore_character(id).await?;

    ctx.respond_to_with(&interaction, restored(name))
        .await
        .context(SendMessageSnafu)?;
    sleep(TRANSIENT_LINGER).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
}
