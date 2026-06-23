//! The text-to-speech manager for speaking character replies aloud via `ElevenLabs`.

use crate::models::character::Character;
use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt as _, Snafu};

/// The `ElevenLabs` text-to-speech endpoint, to which the voice ID is appended.
const TTS_URL_BASE: &str = "https://api.elevenlabs.io/v1/text-to-speech/";

/// The default `ElevenLabs` model, chosen for its Swedish support.
const DEFAULT_TTS_MODEL: &str = "eleven_multilingual_v2";

/// The default `ElevenLabs` model used to synthesize speech.
fn default_tts_model() -> String {
    DEFAULT_TTS_MODEL.to_owned()
}

/// The text-to-speech manager for synthesizing spoken replies via `ElevenLabs`.
#[derive(Debug)]
pub struct TtsManager {
    /// The settings used for synthesis.
    settings: TtsSettings,
}

/// The settings for the `ElevenLabs` text-to-speech service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TtsSettings {
    /// The API key for the `ElevenLabs` service.
    pub api_key: String,
    /// The generic voice ID used when a character has no voice of its own.
    ///
    /// `None` means a character without its own voice cannot be spoken.
    pub default_voice: Option<String>,
    /// The `ElevenLabs` model used to synthesize speech.
    #[serde(default = "default_tts_model")]
    pub model: String,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            default_voice: None,
            model: default_tts_model(),
        }
    }
}

impl TtsSettings {
    /// Returns a Swedish summary of these settings, reporting only whether an API key is set.
    #[must_use]
    pub fn summary(&self) -> String {
        let api_key = if self.api_key.is_empty() {
            "inte inställd"
        } else {
            "inställd"
        };
        let default_voice = self.default_voice.as_deref().unwrap_or("ingen");
        format!(
            "röst-modell: {}\nstandardröst: {default_voice}\napi-nyckel: {api_key}",
            self.model
        )
    }

    /// Overrides any field for which a new value is supplied, leaving the rest untouched.
    pub fn apply_overrides(&mut self, overrides: TtsOverrides) {
        if let Some(new_api_key) = overrides.api_key {
            self.api_key = new_api_key;
        }
        if let Some(new_default_voice) = overrides.default_voice {
            self.default_voice = Some(new_default_voice);
        }
        if let Some(new_model) = overrides.model {
            self.model = new_model;
        }
    }

    /// Resolves the voice to speak `character` in: the character's own linked voice
    /// if it has one, otherwise the configured generic default. `None` when neither
    /// is set, so the caller can report that there is nothing to speak with.
    #[must_use]
    pub fn voice_for(&self, character: &Character) -> Option<String> {
        character
            .voice()
            .map(str::to_owned)
            .or_else(|| self.default_voice.clone())
    }
}

/// The optional text-to-speech setting overrides supplied by the `/tal` command.
///
/// Each present field replaces the corresponding setting; an all-`None` bundle
/// means "show the current settings" rather than change them.
#[derive(Debug, Default)]
pub struct TtsOverrides {
    /// The API key to use, if overridden.
    pub api_key: Option<String>,
    /// The generic default voice ID, if overridden.
    pub default_voice: Option<String>,
    /// The `ElevenLabs` model, if overridden.
    pub model: Option<String>,
}

impl TtsOverrides {
    /// Returns `true` when no override was supplied.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.api_key.is_none() && self.default_voice.is_none() && self.model.is_none()
    }
}

impl TtsManager {
    /// Creates a new text-to-speech manager with the given settings.
    #[must_use]
    pub const fn new(settings: TtsSettings) -> Self {
        Self { settings }
    }

    /// Resolves the voice to speak `character` in (see [`TtsSettings::voice_for`]).
    #[must_use]
    pub fn voice_for(&self, character: &Character) -> Option<String> {
        self.settings.voice_for(character)
    }

