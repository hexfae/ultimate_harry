//! The LLM manager for generating responses from AI models.

use crate::models::message::{
    AttachmentMode, EncodedAudio, Message as ChatMessage, audio_format_from_url,
};
use crate::tts::{DialogueTurn, VoiceEntry};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use core::pin::Pin;
use miette::Diagnostic;
use rig::message::AudioMediaType;
use rig::{
    agent::{AgentBuilder, MultiTurnStreamItem, StreamingError},
    http_client::Error as RigError,
    message::Message,
    providers::openrouter::{Client, CompletionModel, streaming::StreamingCompletionResponse},
    streaming::StreamingChat as _,
};
use serde::{Deserialize, Serialize};
use serenity::futures::Stream;
use snafu::{IntoError, OptionExt as _, ResultExt as _, Snafu};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// The `OpenRouter` endpoint listing every available model and its capabilities.
const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// The `OpenRouter` chat-completions endpoint, used to describe images.
const CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

/// The instruction given to the vision model when describing an image.
const DESCRIBE_PROMPT: &str =
    "Describe the image in as much detail as possible. Write the description in Swedish.";

/// The instruction given to the audio model when transcribing a voice message.
const TRANSCRIBE_PROMPT: &str = "Transcribe the spoken audio verbatim, keeping the transcription in the original spoken language (do not translate it). Briefly describe any non-speech sounds in square brackets. Reply with only the transcription, no explanation.";

/// The instruction given to the tag model when enriching a reply with audio tags.
const TAG_PROMPT: &str = "You are given a single line of dialogue from a roleplay. Insert ElevenLabs v3 audio tags (square-bracketed, e.g. [laughs], [sighs], [whispers], [angry], [sad]) at fitting points so it sounds expressive when read aloud. Keep all of the original text and its language exactly as given: do not translate, rephrase, or change any words; only add tags. The tags themselves must always be in English, even when the dialogue is in another language. Reply with only the tagged text, no explanation.";

/// The instruction given to the model when splitting a reply into per-voice turns.
const SEGMENT_PROMPT: &str = "You are given a roleplay reply and a list of available voices. Split the reply into an ordered sequence of speaker turns and assign each turn one of the available voices by its id, choosing the voice whose description best matches that speaker. Cover the entire reply in order, keeping every word and its original language exactly as given: do not translate, rephrase, drop, or reorder any text. Use only voice ids from the provided list. Reply with ONLY a JSON array of objects with the keys \"voice_id\" and \"text\", and nothing else (no prose, no code fences).";

/// The extra instruction folded into [`SEGMENT_PROMPT`] when the synthesis model
/// is audio-tag aware, so each turn's text is also enriched with v3 tags.
const SEGMENT_TAG_CLAUSE: &str = " Additionally, insert ElevenLabs v3 audio tags (square-bracketed and always in English, e.g. [laughs], [sighs], [whispers]) at fitting points within each turn's text so it sounds expressive when read aloud.";

/// The LLM manager for generating responses from AI models.
///
/// This struct manages the settings for the AI model and provides methods
/// for generating responses from the model.
#[derive(Debug)]
pub struct LlmManager {
    /// The settings used for the AI model.
    settings: ModelSettings,
}

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
            "modell: {}\nsynmodell: {vision_model}\nljudmodell: {audio_model}\ntemperatur: {}\napi-nyckel: {api_key}",
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

impl LlmManager {
    /// Creates a new LLM manager with the given settings.
    #[must_use]
    pub const fn new(settings: ModelSettings) -> Self {
        Self { settings }
    }

