//! The bot's Discord slash command for setting various AI model settings for a specific character.

use crate::{
    AppResult, Context,
    commands::{
        autocomplete, autocomplete_character_provider, autocomplete_model,
        first_character_or_notify,
    },
    error::SendMessageSnafu,
    llm::{model_endpoints, pin_for_model},
    phrases,
    traits::SayEphemeral as _,
};
use snafu::ResultExt as _;

/// Ställer in en gubbes AI-modellinställningar.
///
/// Only the model, its provider pin, and the temperature can be overridden per
/// character; the API key and the vision/audio fallback models always come from
/// the global `/modell` settings.
#[poise::command(slash_command, rename = "modell")]
pub async fn model(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
    #[rename = "modell"]
    #[description = "Modellen att använda"]
    #[autocomplete = autocomplete_model]
    model: Option<String>,
    #[rename = "leverantör"]
    #[description = "Leverantören att låsa modellen till"]
    #[autocomplete = autocomplete_character_provider]
    provider: Option<String>,
    #[rename = "temperatur"]
    #[description = "Temperaturen (högre = mer slumpmässig)"]
    temperature: Option<f32>,
) -> AppResult {
    let db = &ctx.data().db;
    let Some(character) = first_character_or_notify(ctx, name).await? else {
        return Ok(());
    };

    if model.is_none() && temperature.is_none() && provider.is_none() {
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
    let switched_model = model.is_some();
    overrides.model = model.or(overrides.model);
    overrides.temperature = temperature.or(overrides.temperature);
    if let Some(chosen) = provider {
        overrides.provider = Some(chosen);
    }
    // a provider slug names one endpoint of one model, so switching models
    // leaves a pin the new model does not serve behind, failing every request
    if switched_model && let Some(new_model) = overrides.model.clone() {
        let endpoints = model_endpoints(&new_model).await;
        overrides.provider =
            pin_for_model(overrides.provider.as_deref(), &endpoints).map(str::to_owned);
    }
    db.set_character_model_settings(character.id(), overrides)
        .await?;
    ctx.say_ephemeral(phrases::done())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}