    /// Synthesizes `text` into MP3 audio bytes using the `ElevenLabs` `voice_id`.
    ///
    /// Fails early with [`TtsError::MissingApiKey`] when no API key is configured,
    /// rather than making a request that `ElevenLabs` would reject.
    pub async fn synthesize(&self, text: &str, voice_id: &str) -> Result<Vec<u8>, TtsError> {
        if self.settings.api_key.is_empty() {
            return MissingApiKeySnafu.fail();
        }
        let response = reqwest::Client::new()
            .post(tts_url(voice_id))
            .header("xi-api-key", &self.settings.api_key)
            .json(&tts_request_body(text, &self.settings.model))
            .send()
            .await
            .context(RequestSnafu)?;
        let status = response.status();
        if !status.is_success() {
            return HttpSnafu {
                status: status.as_u16(),
            }
            .fail();
        }
        let bytes = response.bytes().await.context(RequestSnafu)?.to_vec();
        if bytes.is_empty() {
            return EmptyAudioSnafu.fail();
        }
        Ok(bytes)
    }
}

/// The full `ElevenLabs` text-to-speech URL for the given voice.
fn tts_url(voice_id: &str) -> String {
    format!("{TTS_URL_BASE}{voice_id}")
}

/// Builds the JSON request body for an `ElevenLabs` synthesis request.
fn tts_request_body(text: &str, model: &str) -> serde_json::Value {
    serde_json::json!({
        "text": text,
        "model_id": model,
    })
}

/// All errors that can happen when synthesizing speech.
#[derive(Debug, Snafu, Diagnostic)]
#[expect(
    variant_size_differences,
    reason = "the reqwest::Error source is unavoidably larger than the unit and status-code variants"
)]
pub enum TtsError {
    /// No API key is configured for the text-to-speech service.
    #[snafu(display("Ingen API-nyckel för uppläsning är inställd"))]
    #[diagnostic(help("Ställ in en API-nyckel med /tal."), code(tts::missing_api_key))]
    MissingApiKey,
    /// The character has no voice and no generic default voice is configured.
    #[snafu(display("Ingen röst är kopplad till karaktären"))]
    #[diagnostic(
        help("Koppla en röst med /gubbe röst, eller ställ in en standardröst med /tal."),
        code(tts::no_voice)
    )]
    NoVoice,
    /// The request to `ElevenLabs` failed to send or read its response.
    #[snafu(display("Kunde inte läsa upp svaret: {source}"))]
    #[diagnostic(
        help("Kontrollera din internetanslutning och ElevenLabs-status."),
        code(tts::request)
    )]
    Request {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// `ElevenLabs` returned a non-success HTTP status.
    #[snafu(display("ElevenLabs svarade med felkod {status}"))]
    #[diagnostic(
        help("Kontrollera att API-nyckeln och röst-ID:t är giltiga."),
        code(tts::http)
    )]
    Http {
        /// The HTTP status code returned.
        status: u16,
    },
    /// `ElevenLabs` returned an empty audio body.
    #[snafu(display("ElevenLabs gav inget ljud"))]
    #[diagnostic(help("Prova igen eller med en annan röst."), code(tts::empty_audio))]
    EmptyAudio,
}

impl TtsError {
    /// Whether retrying might succeed (a transient network, server, or empty-response
    /// failure) rather than a permanent misconfiguration (no API key or no voice).
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Request { .. } | Self::Http { .. } | Self::EmptyAudio
        )
    }
}

/// Tests for voice resolution and request construction.
#[cfg(test)]
mod tests {
    use super::{TtsError, TtsOverrides, TtsSettings, tts_request_body, tts_url};
    use crate::models::character::Character;
    use serenity::all::UserId;

    /// Builds a character with the given optional linked voice.
    fn character(voice: Option<&str>) -> Character {
        let mut character = Character::builder()
            .id("id".to_owned())
            .name("Harry")
            .greeting("hi")
            .creator(UserId::new(1))
            .build();
        character.set_voice(voice.map(str::to_owned));
        character
    }

    /// Settings carrying the given generic default voice.
    fn settings(default_voice: Option<&str>) -> TtsSettings {
        TtsSettings {
            default_voice: default_voice.map(str::to_owned),
            ..TtsSettings::default()
        }
    }

