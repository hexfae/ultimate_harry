//! The bot's Discord slash command for setting various AI model settings.

use crate::{AppResult, Context, error::SendMessageSnafu, traits::SayEphemeral as _};
use snafu::ResultExt as _;

/// The bot's Discord slash command for setting various AI model settings.
#[poise::command(slash_command)]
pub async fn model(
    ctx: Context<'_>,
    model: Option<String>,
    api_key: Option<String>,
    frequency_penalty: Option<f32>,
    presence_penalty: Option<f32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
) -> AppResult {
    let mut model_settings = ctx.data().db.model_settings().await;
    if let Some(new_model) = model {
        model_settings.model = new_model;
    }
    if let Some(new_api_key) = api_key {
        model_settings.api_key = new_api_key;
    }
    if let Some(new_frequency_penalty) = frequency_penalty {
        model_settings.frequency_penalty = new_frequency_penalty;
    }
    if let Some(new_presence_penalty) = presence_penalty {
        model_settings.presence_penalty = new_presence_penalty;
    }
    if let Some(new_temperature) = temperature {
        model_settings.temperature = new_temperature;
    }
    if let Some(new_top_p) = top_p {
        model_settings.top_p = new_top_p;
    }
    ctx.data().db.upsert_model_settings(model_settings).await?;
    ctx.say_ephemeral("done").await.context(SendMessageSnafu)?;
    Ok(())
}
