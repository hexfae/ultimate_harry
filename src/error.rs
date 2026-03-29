//! Global error types for Ultimate Harry.

use core::error::Error;
use core::fmt::Display;
use miette::Diagnostic;
use rig::agent::StreamingError;
use snafu::{Location, Snafu};

use crate::{database::DatabaseError, events::interaction::UnknownInteraction, llm::LlmError};

/// The central error type for the entire bot.
#[derive(Debug, Snafu)]
#[snafu(visibility(pub))]
pub enum AppError {
    /// Sending a message failed.
    #[snafu(display("Kunde inte skicka meddelande"))]
    SendMessage {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Sending a response failed.
    #[snafu(display("Kunde inte skicka interaktionssvar"))]
    SendResponse {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Editing a response failed.
    #[snafu(display("Kunde inte redigera interaktionssvar"))]
    EditResponse {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Editing a message failed.
    #[snafu(display("Kunde inte redigera meddelande"))]
    EditMessage {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Deleting a message failed.
    #[snafu(display("Kunde inte ta bort meddelande"))]
    DeleteMessage {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Deleting a response failed.
    #[snafu(display("Kunde inte ta bort svar"))]
    DeleteResponse {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Retrieving a message failed.
    #[snafu(display("Kunde inte hämta meddelande"))]
    RetrieveMessage {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Showing a modal failed.
    #[snafu(transparent)]
    UnknownInteraction {
        /// The source of the error.
        source: UnknownInteraction,
    },
    /// Showing a modal failed.
    #[snafu(display("Kunde inte visa modal"))]
    ShowModal {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Reacting to a message failed.
    #[snafu(display("Kunde inte reagera på meddelande"))]
    React {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Registering a command failed.
    #[snafu(display("Kunde inte registrera kommando i servern"))]
    RegisterCommand {
        /// The source of the error.
        source: serenity::Error,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Using the LLM client failed.
    #[snafu(transparent)]
    Llm {
        /// The source of the error.
        source: LlmError,
    },
    /// Streaming a reply failed.
    #[snafu(display("Strömning misslyckades"))]
    Streaming {
        /// The source of the error.
        source: StreamingError,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
    /// Interacting with the database failed.
    #[snafu(transparent)]
    Database {
        /// The source of the error.
        source: DatabaseError,
    },
    /// Something else failed.
    #[snafu(whatever, display("{message}"))]
    Whatever {
        /// The message of the error.
        message: String,
        /// The source of the error.
        #[snafu(source(from(Box<dyn Error + Send + Sync>, Some)))]
        source: Option<Box<dyn Error + Send + Sync>>,
        /// The location of the error.
        #[snafu(implicit)]
        location: Location,
    },
}

impl Diagnostic for AppError {
    fn code<'a>(&'a self) -> Option<Box<dyn Display + 'a>> {
        let (variant_name, loc) = match self {
            Self::Database { source } => {
                return source.code();
            }
            Self::UnknownInteraction { source } => {
                return source.code();
            }
            Self::Llm { source, .. } => return source.code(),
            Self::SendMessage { location, .. } => ("send_message", location),
            Self::EditMessage { location, .. } => ("edit_message", location),
            Self::RetrieveMessage { location, .. } => ("retrieve_message", location),
            Self::React { location, .. } => ("react", location),
            Self::DeleteMessage { location, .. } => ("delete_message", location),
            Self::SendResponse { location, .. } => ("send_response", location),
            Self::EditResponse { location, .. } => ("edit_response", location),
            Self::DeleteResponse { location, .. } => ("delete_response", location),
            Self::ShowModal { location, .. } => ("show_modal", location),
            Self::Streaming { location, .. } => ("streaming", location),
            Self::RegisterCommand { location, .. } => ("register_command", location),
            Self::Whatever { location, .. } => ("whatever", location),
        };

        let file = loc.file().replace('\\', "/");
        let module = file
            .strip_prefix("src/")
            .unwrap_or(&file)
            .strip_suffix(".rs")
            .unwrap_or(&file)
            .replace('/', "::");

        Some(Box::new(format!("{module}::{variant_name}")))
    }

    fn help<'a>(&'a self) -> Option<Box<dyn Display + 'a>> {
        match self {
            Self::Database { source } => source.help(),
            Self::UnknownInteraction { source } => source.help(),
            Self::SendMessage { .. } => Some(Box::new("Meddelandet kan ha varit för långt")),
            Self::EditMessage { .. } | Self::RetrieveMessage { .. } | Self::React { .. } => {
                Some(Box::new("Meddelandet kan vara borta"))
            }
            Self::DeleteMessage { .. } => Some(Box::new("Meddelandet kan redan vara borta")),
            Self::SendResponse { .. } | Self::EditResponse { .. } | Self::ShowModal { .. } => {
                Some(Box::new("Interaktionen kan ha gått ut"))
            }
            Self::DeleteResponse { .. } => Some(Box::new(
                "Interaktionen kan ha gått ut eller redan vara borta",
            )),
            Self::Llm { .. } | Self::Streaming { .. } => {
                Some(Box::new("Förmodligen OpenRouter's fel, försök igen"))
            }
            Self::RegisterCommand { .. } | Self::Whatever { .. } => Some(Box::new("¯\\_(ツ)_/¯")),
        }
    }
}
