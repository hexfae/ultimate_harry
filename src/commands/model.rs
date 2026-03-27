//! The bot's Discord slash command for setting various AI model settings.

use crate::{Context, database::DatabaseError, traits::SayEphemeral as _};
use miette::{Diagnostic, Result};
use snafu::{ResultExt as _, Snafu};

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
) -> Result<()> {
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
    ctx.data()
        .db
        .upsert_model_settings(model_settings)
        .await
        .context(SaveSettingsSnafu)?;
    ctx.say_ephemeral("done").await.context(SendMessageSnafu)?;
    Ok(())
}

/// All errors that can happen when changing model settings.
#[derive(Debug, Snafu, Diagnostic)]
enum ModelSettingsError {
    /// Sending a message failed.
    #[snafu(display("Kunde inte skicka meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(commands::model::send_message)
    )]
    SendMessage {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Saving the model settings to the database failed.
    #[snafu(display("Kunde inte spara modellinställningar: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att inställningarna är giltiga"),
        code(commands::model::save_settings)
    )]
    SaveSettings {
        /// The source of the error.
        source: DatabaseError,
    },
}
