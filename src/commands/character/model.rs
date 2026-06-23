//! The bot's Discord slash command for setting various AI model settings for a specific character.

use crate::{
    AppResult, Context,
    commands::{autocomplete, notify_no_character},
    error::SendMessageSnafu,
    llm::ModelOverrides,
    models::character::Character,
    traits::SayEphemeral as _,
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
    #[rename = "syn-modell"]
    #[description = "Modellen som beskriver bilder för modeller utan syn"]
    vision_model: Option<String>,
    #[rename = "ljud-modell"]
    #[description = "Modellen som transkriberar ljud för modeller utan ljud"]
    audio_model: Option<String>,
) -> AppResult {
    let db = &ctx.data().db;
    let characters: Vec<Character> = db.characters_by_similarity(name).await?;
    let Some(character) = characters.first() else {
        notify_no_character(ctx).await?;
        return Ok(());
    };

    let overrides = ModelOverrides {
        model,
        api_key,
        temperature,
        vision_model,
        audio_model,
    };
    let mut model_settings = db.resolved_model_settings(character).await;
    if overrides.is_empty() {
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
    model_settings.apply_overrides(overrides);
    db.set_character_model_settings(character.id(), model_settings)
        .await?;
    ctx.say_ephemeral("Klart!")
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