    /// Returns a stream of responses from the AI model.
    ///
    /// This method is used for generating responses that are streamed back to the user,
    /// providing a more interactive experience.
    ///
    /// `context` is the already-assembled message list (scaffolding followed by the conversation),
    /// built by [`History::build_context`](crate::models::history::History::build_context).
    pub async fn request_stream(
        &self,
        context: &[ChatMessage],
        prompt: Option<String>,
        mode: AttachmentMode,
    ) -> Result<
        Pin<
            Box<
                dyn Stream<
                        Item = Result<
                            MultiTurnStreamItem<StreamingCompletionResponse>,
                            StreamingError,
                        >,
                    > + Send,
            >,
        >,
        LlmError,
    > {
        let client = Client::new(&self.settings.api_key).context(BuildClientSnafu)?;
        let model = CompletionModel::new(client, &self.settings.model);

        let mut rig_messages: Vec<Message> = Vec::new();
        for msg in context {
            rig_messages.extend(msg.to_rig_messages(mode));
        }

        let agent = AgentBuilder::new(model)
            .temperature(self.settings.temperature.into())
            .build();

        Ok(agent
            .stream_chat(
                Message::system(prompt.unwrap_or_else(|| "Fortsätt rollspelet.".to_owned())),
                rig_messages,
            )
            .await)
    }

    /// Returns whether the active model can read images, by checking its `OpenRouter` capabilities.
    ///
    /// The per-model answer is cached process-wide so the full model list is fetched at most once
    /// per model rather than on every request.
    pub async fn supports_vision(&self) -> Result<bool, LlmError> {
        self.supports_modality("image", modality_cache()).await
    }

    /// Returns whether the active model can read audio, by checking its `OpenRouter` capabilities.
    ///
    /// The per-model answer is cached process-wide, like [`supports_vision`](Self::supports_vision).
    pub async fn supports_audio(&self) -> Result<bool, LlmError> {
        self.supports_modality("audio", modality_cache()).await
    }

    /// Returns whether the active model lists `modality` among its accepted input modalities,
    /// caching the per-model answer in `cache` so the model list is fetched at most once per
    /// (model, modality) pair.
    async fn supports_modality(
        &self,
        modality: &str,
        cache: &'static Mutex<HashMap<(String, String), bool>>,
    ) -> Result<bool, LlmError> {
        let key = (self.settings.model.clone(), modality.to_owned());
        if let Some(cached) = cache
            .lock()
            .ok()
            .and_then(|locked| locked.get(&key).copied())
        {
            return Ok(cached);
        }
        let response = reqwest::get(MODELS_URL).await.context(ListModelsSnafu)?;
        let models = response
            .json::<ModelsResponse>()
            .await
            .context(ListModelsSnafu)?;
        let supported = model_supports_modality(&models, &self.settings.model, modality);
        if let Ok(mut locked) = cache.lock() {
            locked.insert(key, supported);
        }
        Ok(supported)
    }

    /// POSTs `body` to the `OpenRouter` chat-completions endpoint and parses the
    /// reply, attaching `request_error` as the snafu context for both the request
    /// and the JSON decode (they share a failure class per caller).
    async fn chat_completion<E>(
        &self,
        body: serde_json::Value,
        request_error: E,
    ) -> Result<ChatResponse, LlmError>
    where
        E: IntoError<LlmError, Source = reqwest::Error> + Copy,
    {
        let response = reqwest::Client::new()
            .post(CHAT_URL)
            .bearer_auth(&self.settings.api_key)
            .json(&body)
            .send()
            .await
            .context(request_error)?;
        response
            .json::<ChatResponse>()
            .await
            .context(request_error)
    }

