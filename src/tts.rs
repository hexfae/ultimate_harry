//! The text-to-speech manager for speaking character replies aloud via `ElevenLabs`.

use crate::models::character::Character;
use jiff::Zoned;
use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt as _, Snafu};

/// The `ElevenLabs` text-to-speech endpoint, to which the voice ID is appended.
const TTS_URL_BASE: &str = "https://api.elevenlabs.io/v1/text-to-speech/";

/// The `ElevenLabs` text-to-dialogue endpoint, which speaks multiple voices in one request.
const DIALOGUE_URL: &str = "https://api.elevenlabs.io/v1/text-to-dialogue";

/// The default `ElevenLabs` model: Eleven v3, the expressive model that interprets audio tags.
const DEFAULT_TTS_MODEL: &str = "eleven_v3";

/// The default `ElevenLabs` model used to synthesize speech.
fn default_tts_model() -> String {
    DEFAULT_TTS_MODEL.to_owned()
}

/// The default `OpenRouter` model used to enrich a reply with audio tags before synthesis.
const DEFAULT_TAG_MODEL: &str = "google/gemini-3.1-flash-lite";

/// The default audio-tag enhancement model.
#[expect(
    clippy::unnecessary_wraps,
    reason = "serde default must produce the Option<String> field type"
)]
fn default_tag_model() -> Option<String> {
    Some(DEFAULT_TAG_MODEL.to_owned())
}

/// One turn of a multi-voice dialogue: a span of text spoken in a given voice.
///
/// Produced by the auto voice-assignment enricher (`LlmManager::assign_voices`)
/// and fed to [`TtsManager::synthesize_dialogue`] as one entry of the
/// `ElevenLabs` text-to-dialogue `inputs` array.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DialogueTurn {
    /// The `ElevenLabs` voice ID this turn is spoken in.
    pub voice_id: String,
    /// The text spoken in this turn.
    pub text: String,
}

/// One configurable voice in the speak-aloud palette, offered in the reply's
/// voice dropdown and described to the auto-assignment enricher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceEntry {
    /// The display name shown in the dropdown and used to match it for removal.
    pub name: String,
    /// The `ElevenLabs` voice ID spoken with.
    pub voice_id: String,
    /// The emoji shown beside the name in the dropdown (unicode or `<:name:id>`).
    pub emoji: String,
    /// The short description the auto enricher matches a speaker against.
    pub description: String,
    /// The `ElevenLabs` model spoken with for solo playback of this voice (the
    /// speak button or a directly chosen dropdown voice), overriding the
    /// configured default model. `None` (or empty) uses the configured model.
    /// Auto/dialogue ignores this and always uses the configured model (Eleven v3).
    #[serde(default)]
    pub model: Option<String>,
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
    /// The `OpenRouter` model that inserts audio tags into a reply before synthesis.
    ///
    /// Enhancement runs only when this is set and the synthesis model is audio-tag
    /// aware (Eleven v3); `None` (or an empty string) disables it. The enhancement
    /// uses the `OpenRouter` API key from the model settings, not the `ElevenLabs` key.
    #[serde(default = "default_tag_model")]
    pub tag_model: Option<String>,
    /// The configurable palette of voices offered in the reply's voice dropdown.
    ///
    /// Empty by default, in which case the dropdown is not shown and only the
    /// per-character / generic default voice (the speak button) is available.
    #[serde(default)]
    pub voices: Vec<VoiceEntry>,
}

