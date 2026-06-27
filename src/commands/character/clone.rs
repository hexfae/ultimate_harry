//! The bot's Discord slash command for cloning a character into a fresh copy.

use crate::{
    AppResult, Context,
    commands::{autocomplete, first_character_or_notify},
    error::SendMessageSnafu,
    phrases::created,
    traits::SayEphemeral as _,
};
use snafu::ResultExt as _;

/// Klonar en gubbe.
#[poise::command(slash_command, rename = "klona")]
pub async fn clone(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
    #[rename = "nytt-namn"]
    #[description = "Den nya gubbens namn (lämna tomt för \"<namn> (kopia)\")"]
    new_name: Option<String>,
) -> AppResult {
    let db = &ctx.data().db;
    let Some(character) = first_character_or_notify(ctx, name).await? else {
        return Ok(());
    };

    let trimmed = new_name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let clone = character.duplicate(ctx.author().id, trimmed);
    let notice = created(&clone);
    db.insert_character(clone).await?;
    ctx.say_ephemeral(notice).await.context(SendMessageSnafu)?;
    Ok(())
}
