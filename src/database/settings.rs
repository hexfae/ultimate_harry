//! The bot-global singleton config files: AI model settings, text-to-speech
//! settings, and the pin channel.

use serenity::all::ChannelId;
use snafu::ResultExt as _;

use super::store::read_or;
use super::{
    Database, DatabaseError, SetModelSettingsSnafu, SetPinsChannelSnafu, SetTtsSettingsSnafu,
};
use crate::llm::ModelSettings;
use crate::models::character::Character;
use crate::tts::{TtsSettings, VoiceEntry};

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the global config operations are split into this child module to separate them from the per-record operations"
)]
impl Database {
    /// Updates or inserts the bot's AI model settings.
    pub async fn upsert_model_settings(
        &self,
        model_settings: ModelSettings,
    ) -> Result<(), DatabaseError> {
        self.write_json(&self.model_settings_path(), &model_settings)
            .await
            .context(SetModelSettingsSnafu)
    }

    /// Returns the configured voice palette for the reply's voice dropdown.
    ///
    /// Empty when no voices are configured, in which case the dropdown is hidden.
    /// Like [`character_menu_options`](Self::character_menu_options), it is read
    /// once per reply and threaded into the renderer, rather than re-read on every
    /// streaming tick.
    pub async fn voice_options(&self) -> Vec<VoiceEntry> {
        self.tts_settings().await.voices
    }

    /// Returns the bot's AI model settings.
    pub async fn model_settings(&self) -> ModelSettings {
        read_or(
            &self.model_settings_path(),
            "model settings",
            ModelSettings::default,
        )
        .await
    }

    /// Returns the global model settings with the character's per-character
    /// overrides (model, provider, and temperature) applied on top, if it has any.
    ///
    /// A global provider pin is only inherited when the character runs the global
    /// model, since a pin names one endpoint of one model and would fail every
    /// request against any other.
    pub async fn resolved_model_settings(&self, character: &Character) -> ModelSettings {
        let mut settings = self.model_settings().await;
        if let Some(overrides) = character.model_settings() {
            let global_model = settings.model.clone();
            if let Some(model) = overrides.model.clone() {
                let runs_global_model = model == global_model;
                settings.model = model;
                if !runs_global_model {
                    settings.provider = None;
                }
            }
            if let Some(provider) = overrides.provider.clone() {
                settings.provider = Some(provider);
            }
            if let Some(temperature) = overrides.temperature {
                settings.temperature = temperature;
            }
        }
        settings
    }

    /// Returns the bot's text-to-speech settings.
    pub async fn tts_settings(&self) -> TtsSettings {
        read_or(
            &self.tts_settings_path(),
            "tts settings",
            TtsSettings::default,
        )
        .await
    }

    /// Updates or inserts the bot's text-to-speech settings.
    pub async fn upsert_tts_settings(
        &self,
        tts_settings: TtsSettings,
    ) -> Result<(), DatabaseError> {
        self.write_json(&self.tts_settings_path(), &tts_settings)
            .await
            .context(SetTtsSettingsSnafu)
    }

    /// Returns the bot's pin channel.
    pub async fn pins_channel(&self) -> ChannelId {
        read_or(&self.pin_channel_path(), "pin channel", ChannelId::default).await
    }

    /// Updates or inserts the bot's pin channel.
    pub async fn upsert_pin_channel(
        &self,
        channel_id: ChannelId,
    ) -> Result<ChannelId, DatabaseError> {
        self.write_json(&self.pin_channel_path(), &channel_id)
            .await
            .context(SetPinsChannelSnafu)?;
        Ok(channel_id)
    }
}
