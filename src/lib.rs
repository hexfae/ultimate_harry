pub mod app_state;
pub mod commands;
pub mod constants;
pub mod db;
pub mod events;
pub mod llm;
pub mod models;
pub mod traits;

pub type Result<T, E = Error> = std::result::Result<T, E>;
pub type Context<'a> = poise::Context<'a, app_state::AppState, miette::Report>;

use serde::{Deserialize, Serialize};
use snafu::Snafu;
use std::time::Duration;
pub use traits::{DeferEphemeralOrBroadcast, DeleteInvokingMessageIfPrefix, RespondToWith};

pub const CHARACTER_LIMIT: usize = 3900;
pub const FIVE_SECONDS: Duration = Duration::from_secs(5);
pub const ONE_MINUTE: Duration = Duration::from_mins(1);
pub const TEN_MINUTES: Duration = Duration::from_mins(10);
pub const ONE_HOUR: Duration = Duration::from_hours(1);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
    pub model: String,
    pub api_key: String,
    pub frequency_penalty: f32,
    pub presence_penalty: f32,
    pub temperature: f32,
    pub top_p: f32,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            model: "deepseek/deepseek-v3.2".to_owned(),
            api_key: String::new(),
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            temperature: 1.0,
            top_p: 0.95,
        }
    }
}

#[derive(Debug, Snafu, miette::Diagnostic)]
pub enum Error {
    #[snafu(display("Kunde inte skicka ett meddelande: {source}"))]
    SendMessage {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Kunde inte redigera ett meddelande: {source}"))]
    EditMessage {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Kunde inte ta bort ett meddelande: {source}"))]
    DeleteMessage {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Kunde inte hämta ett meddelande: {source}"))]
    RetrieveMessage {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Kunde inte skicka ett svar: {source}"))]
    SendResponse {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Kunde inte redigera svar: {source}"))]
    EditResponse {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Kunde inte ta bort ett svar: {source}"))]
    DeleteResponse {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Kunde inte skjuta upp (defer): {source}"))]
    Defer {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Kunde inte visa modalen: {source}"))]
    ShowModal {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Kunde inte reagera med en emoji: {source}"))]
    React {
        source: poise::serenity_prelude::Error,
    },
    #[snafu(display("Okänd interaction: {}", found))]
    UnknownInteraction { found: String },
    #[snafu(display("Kunde inte generera AI-svar: {source}"))]
    LlmGeneration {
        source: rig::completion::PromptError,
    },
}
