//! The bot's Discord slash command for setting various AI model settings for a specific character.

use crate::{
    AppResult, Context,
    commands::{autocomplete, first_character_or_notify},
    error::SendMessageSnafu,
    traits::SayEphemeral as _,
};
use snafu::ResultExt as _;

/// Ställer in en gubbes AI-modellinställningar.
///
/// Only the model and temperature can be overridden per character; the API key and the
/// vision/audio fallback models always come from the global `/modell` settings.
#[poise::command(slash_command, rename = "modell")]
pub async fn model(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
    #[rename = "modell"]
    #[description = "Modellen att använda"]
    model: Option<String>,
    #[rename = "temperatur"]
    #[description = "Temperaturen (högre = mer slumpmässig)"]
    temperature: Option<f32>,
) -> AppResult {
    let db = &ctx.data().db;
    let Some(character) = first_character_or_notify(ctx, name).await? else {
        return Ok(());
    };

    if model.is_none() && temperature.is_none() {
        let model_settings = db.resolved_model_settings(&character).await;
        let scope = if character.has_model_settings() {
            "egna inställningar"
        } else {
            "ärvda globala inställningar"
        };
        ctx.say_ephemeral(format!("{}\n*({scope})*", model_settings.summary()))
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    }

    let mut overrides = character.model_settings().cloned().unwrap_or_default();
    overrides.model = model.or(overrides.model);
    overrides.temperature = temperature.or(overrides.temperature);
    db.set_character_model_settings(character.id(), overrides)
        .await?;
    ctx.say_ephemeral("Klart!")
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
