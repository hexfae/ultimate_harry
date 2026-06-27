//! Global error types for Ultimate Harry.

use core::fmt::Display;
use miette::Diagnostic;
use rig::agent::StreamingError;
use snafu::{Location, Snafu};

use crate::{
    database::DatabaseError, events::interaction::UnknownInteraction, llm::LlmError,
    tts::TtsError,
};

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
    /// A component interaction's `custom_id` could not be parsed.
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
    /// Using the text-to-speech client failed.
    #[snafu(transparent)]
    Tts {
        /// The source of the error.
        source: TtsError,
    },
    /// Streaming a reply failed.
    #[snafu(display("Kunde inte strömma svaret"))]
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
}

impl AppError {
    /// The Swedish, user-facing rendering of this error: its display message, its
    /// help hint when present, and a uniform retry line when the failure is
    /// [`retryable`](Self::retryable), each on its own small line.
    ///
    /// Unlike [`report_error`](crate::util::report_error) (which is for
    /// logs), this carries no diagnostic code, source location, or source chain,
    /// so it can be shown to a user as a clean error notice.
    #[must_use]
    pub fn user_message(&self) -> String {
        let help = Diagnostic::help(self)
            .map(|help| format!("\n-# {help}"))
            .unwrap_or_default();
        let retry = if self.retryable() {
            "\n-# Du kan försöka igen."
        } else {
            ""
        };
        format!("{self}{help}{retry}")
    }

    /// Whether retrying the same action might succeed (a transient Discord, network,
    /// or storage failure) rather than being a permanent condition (a stale button,
    /// a missing record, or a misconfiguration). Drives the retry hint in
    /// [`user_message`](Self::user_message).
    #[must_use]
    pub const fn retryable(&self) -> bool {
        match self {
            Self::Database { source } => source.retryable(),
            Self::Llm { source } => source.retryable(),
            Self::Tts { source } => source.retryable(),
            Self::SendMessage { .. }
            | Self::SendResponse { .. }
            | Self::EditResponse { .. }
            | Self::EditMessage { .. }
            | Self::DeleteMessage { .. }
            | Self::DeleteResponse { .. }
            | Self::RetrieveMessage { .. }
            | Self::ShowModal { .. }
            | Self::Streaming { .. } => true,
            Self::UnknownInteraction { .. } | Self::RegisterCommand { .. } => false,
        }
    }
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
            Self::Tts { source, .. } => return source.code(),
            Self::SendMessage { location, .. } => ("send_message", location),
            Self::EditMessage { location, .. } => ("edit_message", location),
            Self::RetrieveMessage { location, .. } => ("retrieve_message", location),
            Self::DeleteMessage { location, .. } => ("delete_message", location),
            Self::SendResponse { location, .. } => ("send_response", location),
            Self::EditResponse { location, .. } => ("edit_response", location),
            Self::DeleteResponse { location, .. } => ("delete_response", location),
            Self::ShowModal { location, .. } => ("show_modal", location),
            Self::Streaming { location, .. } => ("streaming", location),
            Self::RegisterCommand { location, .. } => ("register_command", location),
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
            Self::SendMessage { .. } => Some(Box::new("Meddelandet kan ha varit för långt.")),
            Self::EditMessage { .. } | Self::RetrieveMessage { .. } => {
                Some(Box::new("Meddelandet kan vara borta."))
            }
            Self::DeleteMessage { .. } => Some(Box::new("Meddelandet kan redan vara borta.")),
            Self::SendResponse { .. } | Self::EditResponse { .. } | Self::ShowModal { .. } => {
                Some(Box::new("Interaktionen kan ha gått ut."))
            }
            Self::DeleteResponse { .. } => Some(Box::new(
                "Interaktionen kan ha gått ut eller redan vara borta.",
            )),
            Self::Llm { source } => source.help(),
            Self::Streaming { .. } => Some(Box::new("Förmodligen ett fel hos OpenRouter.")),
            Self::Tts { source } => source.help(),
            Self::RegisterCommand { .. } => Some(Box::new("¯\\_(ツ)_/¯")),
        }
    }
}

/// Tests for the user-facing error rendering and the retryable classification.
#[cfg(test)]
mod tests {
    use super::{AppError, RegisterCommandSnafu, SendMessageSnafu};
    use crate::events::interaction::InteractionKind;
    use crate::llm::LlmError;
    use crate::tts::TtsError;
    use snafu::IntoError as _;
    use std::io::Error as IoError;

    /// A permanent failure (no vision model configured) is not retryable and its
    /// user message shows the display and help but no retry hint.
    #[test]
    fn permanent_error_has_no_retry_hint() {
        let error = AppError::Llm {
            source: LlmError::NoVisionModel,
        };
        assert!(!error.retryable(), "a missing vision model is permanent");
        let message = error.user_message();
        assert!(
            message.contains("Ingen synmodell"),
            "the display message is shown"
        );
        assert!(
            !message.contains("Du kan försöka igen"),
            "a permanent error offers no retry hint"
        );
    }

    /// A transient failure (an empty vision description) is retryable and its user
    /// message ends with the uniform retry hint.
    #[test]
    fn transient_error_has_retry_hint() {
        let error = AppError::Llm {
            source: LlmError::EmptyDescription,
        };
        assert!(error.retryable(), "an empty description may differ on retry");
        assert!(
            error.user_message().contains("Du kan försöka igen"),
            "a transient error offers a retry hint"
        );
    }

    /// Each retryable group keeps its verdict: transient Discord-API failures are
    /// retryable, misconfiguration and stale buttons are not, and the sub-error
    /// variants delegate to their own classification.
    #[test]
    fn retryable_classification_covers_each_group() {
        let send = SendMessageSnafu.into_error(serenity::Error::Io(io_error()));
        assert!(send.retryable(), "a Discord send failure is transient");

        let register = RegisterCommandSnafu.into_error(serenity::Error::Io(io_error()));
        assert!(
            !register.retryable(),
            "a command-registration failure is a permanent misconfiguration"
        );

        let unknown = InteractionKind::try_from("zzzz").err();
        assert!(unknown.is_some(), "an unknown tag yields the error we need");
        let Some(source) = unknown else { return };
        let stale = AppError::UnknownInteraction { source };
        assert!(!stale.retryable(), "a stale button is not retryable");

        assert!(
            AppError::Tts {
                source: TtsError::EmptyAudio
            }
            .retryable(),
            "an empty-audio TTS failure delegates to retryable"
        );
        assert!(
            !AppError::Tts {
                source: TtsError::MissingApiKey
            }
            .retryable(),
            "a missing API key delegates to permanent"
        );
    }

    /// A throwaway IO error to stand in as a `serenity::Error` source.
    fn io_error() -> IoError {
        IoError::other("test")
    }
}