    /// Describes the image at `url` using the configured vision model, returning its caption.
    pub async fn describe_image(&self, url: &str) -> Result<String, LlmError> {
        let vision_model = self
            .settings
            .vision_model
            .as_deref()
            .context(NoVisionModelSnafu)?;
        let body = serde_json::json!({
            "model": vision_model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": DESCRIBE_PROMPT},
                    {"type": "image_url", "image_url": {"url": url}},
                ],
            }],
        });
        let parsed = self.chat_completion(body, DescribeImageSnafu).await?;
        extract_description(&parsed).context(EmptyDescriptionSnafu)
    }

    /// Transcribes the voice message at `url` using the configured audio model, returning its text.
    ///
    /// The clip is downloaded and base64-encoded first, since `OpenRouter` accepts audio only as
    /// base64 `input_audio`, never as a URL.
    pub async fn transcribe_audio(&self, url: &str) -> Result<String, LlmError> {
        let audio_model = self
            .settings
            .audio_model
            .as_deref()
            .context(NoAudioModelSnafu)?;
        let encoded = fetch_audio_base64(url).await?;
        let body = serde_json::json!({
            "model": audio_model,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": TRANSCRIBE_PROMPT},
                    {"type": "input_audio", "input_audio": {"data": encoded.data, "format": encoded.format}},
                ],
            }],
        });
        let parsed = self.chat_completion(body, TranscribeAudioSnafu).await?;
        extract_description(&parsed).context(EmptyTranscriptionSnafu)
    }

    /// Rewrites `text` with inline `ElevenLabs` v3 audio tags using `model`, for more
    /// expressive text-to-speech. Only the spoken text is enriched; the visible reply
    /// is left untouched by the caller.
    pub async fn add_audio_tags(&self, text: &str, model: &str) -> Result<String, LlmError> {
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": TAG_PROMPT},
                {"role": "user", "content": text},
            ],
        });
        let parsed = self.chat_completion(body, AddTagsSnafu).await?;
        extract_description(&parsed).context(EmptyTagsSnafu)
    }

    /// Splits `text` into ordered per-voice turns using `model`, assigning each
    /// turn one of the supplied `choices` by description, for multi-voice
    /// text-to-dialogue synthesis. When `add_tags` is set, each turn's text is
    /// also enriched with v3 audio tags in the same call.
    ///
    /// The returned `voice_id`s are whatever the model produced; the caller
    /// validates them against the allowed set before synthesizing.
    pub async fn assign_voices(
        &self,
        text: &str,
        model: &str,
        choices: &[VoiceChoice],
        add_tags: bool,
    ) -> Result<Vec<DialogueTurn>, LlmError> {
        let mut system = SEGMENT_PROMPT.to_owned();
        if add_tags {
            system.push_str(SEGMENT_TAG_CLAUSE);
        }
        let voices = choices
            .iter()
            .map(VoiceChoice::prompt_line)
            .collect::<Vec<_>>()
            .join("\n");
        let user = format!("Available voices:\n{voices}\n\nReply:\n{text}");
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
        });
        let parsed = self.chat_completion(body, AssignVoicesSnafu).await?;
        let content = extract_description(&parsed).context(EmptyVoicesSnafu)?;
        parse_dialogue_turns(&content).context(EmptyVoicesSnafu)
    }
}

/// One voice offered to the auto voice-assignment enricher: the id it should
/// emit, plus the name and description it matches a speaker against.
#[derive(Debug, Clone)]
pub struct VoiceChoice {
    /// The `ElevenLabs` voice ID the model should emit for a matching turn.
    pub voice_id: String,
    /// The voice's display name, shown to the model for context.
    pub name: String,
    /// The description the model matches a speaker against.
    pub description: String,
}

impl VoiceChoice {
    /// Builds a [`VoiceChoice`] from a palette [`VoiceEntry`].
    #[must_use]
    pub fn from_entry(entry: &VoiceEntry) -> Self {
        Self {
            voice_id: entry.voice_id.clone(),
            name: entry.name.clone(),
            description: entry.description.clone(),
        }
    }

    /// The single line describing this voice in the enricher's prompt.
    fn prompt_line(&self) -> String {
        format!(
            "- {} (id: {}): {}",
            self.name, self.voice_id, self.description
        )
    }
}

/// Parses the enricher's reply into dialogue turns, tolerating a Markdown code
/// fence around the JSON array. Returns `None` when no non-empty turn parses.
fn parse_dialogue_turns(content: &str) -> Option<Vec<DialogueTurn>> {
    let trimmed = content.trim();
    let body = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map_or(trimmed, |rest| rest.trim_start());
    let unfenced = body.strip_suffix("```").unwrap_or(body).trim();
    let parsed = serde_json::from_str::<Vec<DialogueTurn>>(unfenced).ok()?;
    let usable: Vec<DialogueTurn> = parsed
        .into_iter()
        .filter(|turn| !turn.text.trim().is_empty() && !turn.voice_id.trim().is_empty())
        .collect();
    if usable.is_empty() { None } else { Some(usable) }
}

