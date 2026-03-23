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

pub use traits::RespondToWith;

#[derive(Debug, snafu::Snafu, miette::Diagnostic)]
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
