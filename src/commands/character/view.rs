//! The bot's Discord slash command for viewing characters.

use crate::{
    AppResult, Context,
    commands::{autocomplete, character::paginate::browse},
};

/// Visar en gubbe.
#[poise::command(slash_command, rename = "visa")]
pub async fn view(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: Option<String>,
) -> AppResult {
    let characters = match name {
        Some(character_name) => ctx.data().db.characters_by_similarity(character_name).await,
        None => ctx.data().db.characters_by_usage().await,
    }?;
    browse(ctx, characters).await
}
