use crate::{LlmGenerationSnafu, config::ModelSettings, models::history::History};
use rig::{
    agent::AgentBuilder,
    completion::Chat,
    message::Message,
    providers::openrouter::{Client, CompletionModel},
};
use snafu::ResultExt;

pub struct LlmManager {
    settings: ModelSettings,
}

impl LlmManager {
    pub fn new(settings: ModelSettings) -> Self {
        Self { settings }
    }

    pub async fn request(&self, history: &History) -> Result<String, crate::Error> {
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
            .chat(Message::system("Fortsätt rollspelet."), rig_messages)
            .await
            .context(LlmGenerationSnafu)?;

        Ok(response)
    }
}
