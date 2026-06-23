//! The bot's Discord slash command for restoring deleted characters.

use crate::{
    AppResult, Context,
    commands::{
        autocomplete_deleted,
        character::paginate::{confirm_prompt, paginate_deleted, respond_then_clear},
    },
    constants::RESTORE,
    models::character::Character,
    phrases::{ask_restore, restored},
};
use poise::serenity_prelude::ComponentInteraction;

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

/// Asks the user to confirm restoring `character`, then restores it (and clears
/// the confirmation), or cancels.
async fn confirm_restoration(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    character: Character,
) -> AppResult {
    let name = character.to_string();
    let Some(response) = confirm_prompt(ctx, interaction, ask_restore(&name)).await? else {
        return Ok(());
    };
    ctx.data().db.restore_character(character.id()).await?;
    respond_then_clear(ctx, response, restored(&name)).await
}