/// Process-wide cache of (model id, modality) to whether the model accepts that input modality, so
/// the `OpenRouter` model list is fetched at most once per (model, modality) pair.
fn modality_cache() -> &'static Mutex<HashMap<(String, String), bool>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, String), bool>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Downloads the audio at `url` and base64-encodes it, detecting the format from the URL.
///
/// `OpenRouter` accepts audio only as base64 `input_audio`, so this is needed both to transcribe a
/// voice message and to send it natively to an audio-capable model.
pub async fn fetch_audio_base64(url: &str) -> Result<EncodedAudio, LlmError> {
    let response = reqwest::get(url).await.context(FetchAudioSnafu)?;
    let bytes = response.bytes().await.context(FetchAudioSnafu)?;
    let data = STANDARD.encode(&bytes);
    Ok(EncodedAudio {
        url: url.to_owned(),
        data,
        format: audio_format_from_url(url).unwrap_or(AudioMediaType::OGG),
    })
}

/// Whether the model with `model_id` lists `modality` among its accepted input modalities.
fn model_supports_modality(models: &ModelsResponse, model_id: &str, modality: &str) -> bool {
    models
        .data
        .iter()
        .find(|entry| entry.id == model_id)
        .is_some_and(|entry| {
            entry
                .architecture
                .input_modalities
                .iter()
                .any(|listed| listed == modality)
        })
}

/// Pulls the description text out of a chat-completions response, if any non-blank content exists.
fn extract_description(response: &ChatResponse) -> Option<String> {
    response
        .choices
        .first()
        .map(|choice| choice.message.content.trim().to_owned())
        .filter(|content| !content.is_empty())
}

/// The subset of `OpenRouter`'s model-list response we care about.
#[derive(Deserialize)]
struct ModelsResponse {
    /// The listed models.
    #[serde(default)]
    data: Vec<ModelEntry>,
}

/// One model in `OpenRouter`'s model list.
#[derive(Deserialize)]
struct ModelEntry {
    /// The model identifier, for example `deepseek/deepseek-v3.2`.
    id: String,
    /// The model's input/output modality metadata.
    #[serde(default)]
    architecture: Architecture,
}

/// A model's modality metadata.
#[derive(Default, Deserialize)]
struct Architecture {
    /// The input modalities the model accepts, for example `text` and `image`.
    #[serde(default)]
    input_modalities: Vec<String>,
}

/// The subset of a chat-completions response we care about.
#[derive(Deserialize)]
struct ChatResponse {
    /// The completion choices returned.
    #[serde(default)]
    choices: Vec<ChatChoice>,
}

/// One choice in a chat-completions response.
#[derive(Deserialize)]
struct ChatChoice {
    /// The message produced for this choice.
    message: ChatMessageContent,
}

