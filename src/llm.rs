//! The LLM manager for generating responses from AI models.

mod completions;
mod settings;

use crate::models::message::{AttachmentMode, Message as ChatMessage};
use core::pin::Pin;
use miette::Diagnostic;
use rig::{
    agent::{AgentBuilder, MultiTurnStreamItem, StreamingError},
    http_client::Error as RigError,
    message::Message,
    providers::openrouter::{Client, CompletionModel, streaming::StreamingCompletionResponse},
    streaming::StreamingChat as _,
};
use serenity::futures::Stream;
use snafu::{ResultExt as _, Snafu};

pub use completions::{VoiceChoice, fetch_audio_base64};
pub use settings::{CharacterModelSettings, ModelOverrides, ModelSettings};

/// A streamed reply from the AI model, as returned by
/// [`LlmManager::request_stream`].
pub type ReplyStream = Pin<
    Box<dyn Stream<Item = Result<MultiTurnStreamItem<StreamingCompletionResponse>, StreamingError>> + Send>,
>;

/// The LLM manager for generating responses from AI models.
///
/// This struct manages the settings for the AI model and provides methods
/// for generating responses from the model.
#[derive(Debug)]
pub struct LlmManager {
    /// The settings used for the AI model.
    settings: ModelSettings,
}

impl LlmManager {
    /// Creates a new LLM manager with the given settings.
    #[must_use]
    pub const fn new(settings: ModelSettings) -> Self {
        Self { settings }
    }

    /// Returns a stream of responses from the AI model.
    ///
    /// This method is used for generating responses that are streamed back to the user,
    /// providing a more interactive experience.
    ///
    /// `context` is the already-assembled message list (scaffolding followed by the conversation),
    /// built by [`History::build_context`](crate::models::history::History::build_context).
    pub async fn request_stream(
        &self,
        context: &[ChatMessage],
        prompt: Option<String>,
        mode: AttachmentMode,
    ) -> Result<ReplyStream, LlmError> {
        let client = Client::new(&self.settings.api_key).context(BuildClientSnafu)?;
        let model = CompletionModel::new(client, &self.settings.model);

        let mut rig_messages: Vec<Message> = Vec::new();
        for msg in context {
            rig_messages.extend(msg.to_rig_messages(mode));
        }

        let agent = AgentBuilder::new(model)
            .temperature(self.settings.temperature.into())
            .build();

        Ok(agent
            .stream_chat(
                Message::system(prompt.unwrap_or_else(|| "Fortsätt rollspelet.".to_owned())),
                rig_messages,
            )
            .await)
    }
}

