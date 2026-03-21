use miette::Diagnostic;
use serde::{Deserialize, Serialize};
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
}

impl Config {
    pub fn load(path: &str) -> Result<Self, ConfigError> {
        let contents = read_to_string(path).context(ReadSnafu)?;
        toml::from_str(&contents).context(ParseSnafu)
    }
}
