use crate::{LlmGenerationSnafu, ModelSettings, models::history::History};
use rig::{
    agent::AgentBuilder,
    completion::Chat,
    message::Message,
    providers::openrouter::{Client, CompletionModel},
};
use snafu::ResultExt;
use unicode_segmentation::UnicodeSegmentation;

pub struct LlmManager {
    settings: ModelSettings,
}

impl LlmManager {
    pub fn new(settings: ModelSettings) -> Self {
        Self { settings }
    }

    /// Returns the response of the given character of the given history.
    ///
    /// The answer is trimmed to 3900 characters, in order to easily fit within Discord's
    /// 4000-character limit for components (which includes other text like the footer or
    /// the character's name).
    pub async fn request(
        &self,
        history: &History,
        prompt: Option<String>,
    ) -> Result<String, crate::Error> {
        let client = Client::new(&self.settings.api_key).unwrap();
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
            .context(LlmGenerationSnafu)?;

        Ok(response.graphemes(true).take(3900).chain([" "]).collect())
    }
}