/// All errors that can happen when using the AI client.
#[derive(Debug, Snafu, Diagnostic)]
pub enum LlmError {
    /// Failed to build the Rig client.
    #[snafu(display("Kunde inte bygga LLM-klienten"))]
    #[diagnostic(
        help("Kontrollera att API-nyckeln är korrekt och att du har tillgång till modellen."),
        code(llm::build_client)
    )]
    BuildClient {
        /// The source of the error.
        source: RigError,
    },
    /// Failed to list the available models from `OpenRouter`.
    #[snafu(display("Kunde inte hämta modeller från OpenRouter"))]
    #[diagnostic(
        help("Kontrollera din internetanslutning och OpenRouter-status."),
        code(llm::list_models)
    )]
    ListModels {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// `OpenRouter` answered a chat-completions request with an error status.
    #[snafu(display("OpenRouter svarade med felkod {status}: {message}"))]
    #[diagnostic(
        help("Kontrollera att API-nyckeln och modellen är giltiga och att saldot räcker."),
        code(llm::http)
    )]
    Http {
        /// The HTTP status code returned.
        status: u16,
        /// The error message from the response body.
        message: String,
    },
    /// Failed to describe an image with the vision model.
    #[snafu(display("Kunde inte beskriva bilden"))]
    #[diagnostic(
        help("Kontrollera att synmodellen och API-nyckeln är giltiga."),
        code(llm::describe_image)
    )]
    DescribeImage {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// No vision model is configured to describe images.
    #[snafu(display("Ingen synmodell är inställd"))]
    #[diagnostic(help("Ställ in en synmodell med /modell."), code(llm::no_vision_model))]
    NoVisionModel,
    /// The vision model returned an empty description.
    #[snafu(display("Synmodellen gav ingen beskrivning"))]
    #[diagnostic(help("Prova en annan synmodell."), code(llm::empty_description))]
    EmptyDescription,
    /// Failed to download the voice message before transcribing it.
    #[snafu(display("Kunde inte hämta ljudet"))]
    #[diagnostic(help("Kontrollera att filen finns kvar."), code(llm::fetch_audio))]
    FetchAudio {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// Failed to transcribe a voice message with the audio model.
    #[snafu(display("Kunde inte transkribera ljudet"))]
    #[diagnostic(
        help("Kontrollera att ljudmodellen och API-nyckeln är giltiga."),
        code(llm::transcribe_audio)
    )]
    TranscribeAudio {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// No audio model is configured to transcribe voice messages.
    #[snafu(display("Ingen ljudmodell är inställd"))]
    #[diagnostic(help("Ställ in en ljudmodell med /modell."), code(llm::no_audio_model))]
    NoAudioModel,
    /// The audio model returned an empty transcription.
    #[snafu(display("Ljudmodellen gav ingen transkription"))]
    #[diagnostic(help("Prova en annan ljudmodell."), code(llm::empty_transcription))]
    EmptyTranscription,
    /// Failed to enrich the reply with audio tags.
    #[snafu(display("Kunde inte lägga till ljudtaggar"))]
    #[diagnostic(
        help("Kontrollera att tagg-modellen och API-nyckeln är giltiga."),
        code(llm::add_tags)
    )]
    AddTags {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// The tag model returned no enriched text.
    #[snafu(display("Tagg-modellen gav ingen text"))]
    #[diagnostic(help("Prova en annan tagg-modell."), code(llm::empty_tags))]
    EmptyTags,
    /// Failed to assign voices for a multi-voice reading.
    #[snafu(display("Kunde inte fördela rösterna"))]
    #[diagnostic(
        help("Kontrollera att tagg-modellen och API-nyckeln är giltiga."),
        code(llm::assign_voices)
    )]
    AssignVoices {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// The model returned no usable voice assignment.
    #[snafu(display("Röstfördelningen gick inte att tolka"))]
    #[diagnostic(help("Prova en annan tagg-modell."), code(llm::empty_voices))]
    EmptyVoices,
}

impl LlmError {
    /// Whether retrying might succeed (a transient network or empty-response
    /// failure) rather than a permanent misconfiguration (a bad client, no
    /// vision model configured, or an API error like a bad key or no credits).
    #[must_use]
    pub const fn retryable(&self) -> bool {
        match self {
            Self::ListModels { .. }
            | Self::DescribeImage { .. }
            | Self::EmptyDescription { .. }
            | Self::FetchAudio { .. }
            | Self::TranscribeAudio { .. }
            | Self::EmptyTranscription { .. }
            | Self::AddTags { .. }
            | Self::EmptyTags { .. }
            | Self::AssignVoices { .. }
            | Self::EmptyVoices { .. } => true,
            Self::Http { status, .. } => *status >= 500 || matches!(status, 408 | 429),
            Self::BuildClient { .. } | Self::NoVisionModel | Self::NoAudioModel => false,
        }
    }
}

/// Tests for the error classification.
#[cfg(test)]
mod tests {
    use super::LlmError;

    /// Missing-model misconfigurations are permanent, while empty-response
    /// failures are transient and may differ on retry.
    #[test]
    fn retryable_classification_splits_misconfiguration_from_transient() {
        assert!(
            !LlmError::NoVisionModel.retryable(),
            "no configured vision model is a permanent misconfiguration"
        );
        assert!(
            !LlmError::NoAudioModel.retryable(),
            "no configured audio model is a permanent misconfiguration"
        );
        assert!(
            LlmError::EmptyDescription.retryable(),
            "an empty description may differ on retry"
        );
        assert!(
            LlmError::EmptyTranscription.retryable(),
            "an empty transcription may differ on retry"
        );
        assert!(
            LlmError::EmptyTags.retryable(),
            "empty tag output may differ on retry"
        );
        assert!(
            LlmError::EmptyVoices.retryable(),
            "an unusable voice assignment may differ on retry"
        );
    }

    /// Server-side and rate-limit statuses are transient, while client errors
    /// like a bad API key or missing credits are permanent.
    #[test]
    fn retryable_classification_splits_http_statuses() {
        for status in [500, 502, 408, 429] {
            assert!(
                LlmError::Http {
                    status,
                    message: String::new()
                }
                .retryable(),
                "status {status} may succeed on retry"
            );
        }
        for status in [400, 401, 402, 403, 404] {
            assert!(
                !LlmError::Http {
                    status,
                    message: String::new()
                }
                .retryable(),
                "status {status} is a permanent misconfiguration"
            );
        }
    }
}
