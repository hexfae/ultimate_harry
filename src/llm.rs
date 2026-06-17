//! The LLM manager for generating responses from AI models.

use crate::models::message::Message as ChatMessage;
use core::pin::Pin;
use miette::Diagnostic;
use rig::{
    agent::{AgentBuilder, MultiTurnStreamItem, StreamingError},
    completion::PromptError,
    http_client::Error as RigError,
    message::Message,
    providers::openrouter::{Client, CompletionModel, streaming::StreamingCompletionResponse},
    streaming::StreamingChat as _,
};
use serde::{Deserialize, Serialize};
use serenity::futures::Stream;
use snafu::{ResultExt as _, Snafu};

/// The LLM manager for generating responses from AI models.
///
/// This struct manages the settings for the AI model and provides methods
/// for generating responses from the model.
#[derive(Debug)]
pub struct LlmManager {
    /// The settings used for the AI model.
    settings: ModelSettings,
}

/// The settings for the AI model.
///
/// These settings control various aspects of how the AI model generates responses,
/// such as temperature, penalties for repeated tokens, and the model to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
    /// The model to use for generating responses.
    ///
    /// This should be a valid model identifier for the `OpenRouter` API.
    pub model: String,
    /// The API key for the `OpenRouter` service.
    pub api_key: String,
    /// The temperature to use for generating responses.
    ///
    /// Higher values make the output more random, while lower values make it
    /// more deterministic and focused.
    pub temperature: f32,
}

impl ModelSettings {
    /// Returns a Swedish summary of these settings, reporting only whether an API key is set.
    #[must_use]
    pub fn summary(&self) -> String {
        let api_key = if self.api_key.is_empty() {
            "inte inställd"
        } else {
            "inställd"
        };
        format!(
            "modell: {}\ntemperatur: {}\napi-nyckel: {api_key}",
            self.model, self.temperature
        )
    }

    /// Overrides any field for which a new value is supplied, leaving the rest untouched.
    pub fn apply_overrides(
        &mut self,
        model: Option<String>,
        api_key: Option<String>,
        temperature: Option<f32>,
    ) {
        if let Some(new_model) = model {
            self.model = new_model;
        }
        if let Some(new_api_key) = api_key {
            self.api_key = new_api_key;
        }
        if let Some(new_temperature) = temperature {
            self.temperature = new_temperature;
        }
    }
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
    /// built by [`Database::build_context`](crate::database::Database::build_context).
    pub async fn request_stream(
        &self,
        context: &[ChatMessage],
        prompt: Option<String>,
    ) -> Result<
        Pin<
            Box<
                dyn Stream<
                        Item = Result<
                            MultiTurnStreamItem<StreamingCompletionResponse>,
                            StreamingError,
                        >,
                    > + Send,
            >,
        >,
        LlmError,
    > {
        let client = Client::new(&self.settings.api_key).context(BuildClientSnafu)?;
        let model = CompletionModel::new(client, &self.settings.model);

        let mut rig_messages: Vec<Message> = Vec::new();
        for msg in context {
            rig_messages.extend(msg.to_rig_messages());
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

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            model: "deepseek/deepseek-v3.2".to_owned(),
            api_key: String::new(),
            temperature: 1.0,
        }
    }
}

/// All errors that can happen when using the AI client.
#[derive(Debug, Snafu, Diagnostic)]
pub enum LlmError {
    /// Failed to build the Rig client.
    #[snafu(display("Misslyckades med att bygga LLM-klienten: {source}"))]
    #[diagnostic(
        help("Kontrollera att API-nyckeln är korrekt och att du har tillgång till modellen."),
        code(llm::build_client)
    )]
    BuildClient {
        /// The source of the error.
        source: RigError,
    },
    /// Failed to get a response from the AI.
    #[snafu(display("Misslyckades med att få svar från AI: {source}"))]
    #[diagnostic(
        help(
            "Prompten kan vara för långt eller innehålla ogiltiga tecken. Försök med ett kortare meddelande."
        ),
        code(llm::get_response)
    )]
    GetResponse {
        /// The source of the error.
        source: PromptError,
    },
}
