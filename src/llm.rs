use std::pin::Pin;

use crate::{constants::CHARACTER_LIMIT, models::history::History};
use miette::Diagnostic;
use rig::{
    agent::{AgentBuilder, MultiTurnStreamItem, StreamingError},
    completion::Chat,
    message::Message,
    providers::openrouter::{Client, CompletionModel, streaming::StreamingCompletionResponse},
    streaming::StreamingChat,
};
use serde::{Deserialize, Serialize};
use serenity::futures::Stream;
use snafu::{ResultExt, Snafu};
use unicode_segmentation::UnicodeSegmentation;

pub struct LlmManager {
    settings: ModelSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
    pub model: String,
    pub api_key: String,
    pub frequency_penalty: f32,
    pub presence_penalty: f32,
    pub temperature: f32,
    pub top_p: f32,
}

#[derive(Debug, Snafu, Diagnostic)]
pub enum LlmError {
    BuildClient {
        source: rig::http_client::Error,
    },
    GetResponse {
        source: rig::completion::PromptError,
    },
}

impl LlmManager {
    pub fn new(settings: ModelSettings) -> Self {
        Self { settings }
    }

    /// Returns the response of the given character of the given history.
    pub async fn request(
        &self,
        history: &History,
        prompt: Option<String>,
    ) -> Result<String, LlmError> {
        let client = Client::new(&self.settings.api_key).context(BuildClientSnafu)?;
        let model = CompletionModel::new(client, &self.settings.model);

        let mut rig_messages: Vec<Message> = Vec::new();
        for msg in history.previous_messages() {
            rig_messages.extend(msg.to_rig_messages());
        }

        let agent = AgentBuilder::new(model)
            .temperature(self.settings.temperature.into())
            .build();

        let response = agent
            .chat(
                Message::system(prompt.unwrap_or_else(|| "Fortsätt rollspelet.".to_owned())),
                rig_messages,
            )
            .await
            .context(GetResponseSnafu)?;

        Ok(response
            .graphemes(true)
            .take(CHARACTER_LIMIT)
            .chain([" "])
            .collect())
    }

    pub async fn request_stream(
        &self,
        history: &History,
        prompt: Option<String>,
    ) -> Pin<
        Box<
            dyn Stream<
                    Item = Result<MultiTurnStreamItem<StreamingCompletionResponse>, StreamingError>,
                > + Send,
        >,
    > {
        // TODO: make this not panic
        let client = Client::new(&self.settings.api_key).expect("openrouter api key");
        let model = CompletionModel::new(client, &self.settings.model);

        let mut rig_messages: Vec<Message> = Vec::new();
        for msg in history.previous_messages() {
            rig_messages.extend(msg.to_rig_messages());
        }

        let agent = AgentBuilder::new(model)
            .temperature(self.settings.temperature.into())
            .build();

        agent
            .stream_chat(
                Message::system(prompt.unwrap_or_else(|| "Fortsätt rollspelet.".to_owned())),
                rig_messages,
            )
            .await
    }
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
