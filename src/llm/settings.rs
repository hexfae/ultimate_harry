//! The serializable model-configuration types: the global model settings, the
//! per-character overrides applied on top, and the `/modell` command's override bundle.
//!
//! Split out from `llm.rs`; these plain data types are imported independently of the
//! streaming and REST layers by the database, the character model, and the model command.

use serde::{Deserialize, Serialize};

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
    /// The model used to describe image attachments for models that lack vision.
    ///
    /// When the active model cannot read images, this model is asked to caption them and the text
    /// is sent instead. `None` disables the fallback.
    #[serde(default = "default_vision_model")]
    pub vision_model: Option<String>,
    /// The model used to transcribe voice messages for models that lack audio input.
    ///
    /// When the active model cannot read audio, this model is asked to transcribe each voice
    /// message and the text is sent instead. `None` disables the fallback.
    #[serde(default = "default_audio_model")]
    pub audio_model: Option<String>,
}

/// The default vision model used to describe images for models without vision.
#[expect(
    clippy::unnecessary_wraps,
    reason = "serde default must produce the Option<String> field type"
)]
fn default_vision_model() -> Option<String> {
    Some("google/gemini-3.1-flash-lite".to_owned())
}

/// The default audio model used to transcribe voice messages for models without audio input.
#[expect(
    clippy::unnecessary_wraps,
    reason = "serde default must produce the Option<String> field type"
)]
fn default_audio_model() -> Option<String> {
    Some("google/gemini-3.1-flash-lite".to_owned())
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
        let vision_model = self.vision_model.as_deref().unwrap_or("ingen");
        let audio_model = self.audio_model.as_deref().unwrap_or("ingen");
        format!(
            "modell: {}\nsyn-modell: {vision_model}\nljud-modell: {audio_model}\ntemperatur: {}\napi-nyckel: {api_key}",
            self.model, self.temperature
        )
    }

    /// Overrides any field for which a new value is supplied, leaving the rest untouched.
    pub fn apply_overrides(&mut self, overrides: ModelOverrides) {
        if let Some(new_model) = overrides.model {
            self.model = new_model;
        }
        if let Some(new_api_key) = overrides.api_key {
            self.api_key = new_api_key;
        }
        if let Some(new_temperature) = overrides.temperature {
            self.temperature = new_temperature;
        }
        if let Some(new_vision_model) = overrides.vision_model {
            self.vision_model = Some(new_vision_model);
        }
        if let Some(new_audio_model) = overrides.audio_model {
            self.audio_model = Some(new_audio_model);
        }
    }
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            model: "deepseek/deepseek-v3.2".to_owned(),
            api_key: String::new(),
            temperature: 1.0,
            vision_model: default_vision_model(),
            audio_model: default_audio_model(),
        }
    }
}

/// A character's per-character overrides for the global model settings.
///
/// Only the model and temperature can be overridden per character; the API key and the
/// vision/audio fallback models always come from the global settings. Each present field
/// replaces the corresponding global setting at request time (see
/// [`Database::resolved_model_settings`](crate::database::Database::resolved_model_settings)).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CharacterModelSettings {
    /// The model to use for this character, if overridden.
    #[serde(default)]
    pub model: Option<String>,
    /// The sampling temperature for this character, if overridden.
    #[serde(default)]
    pub temperature: Option<f32>,
}

/// The optional model-setting overrides supplied by the `/modell` commands.
///
/// Each present field replaces the corresponding setting; an all-`None` bundle
/// means "show the current settings" rather than change them.
#[derive(Debug, Default)]
pub struct ModelOverrides {
    /// The model to use, if overridden.
    pub model: Option<String>,
    /// The API key to use, if overridden.
    pub api_key: Option<String>,
    /// The sampling temperature, if overridden.
    pub temperature: Option<f32>,
    /// The vision model for image descriptions, if overridden.
    pub vision_model: Option<String>,
    /// The audio model for voice-message transcriptions, if overridden.
    pub audio_model: Option<String>,
}

impl ModelOverrides {
    /// Returns `true` when no override was supplied.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.model.is_none()
            && self.api_key.is_none()
            && self.temperature.is_none()
            && self.vision_model.is_none()
            && self.audio_model.is_none()
    }
}

/// Tests for the model-settings overrides and summary.
#[cfg(test)]
mod tests {
    use super::{ModelOverrides, ModelSettings};

    /// `apply_overrides` replaces only the supplied fields, across each branch.
    #[test]
    fn model_apply_overrides_replaces_only_supplied_fields() {
        let mut settings = ModelSettings::default();
        settings.apply_overrides(ModelOverrides {
            model: Some("vendor/new".to_owned()),
            temperature: Some(0.3),
            vision_model: Some("vendor/vision".to_owned()),
            audio_model: Some("vendor/audio".to_owned()),
            ..ModelOverrides::default()
        });
        assert_eq!(
            settings.model, "vendor/new",
            "the supplied model is replaced"
        );
        assert_eq!(
            settings.temperature.to_bits(),
            0.3_f32.to_bits(),
            "the supplied temperature is replaced"
        );
        assert_eq!(
            settings.vision_model.as_deref(),
            Some("vendor/vision"),
            "the supplied vision model is replaced"
        );
        assert_eq!(
            settings.audio_model.as_deref(),
            Some("vendor/audio"),
            "the supplied audio model is replaced"
        );
        assert!(
            settings.api_key.is_empty(),
            "the untouched API key is left as it was"
        );
    }

    /// `ModelOverrides::is_empty` is true only for an all-None bundle.
    #[test]
    fn model_overrides_emptiness_is_detected() {
        assert!(
            ModelOverrides::default().is_empty(),
            "an all-None bundle is empty"
        );
        assert!(
            !ModelOverrides {
                model: Some("vendor/x".to_owned()),
                ..ModelOverrides::default()
            }
            .is_empty(),
            "a bundle with any field set is not empty"
        );
    }

    /// The summary reports the models and whether an API key is set, without leaking it.
    #[test]
    fn summary_reports_models_and_hides_the_api_key() {
        let settings = ModelSettings {
            api_key: "super-secret".to_owned(),
            ..ModelSettings::default()
        };
        let summary = settings.summary();
        assert!(
            summary.contains("modell: deepseek/deepseek-v3.2"),
            "the configured model is shown"
        );
        assert!(
            summary.contains("api-nyckel: inställd"),
            "a set API key is reported as set"
        );
        assert!(
            !summary.contains("super-secret"),
            "the API key value never appears in the summary"
        );
        assert!(
            ModelSettings::default()
                .summary()
                .contains("api-nyckel: inte inställd"),
            "an unset API key is reported as unset"
        );
    }
}