/// The message content of a chat-completions choice.
#[derive(Deserialize)]
struct ChatMessageContent {
    /// The text content of the message.
    #[serde(default)]
    content: String,
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

/// All errors that can happen when using the AI client.
#[derive(Debug, Snafu, Diagnostic)]
pub enum LlmError {
    /// Failed to build the Rig client.
    #[snafu(display("Misslyckades med att bygga LLM-klienten: {source}"))]
    #[diagnostic(
        help("Kontrollera att API-nyckeln är korrekt och att du har tillgång till modellen."),
        code(llm::build_client)
    )]
    BuildClient {
        /// The source of the error.
        source: RigError,
    },
    /// Failed to list the available models from `OpenRouter`.
    #[snafu(display("Kunde inte hämta modeller från OpenRouter: {source}"))]
    #[diagnostic(
        help("Kontrollera din internetanslutning och OpenRouter-status."),
        code(llm::list_models)
    )]
    ListModels {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// Failed to describe an image with the vision model.
    #[snafu(display("Kunde inte beskriva bilden: {source}"))]
    #[diagnostic(
        help("Kontrollera att synmodellen och API-nyckeln är giltiga."),
        code(llm::describe_image)
    )]
    DescribeImage {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// No vision model is configured to describe images.
    #[snafu(display("Ingen synmodell är inställd"))]
    #[diagnostic(help("Ställ in en synmodell med /modell."), code(llm::no_vision_model))]
    NoVisionModel,
    /// The vision model returned an empty description.
    #[snafu(display("Synmodellen gav ingen beskrivning"))]
    #[diagnostic(help("Prova en annan synmodell."), code(llm::empty_description))]
    EmptyDescription,
    /// Failed to download the voice message before transcribing it.
    #[snafu(display("Kunde inte hämta ljudet: {source}"))]
    #[diagnostic(help("Kontrollera att filen finns kvar."), code(llm::fetch_audio))]
    FetchAudio {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// Failed to transcribe a voice message with the audio model.
    #[snafu(display("Kunde inte transkribera ljudet: {source}"))]
    #[diagnostic(
        help("Kontrollera att ljudmodellen och API-nyckeln är giltiga."),
        code(llm::transcribe_audio)
    )]
    TranscribeAudio {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// No audio model is configured to transcribe voice messages.
    #[snafu(display("Ingen ljudmodell är inställd"))]
    #[diagnostic(help("Ställ in en ljudmodell med /modell."), code(llm::no_audio_model))]
    NoAudioModel,
    /// The audio model returned an empty transcription.
    #[snafu(display("Ljudmodellen gav ingen transkription"))]
    #[diagnostic(help("Prova en annan ljudmodell."), code(llm::empty_transcription))]
    EmptyTranscription,
    /// Failed to enrich the reply with audio tags.
    #[snafu(display("Kunde inte lägga till ljudtaggar: {source}"))]
    #[diagnostic(
        help("Kontrollera att tagg-modellen och API-nyckeln är giltiga."),
        code(llm::add_tags)
    )]
    AddTags {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// The tag model returned no enriched text.
    #[snafu(display("Tagg-modellen gav ingen text"))]
    #[diagnostic(help("Prova en annan tagg-modell."), code(llm::empty_tags))]
    EmptyTags,
    /// Failed to assign voices for a multi-voice reading.
    #[snafu(display("Kunde inte fördela rösterna: {source}"))]
    #[diagnostic(
        help("Kontrollera att tagg-modellen och API-nyckeln är giltiga."),
        code(llm::assign_voices)
    )]
    AssignVoices {
        /// The source of the error.
        source: reqwest::Error,
    },
    /// The model returned no usable voice assignment.
    #[snafu(display("Röstfördelningen gick inte att tolka"))]
    #[diagnostic(help("Prova igen eller en annan tagg-modell."), code(llm::empty_voices))]
    EmptyVoices,
}

impl LlmError {
    /// Whether retrying might succeed (a transient network or empty-response
    /// failure) rather than a permanent misconfiguration (a bad client or no
    /// vision model configured).
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self,
            Self::ListModels { .. }
                | Self::DescribeImage { .. }
                | Self::EmptyDescription { .. }
                | Self::FetchAudio { .. }
                | Self::TranscribeAudio { .. }
                | Self::EmptyTranscription { .. }
                | Self::AddTags { .. }
                | Self::EmptyTags { .. }
                | Self::AssignVoices { .. }
                | Self::EmptyVoices { .. }
        )
    }
}

/// Tests for the `OpenRouter` response parsing helpers.
#[cfg(test)]
mod tests {
    use super::{
        ChatResponse, LlmError, ModelOverrides, ModelSettings, ModelsResponse, extract_description,
        model_supports_modality, parse_dialogue_turns,
    };

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
        assert_eq!(settings.model, "vendor/new", "the supplied model is replaced");
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

    /// Missing-model misconfigurations are permanent, while empty-response
    /// failures are transient and may differ on retry.
    #[test]
    fn retryable_classification_splits_misconfiguration_from_transient() {
        assert!(
            !LlmError::NoVisionModel.retryable(),
            "no configured vision model is a permanent misconfiguration"
        );
        assert!(
            !LlmError::NoAudioModel.retryable(),
            "no configured audio model is a permanent misconfiguration"
        );
        assert!(
            LlmError::EmptyDescription.retryable(),
            "an empty description may differ on retry"
        );
        assert!(
            LlmError::EmptyTranscription.retryable(),
            "an empty transcription may differ on retry"
        );
        assert!(
            LlmError::EmptyTags.retryable(),
            "empty tag output may differ on retry"
        );
        assert!(
            LlmError::EmptyVoices.retryable(),
            "an unusable voice assignment may differ on retry"
        );
    }

