//! The bot's Discord slash command for setting various AI model settings.

use crate::{AppResult, Context, error::SendMessageSnafu, traits::SayEphemeral as _};
use snafu::ResultExt as _;

/// The bot's Discord slash command for setting various AI model settings.
#[poise::command(slash_command)]
pub async fn model(
    ctx: Context<'_>,
    model: Option<String>,
    api_key: Option<String>,
    temperature: Option<f32>,
) -> AppResult {
    let mut model_settings = ctx.data().db.model_settings().await;
    model_settings.apply_overrides(model, api_key, temperature);
    ctx.data().db.upsert_model_settings(model_settings).await?;
    ctx.say_ephemeral("done").await.context(SendMessageSnafu)?;
    Ok(())
}
