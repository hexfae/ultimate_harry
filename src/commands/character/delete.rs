//! The bot's Discord slash command for deleting characters.

use crate::{
    AppResult, Context,
    commands::{
        autocomplete,
        character::paginate::{confirm_prompt, paginate, respond_then_clear},
    },
    constants::DELETE,
    models::character::Character,
    phrases::{ask_delete, deleted},
};
use poise::serenity_prelude::ComponentInteraction;

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

/// Asks the user to confirm deleting `character`, then deletes it (and clears
/// the confirmation), or cancels.
async fn confirm_deletion(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    character: Character,
) -> AppResult {
    let Some(response) = confirm_prompt(ctx, interaction, ask_delete()).await? else {
        return Ok(());
    };
    ctx.data()
        .db
        .delete_character(character.id(), ctx.author())
        .await?;
    respond_then_clear(ctx, response, deleted()).await
}
