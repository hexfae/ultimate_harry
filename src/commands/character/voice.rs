//! The bot's Discord slash command for linking an `ElevenLabs` voice to a character.

use crate::{
    AppResult, Context,
    commands::{autocomplete, first_character_or_notify},
    error::SendMessageSnafu,
    phrases,
    traits::SayEphemeral as _,
};
use snafu::ResultExt as _;

/// Kopplar en ElevenLabs-röst till en gubbe.
#[poise::command(slash_command, rename = "röst")]
pub async fn voice(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
    #[rename = "röst-id"]
    #[description = "ElevenLabs röst-ID (lämna tomt för att visa nuvarande)"]
    voice: Option<String>,
) -> AppResult {
    let db = &ctx.data().db;
    let Some(character) = first_character_or_notify(ctx, name).await? else {
        return Ok(());
    };

    let Some(new_voice) = voice else {
        let current = character.voice().unwrap_or("ingen");
        ctx.say_ephemeral(format!("röst: {current}"))
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };

    db.set_character_voice(character.id(), Some(new_voice)).await?;
    ctx.say_ephemeral(phrases::done()).await.context(SendMessageSnafu)?;
    Ok(())
}
