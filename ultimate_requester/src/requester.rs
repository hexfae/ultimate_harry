use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{CreateChatCompletionRequestArgs, CreateChatCompletionResponse},
};
use miette::Diagnostic;
use snafu::{ResultExt, Snafu};
use ultimate_config::ModelSettings;
use ultimate_history::History;

#[derive(Debug, Snafu, Diagnostic)]
pub enum Error {
    BuildRequest {
        source: async_openai::error::OpenAIError,
    },
    GetResponse {
        source: async_openai::error::OpenAIError,
    },
}

pub struct Requester {
    model_settings: ModelSettings,
}

impl Requester {
    pub fn new(model_settings: ModelSettings) -> Self {
        Self { model_settings }
    }

    pub async fn request(&self, history: History) -> Result<CreateChatCompletionResponse, Error> {
        let client = Client::with_config(
            OpenAIConfig::new()
                .with_api_base(self.model_settings.api_base())
                .with_api_key(self.model_settings.api_key()),
        );

        let request = CreateChatCompletionRequestArgs::default()
            .max_completion_tokens(2048_u32)
            .model(self.model_settings.model())
            .messages(history)
            .build()
            .context(BuildRequestSnafu)?;

        let response = client
            .chat()
            .create(request)
            .await
            .context(GetResponseSnafu)?;

        Ok(response)
    }
}
