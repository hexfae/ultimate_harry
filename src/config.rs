use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use serenity::all::ChannelId;
use snafu::{ResultExt, Snafu};
use std::fs::read_to_string;

#[derive(Debug, Snafu, Diagnostic)]
pub enum ConfigError {
    #[snafu(display("Could not read config file: {source}"))]
    Read { source: std::io::Error },
    #[snafu(display("Could not parse config file: {source}"))]
    Parse { source: toml::de::Error },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub bot_token: String,
    pub model_settings: ModelSettings,
    pub pins_channel_id: ChannelId,
    pub name_substitutions: std::collections::HashMap<u64, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelSettings {
    pub model: String,
    pub api_key: String,
    pub api_base: String,
    pub frequency_penalty: f32,
    pub presence_penalty: f32,
    pub temperature: f32,
    pub top_p: f32,
}

impl Config {
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let contents = read_to_string(path).context(ReadSnafu)?;
        toml::from_str(&contents).context(ParseSnafu)
    }

    pub fn substitute_name(&self, user_id: impl Into<u64>) -> String {
        self.name_substitutions
            .get(&user_id.into())
            .cloned()
            .unwrap_or_else(|| "User".to_owned())
    }
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            model: "deepseek/deepseek-v3.2".to_owned(),
            api_key: String::new(),
            api_base: "https://openrouter.ai/api/v1".to_owned(),
            frequency_penalty: 0.0,
            presence_penalty: 0.0,
            temperature: 1.0,
            top_p: 0.95,
        }
    }
}
