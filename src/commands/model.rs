//! The bot's Discord slash command for setting various AI model settings.

use crate::{
    AppResult, Context,
    commands::{
        autocomplete_audio_model, autocomplete_model, autocomplete_provider,
        autocomplete_vision_model,
    },
    error::SendMessageSnafu,
    llm::{ModelOverrides, model_endpoints, pin_for_model},
    phrases,
    traits::SayEphemeral as _,
};
use snafu::ResultExt as _;

/// Ställer in AI-modellens inställningar.
#[poise::command(slash_command, rename = "modell")]
pub async fn model(
    ctx: Context<'_>,
    #[rename = "modell"]
    #[description = "Modellen att använda"]
    #[autocomplete = autocomplete_model]
    model: Option<String>,
    #[rename = "leverantör"]
    #[description = "Leverantören att låsa modellen till"]
    #[autocomplete = autocomplete_provider]
    provider: Option<String>,
    #[rename = "api-nyckel"]
    #[description = "API-nyckeln att använda"]
    api_key: Option<String>,
    #[rename = "temperatur"]
    #[description = "Temperaturen (högre = mer slumpmässig)"]
    temperature: Option<f32>,
    #[rename = "syn-modell"]
    #[description = "Modellen som beskriver bilder för modeller utan syn"]
    #[autocomplete = autocomplete_vision_model]
    vision_model: Option<String>,
    #[rename = "ljud-modell"]
    #[description = "Modellen som transkriberar ljud för modeller utan ljud"]
    #[autocomplete = autocomplete_audio_model]
    audio_model: Option<String>,
) -> AppResult {
    let overrides = ModelOverrides {
        model,
        api_key,
        temperature,
        provider,
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
    let switched_model = overrides.model.is_some();
    model_settings.apply_overrides(overrides);
    // a provider slug names one endpoint of one model, so switching models
    // leaves a pin the new model does not serve behind, failing every request
    if switched_model {
        let endpoints = model_endpoints(&model_settings.model).await;
        model_settings.provider =
            pin_for_model(model_settings.provider.as_deref(), &endpoints).map(str::to_owned);
    }
    ctx.data().db.upsert_model_settings(model_settings).await?;
    ctx.say_ephemeral(phrases::done())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
