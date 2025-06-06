pub mod commands;
mod event_handler;
mod traits;

pub use event_handler::event_handler;

pub type Result<T, E = Error> = std::result::Result<T, E>;
pub type Context<'a> = poise::Context<'a, (), miette::Report>;
// pub type Result<T, E = Error> = std::result::Result<T, E>;
// type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
// pub type Error = miette::Report;

pub use traits::DeferEphemeralOrBroadcast;
pub use traits::DeleteInvokingMessageIfPrefix;
pub use traits::RespondToWith;

use std::time::Duration;
pub const FIVE_SECONDS: Duration = Duration::from_secs(5);
pub const ONE_MINUTE: Duration = Duration::from_secs(60);
pub const TEN_MINUTES: Duration = Duration::from_secs(60 * 10);
pub const ONE_HOUR: Duration = Duration::from_secs(60 * 60);

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
}
