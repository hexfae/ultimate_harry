use crate::{Context, DeferSnafu, SendMessageSnafu};
use miette::Report;
use snafu::ResultExt;

#[poise::command(slash_command)]
pub async fn model(
    ctx: Context<'_>,
    model: Option<String>,
    api_key: Option<String>,
    frequency_penalty: Option<f32>,
    presence_penalty: Option<f32>,
    temperature: Option<f32>,
    top_p: Option<f32>,
) -> Result<(), Report> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;
    let mut model_settings = ctx.data().db.model_settings().await;
    if let Some(model) = model {
        model_settings.model = model;
    }
    if let Some(api_key) = api_key {
        model_settings.api_key = api_key;
    }
    if let Some(frequency_penalty) = frequency_penalty {
        model_settings.frequency_penalty = frequency_penalty;
    }
    if let Some(presence_penalty) = presence_penalty {
        model_settings.presence_penalty = presence_penalty;
    }
    if let Some(temperature) = temperature {
        model_settings.temperature = temperature;
    }
    if let Some(top_p) = top_p {
        model_settings.top_p = top_p;
    }
    ctx.data().db.upsert_model_settings(model_settings).await?;
    ctx.say("done").await.context(SendMessageSnafu)?;
    Ok(())
}