    /// A bare JSON array of turns parses, keeping order, voice, and text.
    #[test]
    fn parse_dialogue_turns_reads_a_bare_array() {
        let parsed = parse_dialogue_turns(
            r#"[{"voice_id":"a","text":"hej"},{"voice_id":"b","text":"svar"}]"#,
        );
        assert!(parsed.is_some(), "a well-formed array parses");
        let Some(turns) = parsed else { return };
        assert_eq!(turns.len(), 2, "both turns are kept");
        assert_eq!(
            turns.first().map(|turn| turn.voice_id.as_str()),
            Some("a"),
            "the first turn keeps its voice"
        );
    }

    /// A code-fenced array still parses, and empty/garbage yields nothing.
    #[test]
    fn parse_dialogue_turns_unfences_and_rejects_garbage() {
        let fenced = "```json\n[{\"voice_id\":\"a\",\"text\":\"hej\"}]\n```";
        assert!(
            parse_dialogue_turns(fenced).is_some(),
            "a fenced array is unwrapped and parsed"
        );
        let no_closing_fence = "```json\n[{\"voice_id\":\"a\",\"text\":\"hej\"}]";
        assert!(
            parse_dialogue_turns(no_closing_fence).is_some(),
            "an opening fence without a closing one is still unwrapped and parsed"
        );
        assert!(
            parse_dialogue_turns("not json at all").is_none(),
            "non-JSON yields no turns"
        );
        assert!(
            parse_dialogue_turns(r#"[{"voice_id":"","text":"  "}]"#).is_none(),
            "turns with blank voice or text are dropped, leaving nothing"
        );
    }

    /// A model supports a modality only when it lists it among its input modalities.
    #[test]
    fn model_supports_modality_checks_input_modalities() {
        let json = r#"{"data":[
            {"id":"vendor/multi","architecture":{"input_modalities":["text","image","audio"]}},
            {"id":"vendor/sees","architecture":{"input_modalities":["text","image"]}},
            {"id":"vendor/text","architecture":{"input_modalities":["text"]}}
        ]}"#;
        let parsed = serde_json::from_str::<ModelsResponse>(json);
        assert!(parsed.is_ok(), "the model list should parse");
        let Ok(models) = parsed else { return };

        assert!(
            model_supports_modality(&models, "vendor/sees", "image"),
            "a model listing image input supports vision"
        );
        assert!(
            !model_supports_modality(&models, "vendor/text", "image"),
            "a model without image input does not support vision"
        );
        assert!(
            model_supports_modality(&models, "vendor/multi", "audio"),
            "a model listing audio input supports audio"
        );
        assert!(
            !model_supports_modality(&models, "vendor/sees", "audio"),
            "a vision-only model does not support audio"
        );
        assert!(
            !model_supports_modality(&models, "vendor/missing", "audio"),
            "an unlisted model is treated as supporting no modality"
        );
    }

    /// The first choice's trimmed content is taken as the description.
    #[test]
    fn extract_description_reads_the_first_choice() {
        let parsed = serde_json::from_str::<ChatResponse>(
            r#"{"choices":[{"message":{"content":"  en hund  "}}]}"#,
        );
        assert!(parsed.is_ok(), "the chat response should parse");
        let Ok(response) = parsed else { return };
        assert_eq!(
            extract_description(&response).as_deref(),
            Some("en hund"),
            "the first choice's trimmed content is the description"
        );
    }

    /// No choices or blank content yields no description.
    #[test]
    fn extract_description_is_none_when_empty() {
        let parsed_empty = serde_json::from_str::<ChatResponse>(r#"{"choices":[]}"#);
        assert!(parsed_empty.is_ok(), "an empty chat response should parse");
        let Ok(empty) = parsed_empty else { return };
        assert!(
            extract_description(&empty).is_none(),
            "no choices yields no description"
        );

        let parsed_blank =
            serde_json::from_str::<ChatResponse>(r#"{"choices":[{"message":{"content":"   "}}]}"#);
        assert!(parsed_blank.is_ok(), "a blank chat response should parse");
        let Ok(blank) = parsed_blank else { return };
        assert!(
            extract_description(&blank).is_none(),
            "blank content yields no description"
        );
    }
}