    /// A character's own voice takes precedence over the generic default.
    #[test]
    fn character_voice_takes_precedence_over_the_default() {
        let resolved = settings(Some("default-voice")).voice_for(&character(Some("char-voice")));
        assert_eq!(
            resolved.as_deref(),
            Some("char-voice"),
            "the character's linked voice is used when present"
        );
    }

    /// A character without its own voice falls back to the generic default.
    #[test]
    fn falls_back_to_the_default_voice() {
        let resolved = settings(Some("default-voice")).voice_for(&character(None));
        assert_eq!(
            resolved.as_deref(),
            Some("default-voice"),
            "the generic default is used when the character has no voice"
        );
    }

    /// With neither a character voice nor a default, there is nothing to speak with.
    #[test]
    fn no_voice_when_neither_is_set() {
        let resolved = settings(None).voice_for(&character(None));
        assert!(
            resolved.is_none(),
            "no voice resolves when neither a character voice nor a default exists"
        );
    }

    /// The summary reports whether an API key is set without leaking it.
    #[test]
    fn summary_hides_the_api_key() {
        let mut configured = settings(Some("voice-1"));
        configured.api_key = "super-secret".to_owned();
        let summary = configured.summary();
        assert!(
            summary.contains("api-nyckel: inställd"),
            "a set API key is reported as set"
        );
        assert!(
            !summary.contains("super-secret"),
            "the API key value never appears in the summary"
        );
        assert!(
            summary.contains("standardröst: voice-1"),
            "the default voice is shown"
        );

        assert!(
            TtsSettings::default()
                .summary()
                .contains("api-nyckel: inte inställd"),
            "an unset API key is reported as unset"
        );
    }

    /// `apply_overrides` replaces only the supplied fields.
    #[test]
    fn apply_overrides_replaces_only_supplied_fields() {
        let mut configured = settings(Some("old-voice"));
        configured.api_key = "old-key".to_owned();
        configured.apply_overrides(TtsOverrides {
            default_voice: Some("new-voice".to_owned()),
            ..TtsOverrides::default()
        });
        assert_eq!(
            configured.default_voice.as_deref(),
            Some("new-voice"),
            "the supplied default voice is replaced"
        );
        assert_eq!(
            configured.api_key, "old-key",
            "the untouched API key is left as it was"
        );
    }

    /// An empty override bundle is detected, a non-empty one is not.
    #[test]
    fn overrides_emptiness_is_detected() {
        assert!(
            TtsOverrides::default().is_empty(),
            "an all-None bundle is empty"
        );
        assert!(
            !TtsOverrides {
                model: Some("eleven_turbo_v2".to_owned()),
                ..TtsOverrides::default()
            }
            .is_empty(),
            "a bundle with any field set is not empty"
        );
    }

    /// The request URL ends with the voice ID, and the body carries the text and model.
    #[test]
    fn request_targets_the_voice_with_the_text() {
        assert_eq!(
            tts_url("abc123"),
            "https://api.elevenlabs.io/v1/text-to-speech/abc123",
            "the voice ID is appended to the endpoint"
        );
        let body = tts_request_body("hej världen", "eleven_multilingual_v2");
        assert_eq!(
            body.get("text").and_then(serde_json::Value::as_str),
            Some("hej världen"),
            "the text to speak is in the body"
        );
        assert_eq!(
            body.get("model_id").and_then(serde_json::Value::as_str),
            Some("eleven_multilingual_v2"),
            "the model is sent as model_id"
        );
    }

    /// Misconfiguration is permanent, while transient failures are retryable.
    #[test]
    fn retryable_classification() {
        assert!(
            !TtsError::MissingApiKey.retryable(),
            "a missing API key is permanent"
        );
        assert!(!TtsError::NoVoice.retryable(), "a missing voice is permanent");
        assert!(
            TtsError::Http { status: 500 }.retryable(),
            "a server error may differ on retry"
        );
        assert!(
            TtsError::EmptyAudio.retryable(),
            "empty audio may differ on retry"
        );
    }
}
