//! The bot's Discord slash command for setting various AI model settings for a specific character.

use crate::{
    AppResult, Context, commands::autocomplete, error::SendMessageSnafu,
    models::character::Character, phrases::no_character, traits::SayEphemeral as _,
};
use snafu::ResultExt as _;

/// Ställer in en gubbes AI-modellinställningar.
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
    #[rename = "api-nyckel"]
    #[description = "API-nyckeln att använda"]
    api_key: Option<String>,
    #[rename = "temperatur"]
    #[description = "Temperaturen (högre = mer slumpmässig)"]
    temperature: Option<f32>,
) -> AppResult {
    let db = &ctx.data().db;
    let characters: Vec<Character> = db.characters_by_similarity(name).await?;
    let Some(character) = characters.first() else {
        ctx.say_ephemeral(no_character())
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };

    let mut model_settings = db.resolved_model_settings(character).await;
    if model.is_none() && api_key.is_none() && temperature.is_none() {
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
    model_settings.apply_overrides(model, api_key, temperature);
    db.set_character_model_settings(character.id(), model_settings)
        .await?;
    ctx.say_ephemeral("Klart!").await.context(SendMessageSnafu)?;
    Ok(())
}
