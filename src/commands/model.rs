//! The bot's Discord slash command for setting various AI model settings.

use crate::{
    AppResult, Context, error::SendMessageSnafu, llm::ModelOverrides, phrases,
    traits::SayEphemeral as _,
};
use snafu::ResultExt as _;

/// Ställer in AI-modellens inställningar.
#[poise::command(slash_command, rename = "modell")]
pub async fn model(
    ctx: Context<'_>,
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
    let overrides = ModelOverrides {
        model,
        api_key,
        temperature,
        vision_model,
        audio_model,
    };
    let mut model_settings = ctx.data().db.model_settings().await;
    if overrides.is_empty() {
        ctx.say_ephemeral(model_settings.summary())
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    }
    model_settings.apply_overrides(overrides);
    ctx.data().db.upsert_model_settings(model_settings).await?;
    ctx.say_ephemeral(phrases::done())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
