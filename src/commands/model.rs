//! The bot's Discord slash command for setting various AI model settings.

use crate::{AppResult, Context, error::SendMessageSnafu, traits::SayEphemeral as _};
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
) -> AppResult {
    let mut model_settings = ctx.data().db.model_settings().await;
    model_settings.apply_overrides(model, api_key, temperature);
    ctx.data().db.upsert_model_settings(model_settings).await?;
    ctx.say_ephemeral("Klart!").await.context(SendMessageSnafu)?;
    Ok(())
}