impl Default for TtsSettings {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            default_voice: None,
            model: default_tts_model(),
            tag_model: default_tag_model(),
            voices: Vec::new(),
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
        let tag_model = self.tag_model_if_enabled().unwrap_or("av");
        format!(
            "röst-modell: {}\nstandardröst: {default_voice}\ntagg-modell: {tag_model}\nröster: {} st\napi-nyckel: {api_key}",
            self.model,
            self.voices.len(),
        )
    }

    /// The configurable voice palette offered in the reply's voice dropdown.
    #[must_use]
    pub fn voices(&self) -> &[VoiceEntry] {
        &self.voices
    }

    /// Adds a voice to the palette, replacing any existing entry with the same
    /// name (case-insensitive) so re-adding a name updates it rather than
    /// duplicating it.
    pub fn add_voice(&mut self, voice: VoiceEntry) {
        let name = voice.name.to_lowercase();
        self.voices
            .retain(|existing| existing.name.to_lowercase() != name);
        self.voices.push(voice);
    }

    /// Removes the palette voice with the given name (case-insensitive),
    /// returning whether one was removed.
    pub fn remove_voice(&mut self, name: &str) -> bool {
        let lowered = name.to_lowercase();
        let before = self.voices.len();
        self.voices
            .retain(|existing| existing.name.to_lowercase() != lowered);
        self.voices.len() != before
    }

    /// The synthesis model for solo playback of `voice_id` (the speak button or a
    /// directly chosen dropdown voice): the palette voice with that ID's own model
    /// override if it set a non-empty one, otherwise the configured default model.
    /// Auto/dialogue ignores this and always speaks with the configured model.
    #[must_use]
    pub fn solo_model(&self, voice_id: &str) -> &str {
        self.voices
            .iter()
            .find(|voice| voice.voice_id == voice_id)
            .and_then(|voice| voice.model.as_deref())
            .filter(|model| !model.is_empty())
            .unwrap_or(&self.model)
    }

    /// The audio-tag enhancement model to use when speaking with `model`, or `None`
    /// when enhancement does not apply: a non-empty tag model must be configured
    /// *and* `model` must be audio-tag aware (Eleven v3), since tags are meaningless
    /// on other models. Keyed off the effective synthesis model so a voice pinned to
    /// a non-v3 model skips the tags rather than speaking them literally.
    #[must_use]
    pub fn tag_model_for(&self, model: &str) -> Option<&str> {
        if !model_supports_audio_tags(model) {
            return None;
        }
        self.tag_model.as_deref().filter(|tag| !tag.is_empty())
    }

    /// The audio-tag enhancement model to use for the configured default model (see
    /// [`tag_model_for`](Self::tag_model_for)). Used by the auto/dialogue path, which
    /// always speaks with the configured model.
    #[must_use]
    pub fn tag_model_if_enabled(&self) -> Option<&str> {
        self.tag_model_for(&self.model)
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
        if let Some(new_tag_model) = overrides.tag_model {
            self.tag_model = Some(new_tag_model);
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
    /// The audio-tag enhancement model, if overridden.
    pub tag_model: Option<String>,
}

impl TtsOverrides {
    /// Returns `true` when no override was supplied.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.api_key.is_none()
            && self.default_voice.is_none()
            && self.model.is_none()
            && self.tag_model.is_none()
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

    /// Synthesizes `text` into MP3 audio bytes using the `ElevenLabs` `voice_id`,
    /// spoken with `model` (resolved by the caller via [`TtsSettings::solo_model`]
    /// for solo playback, or the configured model for the auto fallback).
    ///
    /// Fails early with [`TtsError::MissingApiKey`] when no API key is configured,
    /// rather than making a request that `ElevenLabs` would reject.
    pub async fn synthesize(
        &self,
        text: &str,
        voice_id: &str,
        model: &str,
    ) -> Result<Vec<u8>, TtsError> {
        self.post_audio(tts_url(voice_id), tts_request_body(text, model))
            .await
    }

    /// Synthesizes a multi-voice dialogue into MP3 audio bytes via the
    /// `ElevenLabs` text-to-dialogue endpoint, speaking each [`DialogueTurn`] in
    /// its own voice. Fails early with [`TtsError::MissingApiKey`] when no API
    /// key is configured, like [`synthesize`](Self::synthesize).
    pub async fn synthesize_dialogue(&self, turns: &[DialogueTurn]) -> Result<Vec<u8>, TtsError> {
        self.post_audio(
            DIALOGUE_URL.to_owned(),
            dialogue_request_body(turns, &self.settings.model),
        )
        .await
    }

    /// POSTs `body` to `url` on `ElevenLabs` and returns the MP3 bytes, failing
    /// early without an API key and validating the response status and that the
    /// audio is non-empty. Shared by the single-voice and dialogue paths.
    async fn post_audio(&self, url: String, body: serde_json::Value) -> Result<Vec<u8>, TtsError> {
        if self.settings.api_key.is_empty() {
            return MissingApiKeySnafu.fail();
        }
        let response = reqwest::Client::new()
            .post(url)
            .header("xi-api-key", &self.settings.api_key)
            .json(&body)
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

/// Whether `model` interprets audio tags (the Eleven v3 family).
fn model_supports_audio_tags(model: &str) -> bool {
    model.contains("v3")
}

/// Whether `text` has anything worth speaking, so the button can skip synthesizing
/// an empty reply (such as the "skip the greeting" choice), which `ElevenLabs` rejects.
pub fn is_speakable(text: &str) -> bool {
    !text.trim().is_empty()
}

/// The fallback filename stem when a character's name has nothing filename-safe.
const FILENAME_FALLBACK: &str = "uppläsning";

/// Builds a filename-safe audio attachment name from the character name and the time
/// the reading was requested, e.g. `Harry_2026-06-23_1430.mp3`. `when` is expected to
/// already be in the desired timezone.
pub fn audio_filename(character_name: &str, when: &Zoned) -> String {
    format!(
        "{}_{}.mp3",
        sanitize_filename(character_name),
        when.strftime("%Y-%m-%d_%H%M")
    )
}

/// Reduces `name` to a filename-safe stem: alphanumerics are kept, every other run
/// collapses to a single underscore, and a name with nothing usable falls back to a
/// default. Swedish letters are alphanumeric, so they survive unchanged.
fn sanitize_filename(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars() {
        if character.is_alphanumeric() {
            out.push(character);
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        FILENAME_FALLBACK.to_owned()
    } else {
        trimmed.to_owned()
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

/// Builds the JSON request body for an `ElevenLabs` text-to-dialogue request:
/// the model plus an ordered `inputs` array of `{text, voice_id}` turns.
fn dialogue_request_body(turns: &[DialogueTurn], model: &str) -> serde_json::Value {
    let inputs: Vec<serde_json::Value> = turns
        .iter()
        .map(|turn| {
            serde_json::json!({
                "text": turn.text,
                "voice_id": turn.voice_id,
            })
        })
        .collect();
    serde_json::json!({
        "model_id": model,
        "inputs": inputs,
    })
}

/// Replaces any turn whose `voice_id` is not in `allowed` with `fallback`, so a
/// model that invents a voice ID still speaks in a real, configured voice.
#[must_use]
pub fn enforce_allowed_voices(
    turns: Vec<DialogueTurn>,
    allowed: &[String],
    fallback: &str,
) -> Vec<DialogueTurn> {
    turns
        .into_iter()
        .map(|mut turn| {
            if !allowed.contains(&turn.voice_id) {
                fallback.clone_into(&mut turn.voice_id);
            }
            turn
        })
        .collect()
}

/// A plan for synthesizing assigned dialogue turns.
pub enum DialoguePlan {
    /// The turns use at most one distinct voice, so they are spoken as a single
    /// single-voice request over the joined text.
    Single {
        /// The joined text of every turn.
        text: String,
        /// The single voice the joined text is spoken in.
        voice_id: String,
    },
    /// The turns use more than one distinct voice, spoken via text-to-dialogue.
    Multi(Vec<DialogueTurn>),
}

/// Collapses `turns` to a [`DialoguePlan::Single`] when they use at most one
/// distinct voice (joining their texts, and using `fallback` when there are no
/// turns), otherwise keeps them as a [`DialoguePlan::Multi`].
#[must_use]
pub fn plan_dialogue(turns: Vec<DialogueTurn>, fallback: &str) -> DialoguePlan {
    let distinct = {
        let mut ids: Vec<&str> = turns.iter().map(|turn| turn.voice_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    };
    if distinct <= 1 {
        let text = turns
            .iter()
            .map(|turn| turn.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let voice_id = turns
            .first()
            .map_or_else(|| fallback.to_owned(), |turn| turn.voice_id.clone());
        DialoguePlan::Single { text, voice_id }
    } else {
        DialoguePlan::Multi(turns)
    }
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
    use super::{
        DialoguePlan, DialogueTurn, TtsError, TtsOverrides, TtsSettings, VoiceEntry, audio_filename,
        dialogue_request_body, enforce_allowed_voices, is_speakable, plan_dialogue,
        tts_request_body, tts_url,
    };

    /// Builds a dialogue turn with the given voice ID and text.
    fn turn(voice_id: &str, text: &str) -> DialogueTurn {
        DialogueTurn {
            voice_id: voice_id.to_owned(),
            text: text.to_owned(),
        }
    }
    use crate::models::character::Character;
    use serenity::all::UserId;

    /// Builds a palette voice entry with the given name and voice ID and no model override.
    fn voice_entry(name: &str, voice_id: &str) -> VoiceEntry {
        VoiceEntry {
            name: name.to_owned(),
            voice_id: voice_id.to_owned(),
            emoji: "🎙️".to_owned(),
            description: "a test voice".to_owned(),
            model: None,
        }
    }

    /// Builds a palette voice entry carrying the given optional solo model override.
    fn voice_entry_with_model(name: &str, voice_id: &str, model: Option<&str>) -> VoiceEntry {
        VoiceEntry {
            model: model.map(str::to_owned),
            ..voice_entry(name, voice_id)
        }
    }

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

    /// Builds settings with the given synthesis model and tag model.
    fn tagging(model: &str, tag_model: Option<&str>) -> TtsSettings {
        TtsSettings {
            model: model.to_owned(),
            tag_model: tag_model.map(str::to_owned),
            ..TtsSettings::default()
        }
    }

    /// `tag_model_if_enabled` keys the enhancement off the configured model,
    /// delegating to `tag_model_for` (whose full case matrix is covered by
    /// `tag_model_for_keys_off_the_effective_model`).
    #[test]
    fn tag_model_if_enabled_uses_the_configured_model() {
        let settings = tagging("eleven_multilingual_v2", Some("vendor/cheap"));
        assert!(
            settings.tag_model_if_enabled().is_none(),
            "a v2 configured model does not enhance"
        );
        assert_eq!(
            settings.tag_model_for("eleven_v3"),
            Some("vendor/cheap"),
            "the same settings enhance a v3 effective model, so it is the configured model that decides"
        );
    }

    /// A palette voice with its own model override speaks solo in that model; one
    /// without falls back to the configured default model.
    #[test]
    fn solo_model_prefers_the_voice_override() {
        let mut configured = tagging("eleven_v3", None);
        configured.add_voice(voice_entry_with_model(
            "Adam",
            "adam-id",
            Some("eleven_multilingual_v2"),
        ));
        configured.add_voice(voice_entry_with_model("Eva", "eva-id", None));
        assert_eq!(
            configured.solo_model("adam-id"),
            "eleven_multilingual_v2",
            "a voice with an override speaks solo in its own model"
        );
        assert_eq!(
            configured.solo_model("eva-id"),
            "eleven_v3",
            "a voice without an override falls back to the configured model"
        );
    }

    /// An unknown voice ID or an empty override falls back to the configured model.
    #[test]
    fn solo_model_falls_back_for_unknown_or_empty_overrides() {
        let mut configured = tagging("eleven_v3", None);
        configured.add_voice(voice_entry_with_model("Adam", "adam-id", Some("")));
        assert_eq!(
            configured.solo_model("missing-id"),
            "eleven_v3",
            "an unknown voice ID falls back to the configured model"
        );
        assert_eq!(
            configured.solo_model("adam-id"),
            "eleven_v3",
            "an empty override is ignored and falls back to the configured model"
        );
    }

    /// Tag enhancement is decided against the effective model: a v2 model skips the
    /// v3 audio tags even when a tag model is configured.
    #[test]
    fn tag_model_for_keys_off_the_effective_model() {
        let settings = tagging("eleven_v3", Some("vendor/cheap"));
        assert_eq!(
            settings.tag_model_for("eleven_v3"),
            Some("vendor/cheap"),
            "a v3 effective model with a tag model enhances"
        );
        assert!(
            settings.tag_model_for("eleven_multilingual_v2").is_none(),
            "a v2 effective model skips tags even with a tag model configured"
        );
        assert!(
            tagging("eleven_v3", None)
                .tag_model_for("eleven_v3")
                .is_none(),
            "no tag model means no enhancement"
        );
        assert!(
            tagging("eleven_v3", Some(""))
                .tag_model_for("eleven_v3")
                .is_none(),
            "an empty tag model disables enhancement"
        );
    }

    /// A v2-pinned palette voice resolves to its model and skips tags, while a
    /// non-pinned voice keeps the configured v3 model and its tags.
    #[test]
    fn pinned_voice_uses_v2_and_skips_tags() {
        let mut configured = tagging("eleven_v3", Some("vendor/cheap"));
        configured.add_voice(voice_entry_with_model(
            "Adam",
            "adam-id",
            Some("eleven_multilingual_v2"),
        ));
        configured.add_voice(voice_entry_with_model("Eva", "eva-id", None));

        let adam_model = configured.solo_model("adam-id");
        assert_eq!(adam_model, "eleven_multilingual_v2", "Adam is pinned to v2");
        assert!(
            configured.tag_model_for(adam_model).is_none(),
            "Adam's v2 playback skips the v3 audio tags"
        );

        let eva_model = configured.solo_model("eva-id");
        assert_eq!(eva_model, "eleven_v3", "Eva keeps the configured v3 model");
        assert_eq!(
            configured.tag_model_for(eva_model),
            Some("vendor/cheap"),
            "Eva's v3 playback still enhances with tags"
        );
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

    /// The dialogue body carries the model and an ordered inputs array of
    /// `{text, voice_id}` turns.
    #[test]
    fn dialogue_body_lists_turns_with_voices() {
        let turns = vec![
            DialogueTurn {
                voice_id: "voice-a".to_owned(),
                text: "hej".to_owned(),
            },
            DialogueTurn {
                voice_id: "voice-b".to_owned(),
                text: "svar".to_owned(),
            },
        ];
        let body = dialogue_request_body(&turns, "eleven_v3");
        assert_eq!(
            body.get("model_id").and_then(serde_json::Value::as_str),
            Some("eleven_v3"),
            "the model is sent as model_id"
        );
        let inputs_value = body.get("inputs").and_then(serde_json::Value::as_array);
        assert_eq!(inputs_value.map(Vec::len), Some(2), "both turns are listed");
        let Some(inputs) = inputs_value else { return };
        assert_eq!(
            inputs.first().and_then(|input| input.get("voice_id"))
                .and_then(serde_json::Value::as_str),
            Some("voice-a"),
            "the first turn keeps its voice"
        );
        assert_eq!(
            inputs.first().and_then(|input| input.get("text"))
                .and_then(serde_json::Value::as_str),
            Some("hej"),
            "the first turn keeps its text"
        );
    }

    /// Re-adding a name updates the entry in place; removal reports whether it hit.
    #[test]
    fn palette_add_replaces_and_remove_reports() {
        let mut configured = TtsSettings::default();
        configured.add_voice(voice_entry("Berättare", "old-id"));
        configured.add_voice(voice_entry("berättare", "new-id"));
        assert_eq!(
            configured.voices().len(),
            1,
            "re-adding the same name (case-insensitive) replaces rather than duplicates"
        );
        assert_eq!(
            configured.voices().first().map(|voice| voice.voice_id.as_str()),
            Some("new-id"),
            "the replacement keeps the latest voice ID"
        );
        assert!(
            configured.remove_voice("BERÄTTARE"),
            "removing an existing name reports a hit"
        );
        assert!(configured.voices().is_empty(), "the palette is now empty");
        assert!(
            !configured.remove_voice("Berättare"),
            "removing a missing name reports no hit"
        );
    }

    /// The filename combines the sanitized character name with the request time,
    /// collapsing unsafe characters and falling back when nothing usable remains.
    #[test]
    fn audio_filename_combines_name_and_request_time() {
        use jiff::civil::date;
        let zoned = date(2026, 6, 23).at(14, 30, 0, 0).in_tz("Europe/Stockholm");
        assert!(
            zoned.is_ok(),
            "the test timezone must resolve (jiff tzdb available)"
        );
        let Ok(when) = zoned else {
            return;
        };
        assert_eq!(
            audio_filename("Harry", &when),
            "Harry_2026-06-23_1430.mp3",
            "a plain name pairs with the formatted request time"
        );
        assert_eq!(
            audio_filename("Captain O'Hara!", &when),
            "Captain_O_Hara_2026-06-23_1430.mp3",
            "spaces and punctuation collapse to single underscores"
        );
        assert_eq!(
            audio_filename("Åsa Ö", &when),
            "Åsa_Ö_2026-06-23_1430.mp3",
            "Swedish letters survive as alphanumeric"
        );
        assert_eq!(
            audio_filename("🙂", &when),
            "uppläsning_2026-06-23_1430.mp3",
            "a name with nothing filename-safe falls back to a default"
        );
    }

    /// Empty or whitespace-only replies are not speakable, so the button skips the call.
    #[test]
    fn blank_text_is_not_speakable() {
        assert!(is_speakable("hej"), "real text is speakable");
        assert!(!is_speakable(""), "empty text is not speakable");
        assert!(
            !is_speakable("   \n\t  "),
            "whitespace-only text is not speakable"
        );
    }

    /// An unknown voice ID is rewritten to the fallback, while a known one and the
    /// turn's text are left untouched.
    #[test]
    fn enforce_allowed_voices_replaces_unknown_ids() {
        let turns = vec![turn("a", "hej"), turn("x", "svar")];
        let allowed = vec!["a".to_owned(), "b".to_owned()];
        let fixed = enforce_allowed_voices(turns, &allowed, "b");
        assert_eq!(
            fixed.iter().map(|turn| turn.voice_id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"],
            "a known id is kept and an unknown one becomes the fallback"
        );
        assert_eq!(
            fixed.get(1).map(|turn| turn.text.as_str()),
            Some("svar"),
            "the rewritten turn keeps its text"
        );
    }

    /// Turns using a single distinct voice collapse to a single-voice plan over the
    /// joined text; an empty turn list falls back to the fallback voice.
    #[test]
    fn plan_dialogue_collapses_to_a_single_voice() {
        let single = plan_dialogue(vec![turn("a", "hej"), turn("a", "då")], "fallback");
        assert!(
            matches!(&single, DialoguePlan::Single { voice_id, text } if voice_id == "a" && text == "hej\ndå"),
            "one distinct voice joins into a single-voice plan"
        );

        let empty = plan_dialogue(vec![], "fallback");
        assert!(
            matches!(&empty, DialoguePlan::Single { voice_id, text } if voice_id == "fallback" && text.is_empty()),
            "no turns fall back to the fallback voice"
        );
    }

    /// Turns using more than one distinct voice stay a multi-voice plan.
    #[test]
    fn plan_dialogue_keeps_multiple_voices() {
        let multi = plan_dialogue(vec![turn("a", "hej"), turn("b", "svar")], "fallback");
        assert!(
            matches!(&multi, DialoguePlan::Multi(turns) if turns.len() == 2),
            "two distinct voices stay a multi-voice plan"
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
