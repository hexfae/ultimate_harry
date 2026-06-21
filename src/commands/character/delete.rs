//! The bot's Discord slash command for deleting characters.

use crate::{
    AppResult, Context,
    commands::{
        autocomplete,
        character::paginate::{notify_cancelled, paginate},
    },
    constants::DELETE,
    error::{DeleteResponseSnafu, SendMessageSnafu, SendResponseSnafu},
    events::interaction::InteractionKind,
    models::character::Character,
    phrases::{ask_delete, deleted},
    traits::RespondToWith as _,
};
use core::time::Duration;
use poise::serenity_prelude::{
    ComponentInteraction, ComponentInteractionCollector,
    small_fixed_array::{FixedArray, FixedString},
};
use snafu::ResultExt as _;
use tokio::time::sleep;

/// Dödar en gubbe.
#[poise::command(slash_command, rename = "döda")]
pub async fn delete(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
) -> AppResult {
    paginate(ctx, name, DELETE, confirm_deletion).await
}

/// Asks the user to confirm deleting `character`, then deletes it or cancels.
async fn confirm_deletion(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    character: Character,
) -> AppResult {
    let confirm_id = InteractionKind::Confirm.custom_id(ctx.id());

    let Some(response) = ask_for_confirmation(ctx, interaction).await? else {
        return Ok(());
    };

    if response.data.custom_id == confirm_id {
        delete_confirmed(ctx, response, character.id()).await
    } else {
        notify_cancelled(ctx, response).await
    }
}

/// Sends a message asking the user for confirmation on deleting the character.
async fn ask_for_confirmation(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> AppResult<Option<ComponentInteraction>> {
    let reply = Character::to_confirm_interaction_response(ctx.id(), ask_delete());
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

/// Deletes the character, sends a message stating it was deleted, and deletes the message 5 seconds later.
async fn delete_confirmed(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    id: &str,
) -> AppResult {
    ctx.data().db.delete_character(id, ctx.author()).await?;

    ctx.respond_to_with(&interaction, deleted())
        .await
        .context(SendMessageSnafu)?;
    sleep(Duration::from_secs(5)).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
}
