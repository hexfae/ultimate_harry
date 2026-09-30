//! The hand-rolled `OpenRouter` REST layer: model-capability probing, image
//! description, audio transcription, audio-tag enrichment, and per-voice
//! assignment, plus their request/response DTOs.
//!
//! Split out from `llm.rs`; a child module so its `impl LlmManager` block can
//! reach the manager's private settings while staying separate from the rig
//! streaming core.

use crate::constants::MAX_TOKENS;
use crate::http::http;
use crate::models::message::{EncodedAudio, audio_format_from_url};
use crate::tts::{DialogueTurn, VoiceEntry};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rig::agent::StreamingError;
use rig::completion::CompletionError;
use rig::http_client::Error as RigError;
use rig::message::AudioMediaType;
use serde::Deserialize;
use snafu::{IntoError, OptionExt as _, ResultExt as _};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use super::{
    AddTagsSnafu, AssignVoicesSnafu, DescribeImageSnafu, EmptyDescriptionSnafu, EmptyTagsSnafu,
    EmptyTranscriptionSnafu, EmptyVoicesSnafu, FetchAudioHttpSnafu, FetchAudioSnafu, HttpSnafu,
    ListModelsSnafu, LlmError, LlmManager, NoAudioModelSnafu, NoVisionModelSnafu,
    TranscribeAudioSnafu, settings::ModelSettings,
};

/// The `OpenRouter` endpoint listing every available model and its capabilities.
const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// The `OpenRouter` chat-completions endpoint, used to describe images.
const CHAT_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

/// The instruction given to the vision model when describing an image.
const DESCRIBE_PROMPT: &str =
    "Describe the image in as much detail as possible. Write the description in Swedish.";

/// The instruction given to the audio model when transcribing a voice message.
const TRANSCRIBE_PROMPT: &str = "Transcribe the spoken audio verbatim, keeping the transcription in the original spoken language (do not translate it). Briefly describe any non-speech sounds in square brackets. Reply with only the transcription, no explanation.";

/// The opening instruction given to the tag model when enriching a reply with audio
/// tags. [`TAG_GUIDE`] and [`TAG_PROMPT_REPLY`] are appended to it before sending.
const TAG_PROMPT: &str = "You are given a single line of dialogue from a roleplay. Insert ElevenLabs audio tags (square-bracketed and always in English) throughout it so it sounds as expressive as possible when read aloud.";

/// The audio-tag guidance shared by the single-line enricher and the per-turn
/// dialogue enricher, so both push for the same maximal expressiveness.
const TAG_GUIDE: &str = "Be prolific: every line should carry tags, and every shift in emotion, intensity, volume, pace, or speaker should be marked with one, stacking several tags in a sentence when the performance layers (e.g. \"[tired] [softly] it has been a long day... [upset] how many more can I take?\"). A tag can sit anywhere in the text, and a line usually opens with one. The text you are given is usually bare, carrying little or no direction of its own, so supply the whole performance yourself rather than only annotating what is already written: assume every line has a mood, a setting, and sounds around it, and invent what the moment calls for. A tag the voice cannot quite pull off is a small blemish, while a line left untagged is a flat, lifeless read, so when in doubt, add it. Use the whole tag vocabulary and give the actor as much direction as the moment allows: emotions ([angry], [sad], [sorrowful], [excited], [happily], [sarcastically], [awe], [annoyed], [surprised], [booming], [big laugh]), delivery and pacing ([whispers], [shouts], [softly], [rushed], [slowly], [drawn out], [pause]), human reactions ([laughs], [sighs], [gasps], [coughing], [clears throat], [beginning to speak], [interrupting], [overlapping]), accents and character voices ([French accent], [British accent], [pirate voice]), and sound effects, which are a first-class part of this and not a garnish: a scene has a location and a time of day, so give it a soundscape and let it run ([gunshot], [explosion], [creaking door], [screech], [wind howling], [rain lashing the window], [distant traffic], [waves breaking], [crowd murmuring]), placing them where they belong in the action and keeping them running underneath the words where the setting warrants it. Shape the pacing in the text as well: an ellipsis or a comma for a short pause, a line break for a longer beat, capitals on a word to stress it. Pick tags the voice can actually embody, since one it cannot perform gets read aloud as words: push it hard, but stay inside its register rather than contradicting it. Keep all of the original text and its language exactly as given: do not translate, rephrase, drop, reorder, or change any words; only add tags, never action or narration outside them. When the text already narrates something happening, leave those words as they are and put a tag beside them, so a line reading \"he laughed loudly\" keeps its words and gains [chuckles] rather than being turned into one. Every tag must be in English, even when the dialogue is in another language.";

/// The closing format instruction for [`TAG_PROMPT`].
const TAG_PROMPT_REPLY: &str = "Reply with only the tagged text, no explanation.";

/// The instruction given to the model when splitting a reply into per-voice turns.
const SEGMENT_PROMPT: &str = "You are given a roleplay reply and a list of available voices. Split the reply into an ordered sequence of speaker turns and assign each turn one of the available voices by its id, choosing the voice whose description best matches that speaker. Cover the entire reply in order, keeping every word and its original language exactly as given: do not translate, rephrase, drop, or reorder any text. Use only voice ids from the provided list. Reply with ONLY a JSON array of objects with the keys \"voice_id\" and \"text\", and nothing else (no prose, no code fences).";

/// The extra instruction folded into [`SEGMENT_PROMPT`] when the synthesis model
/// is audio-tag aware, so each turn's text is also enriched with audio tags.
/// [`TAG_GUIDE`] is appended to it.
const SEGMENT_TAG_CLAUSE: &str = " Additionally, insert ElevenLabs audio tags (square-bracketed and always in English) throughout each turn's text so it sounds as expressive as possible when read aloud: tag generously, opening a turn with a tag when it captures the moment and tagging the mood shifts, interruptions, and overlaps between turns, and give the scene a soundscape with sound-effect tags where it calls for them.";

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the OpenRouter REST layer is split into this child module to separate it from the rig streaming core"
)]
impl LlmManager {
    /// Returns whether the active model can read images, by checking its `OpenRouter` capabilities.
    ///
    /// The answer comes from the process-wide model catalog, fetched at most once.
    pub async fn supports_vision(&self) -> Result<bool, LlmError> {
        self.supports_modality("image").await
    }

    /// Returns whether the active model can read audio, by checking its `OpenRouter` capabilities.
    ///
    /// The answer comes from the process-wide model catalog, like
    /// [`supports_vision`](Self::supports_vision).
    pub async fn supports_audio(&self) -> Result<bool, LlmError> {
        self.supports_modality("audio").await
    }

    /// Returns whether the active model lists `modality` among its accepted input modalities,
    /// checking the cached model catalog.
    async fn supports_modality(&self, modality: &str) -> Result<bool, LlmError> {
        let catalog = model_catalog().await?;
        Ok(model_supports_modality(
            &catalog,
            &self.settings.model,
            modality,
        ))
    }

    /// Returns whether the active model always reasons and rejects
    /// `effort: "none"`, checking the cached model catalog.
    ///
    /// An unlisted model is treated as able to turn reasoning off, matching
    /// the optimistic defaults of [`model_supports_modality`].
    pub(super) async fn reasoning_mandatory(&self) -> Result<bool, LlmError> {
        let catalog = model_catalog().await?;
        Ok(model_reasoning_mandatory(&catalog, &self.settings.model))
    }

    /// POSTs `body` to the `OpenRouter` chat-completions endpoint and parses the
    /// reply, attaching `request_error` as the snafu context for both the request
    /// and the JSON decode (they share a failure class per caller). An error
    /// status becomes [`LlmError::Http`] carrying the API's own error message,
    /// so a bad key or empty balance is not mistaken for an empty completion.
    /// Model reasoning is disabled on every request, since these are quick
    /// utility calls where thinking only adds latency; on models where
    /// reasoning cannot be turned off the parameter is left out instead.
    ///
    /// A provider pin rides along only when the body targets the pinned model:
    /// a slug names one endpoint of one model, so sending it with the vision,
    /// audio, or tag model would ask for a provider that does not serve it.
    async fn chat_completion<E>(
        &self,
        mut body: serde_json::Value,
        request_error: E,
    ) -> Result<ChatResponse, LlmError>
    where
        E: IntoError<LlmError, Source = reqwest::Error> + Copy,
    {
        if !self.reasoning_mandatory().await?
            && let Some(fields) = body.as_object_mut()
        {
            fields.insert(
                "reasoning".to_owned(),
                serde_json::json!({"enabled": false}),
            );
        }
        fields_insert_max_tokens(&mut body);
        fields_insert_provider(&mut body, &self.settings);
        let response = http()
            .post(CHAT_URL)
            .bearer_auth(&self.settings.api_key)
            .json(&body)
            .send()
            .await
            .context(request_error)?;
        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            // a rejected pin names the providers that would work, which is the
            // only way out, so it gets its own error instead of a bare status
            if let Some(error) = rejection_error(&error_body) {
                return Err(error);
            }
            return HttpSnafu {
                status: status.as_u16(),
                message: api_error_message(&error_body),
            }
            .fail();
        }
        response.json::<ChatResponse>().await.context(request_error)
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

    /// Rewrites `text` with inline `ElevenLabs` audio tags using `model`, for more
    /// expressive text-to-speech. Only the spoken text is enriched; the visible reply
    /// is left untouched by the caller.
    pub async fn add_audio_tags(&self, text: &str, model: &str) -> Result<String, LlmError> {
        let system = format!("{TAG_PROMPT} {TAG_GUIDE} {TAG_PROMPT_REPLY}");
        let body = serde_json::json!({
            "model": model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": text},
            ],
        });
        let parsed = self.chat_completion(body, AddTagsSnafu).await?;
        extract_description(&parsed).context(EmptyTagsSnafu)
    }

    /// Splits `text` into ordered per-voice turns using `model`, assigning each
    /// turn one of the supplied `choices` by description, for multi-voice
    /// text-to-dialogue synthesis. When `add_tags` is set, each turn's text is
    /// also enriched with audio tags in the same call.
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
            system.push(' ');
            system.push_str(TAG_GUIDE);
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
    if usable.is_empty() {
        None
    } else {
        Some(usable)
    }
}

/// Process-wide cache of the `OpenRouter` model catalog, so the model list is fetched at most
/// once per process. Empty until the first successful fetch.
static CATALOG_CACHE: LazyLock<Mutex<Vec<ModelEntry>>> = LazyLock::new(|| Mutex::new(Vec::new()));

/// The `OpenRouter` model catalog, fetched on the first call and served from the process-wide
/// cache afterwards. Backs both the modality probing and the model-id autocomplete.
pub async fn model_catalog() -> Result<Vec<ModelEntry>, LlmError> {
    if let Some(cached) = CATALOG_CACHE
        .lock()
        .ok()
        .filter(|locked| !locked.is_empty())
        .map(|locked| locked.clone())
    {
        return Ok(cached);
    }
    let response = http()
        .get(MODELS_URL)
        .send()
        .await
        .context(ListModelsSnafu)?;
    let models = response
        .json::<ModelsResponse>()
        .await
        .context(ListModelsSnafu)?;
    if let Ok(mut locked) = CATALOG_CACHE.lock() {
        locked.clone_from(&models.data);
    }
    Ok(models.data)
}

/// Process-wide cache of model id to the provider slugs serving it, so a
/// provider lookup costs at most one request per model per process. Endpoint
/// lists churn, but the bot is long-lived and a stale slug is caught by the
/// request failing, so caching beats re-fetching on every keystroke.
static ENDPOINTS_CACHE: LazyLock<Mutex<HashMap<String, Vec<EndpointEntry>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The provider slugs serving `model`, from the process-wide cache when present.
///
/// Backs the provider autocomplete and the check that drops a pin the new model
/// does not offer. An unknown model yields no endpoints rather than an error, so
/// a bad model id cannot wedge the `/modell` command.
pub async fn model_endpoints(model: &str) -> Vec<EndpointEntry> {
    if let Some(cached) = ENDPOINTS_CACHE
        .lock()
        .ok()
        .and_then(|locked| locked.get(model).cloned())
    {
        return cached;
    }
    let url = format!("{MODELS_URL}/{model}/endpoints");
    let Ok(response) = http().get(url).send().await else {
        return Vec::new();
    };
    if !response.status().is_success() {
        return Vec::new();
    }
    let Ok(parsed) = response.json::<EndpointsResponse>().await else {
        return Vec::new();
    };
    let endpoints = parsed.data.endpoints;
    if let Ok(mut locked) = ENDPOINTS_CACHE.lock() {
        locked.insert(model.to_owned(), endpoints.clone());
    }
    endpoints
}

/// Drops a provider pin that `model` does not serve, so switching models never
/// leaves a pin behind that would fail every request.
///
/// A pin is kept when the model still offers it, and cleared when it does not.
/// A lookup that yields nothing (network failure, unknown model) also clears,
/// since a pin that cannot be confirmed is not worth keeping.
#[must_use]
pub fn pin_for_model<'a>(pinned: Option<&'a str>, endpoints: &[EndpointEntry]) -> Option<&'a str> {
    let slug = pinned?;
    endpoints
        .iter()
        .any(|endpoint| endpoint.tag == slug)
        .then_some(slug)
}

/// Process-wide cache of attachment URL to its base64 encoding, so a voice message is downloaded
/// and encoded at most once rather than on every generation whose context still carries it.
static AUDIO_CACHE: LazyLock<Mutex<HashMap<String, EncodedAudio>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The base64 encoding of the audio at `url`, downloading and encoding it on the first call and
/// serving later calls from the process-wide cache.
///
/// `OpenRouter` accepts audio only as base64 `input_audio`, so this is needed both to transcribe a
/// voice message and to send it natively to an audio-capable model.
pub async fn fetch_audio_base64(url: &str) -> Result<EncodedAudio, LlmError> {
    if let Some(cached) = AUDIO_CACHE
        .lock()
        .ok()
        .and_then(|locked| locked.get(url).cloned())
    {
        return Ok(cached);
    }
    let encoded = download_audio_base64(url).await?;
    if let Ok(mut locked) = AUDIO_CACHE.lock() {
        locked.insert(url.to_owned(), encoded.clone());
    }
    Ok(encoded)
}

/// Downloads the audio at `url` and base64-encodes it, detecting the format from the URL.
async fn download_audio_base64(url: &str) -> Result<EncodedAudio, LlmError> {
    let response = http().get(url).send().await.context(FetchAudioSnafu)?;
    let status = response.status();
    if !status.is_success() {
        return FetchAudioHttpSnafu {
            status: status.as_u16(),
        }
        .fail();
    }
    let bytes = response.bytes().await.context(FetchAudioSnafu)?;
    let data = STANDARD.encode(&bytes);
    Ok(EncodedAudio {
        url: url.to_owned(),
        data,
        format: audio_format_from_url(url).unwrap_or(AudioMediaType::OGG),
    })
}

/// Caps the utility call's output at [`MAX_TOKENS`], since these bodies never
/// set `max_tokens` themselves and `OpenRouter` otherwise defaults to 65536,
/// reserving more output tokens than the balance can cover on pricey models.
fn fields_insert_max_tokens(body: &mut serde_json::Value) {
    if let Some(fields) = body.as_object_mut() {
        fields.insert("max_tokens".to_owned(), serde_json::json!(MAX_TOKENS));
    }
}

/// Adds the `provider` routing field pinning the request to the configured slug,
/// but only when the body targets the pinned model.
///
/// The utility calls in this module run the vision, audio, and tag models, and a
/// slug names one endpoint of one model, so sending the pin with any of them
/// would ask `OpenRouter` for a provider that does not serve it.
fn fields_insert_provider(body: &mut serde_json::Value, settings: &ModelSettings) {
    let Some(provider) = settings.provider.as_deref() else {
        return;
    };
    let targets_pinned_model = body
        .get("model")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|model| model == settings.model);
    if !targets_pinned_model {
        return;
    }
    if let Some(fields) = body.as_object_mut() {
        fields.insert(
            "provider".to_owned(),
            serde_json::json!({
                "only": [provider],
                "allow_fallbacks": false,
            }),
        );
    }
}

/// Whether the model with `model_id` lists `modality` among its accepted input modalities.
fn model_supports_modality(catalog: &[ModelEntry], model_id: &str, modality: &str) -> bool {
    catalog
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

/// Whether the model with `model_id` always reasons and rejects `effort: "none"`.
///
/// An unlisted model is treated as able to turn reasoning off, so a catalog
/// fetch failure never blocks a request that would otherwise have worked.
fn model_reasoning_mandatory(catalog: &[ModelEntry], model_id: &str) -> bool {
    catalog
        .iter()
        .find(|entry| entry.id == model_id)
        .is_some_and(|entry| entry.reasoning.mandatory)
}

/// Pulls the human-readable message out of an `OpenRouter` error body
/// (`{"error": {"message": ...}}`), falling back to a generic phrase when the
/// body is not that envelope or the message is blank.
fn api_error_message(body: &str) -> String {
    /// The envelope `OpenRouter` wraps an error response in.
    #[derive(Deserialize)]
    struct ErrorBody {
        /// The error details.
        error: ErrorDetail,
    }
    /// The details of an `OpenRouter` error.
    #[derive(Deserialize)]
    struct ErrorDetail {
        /// The human-readable error message.
        #[serde(default)]
        message: String,
    }
    serde_json::from_str::<ErrorBody>(body)
        .ok()
        .map(|parsed| parsed.error.message.trim().to_owned())
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| "okänt fel".to_owned())
}

/// Pulls the description text out of a chat-completions response, if any non-blank content exists.
fn extract_description(response: &ChatResponse) -> Option<String> {
    response
        .choices
        .first()
        .map(|choice| choice.message.content.trim().to_owned())
        .filter(|content| !content.is_empty())
}

/// An `OpenRouter` routing rejection: the request named a provider pin that
/// cannot serve the model, and the providers that can are listed alongside.
#[derive(Debug, PartialEq, Eq)]
pub struct ProviderRejection {
    /// The model the request targeted.
    pub model: String,
    /// The provider slugs that do serve it, which the pin could be moved to.
    pub available: Vec<String>,
}

/// Recognizes the provider-routing rejection `OpenRouter` returns for a pin it
/// cannot satisfy, reading the model and the servable slugs out of the routing
/// metadata it reports.
///
/// Returns `None` for any other failure, since a rejection is told apart by
/// carrying routing metadata rather than by its status alone.
pub fn provider_rejection(body: &str) -> Option<ProviderRejection> {
    /// The envelope `OpenRouter` wraps an error response in.
    #[derive(Deserialize)]
    struct ErrorBody {
        /// The error details.
        error: ErrorDetail,
    }
    /// The details of an `OpenRouter` error.
    #[derive(Deserialize)]
    struct ErrorDetail {
        /// The routing metadata naming the providers that were and were not allowed.
        #[serde(default)]
        metadata: Option<RoutingMetadata>,
    }
    /// The routing metadata of a rejected request.
    #[derive(Deserialize)]
    struct RoutingMetadata {
        /// The providers that do serve the model.
        #[serde(default)]
        available_providers: Vec<String>,
    }

    let parsed = serde_json::from_str::<ErrorBody>(body).ok()?;
    let available = parsed.error.metadata?.available_providers;
    if available.is_empty() {
        return None;
    }
    let model = rejected_model(&api_error_message(body));
    Some(ProviderRejection { model, available })
}

/// Recovers the targeted model from the rejection message, which names it as
/// "Providers serving <model>: <slugs>".
///
/// Falls back to an empty string, since the model is a nicety for the message
/// and the available slugs are the part that lets the user fix the pin.
fn rejected_model(message: &str) -> String {
    const SERVING: &str = "Providers serving ";
    let Some(rest) = message.split_once(SERVING).map(|(_, tail)| tail) else {
        return String::new();
    };
    rest.split_once(':')
        .map(|(model, _)| model.trim().to_owned())
        .unwrap_or_default()
}

/// Reads a provider rejection out of a failed reply stream, so a pin that
/// `OpenRouter` refuses can be reported to the user with the slugs that would
/// have worked instead of a generic failure.
///
/// Returns `None` unless the stream failed with a non-success HTTP response
/// whose body is a routing rejection, leaving every other failure to the
/// generic notice.
pub fn rejection_of(error: &StreamingError) -> Option<ProviderRejection> {
    let StreamingError::Completion(CompletionError::HttpError(http)) = error else {
        return None;
    };
    let body = match http {
        RigError::InvalidStatusCodeWithDetails { body, .. }
        | RigError::InvalidStatusCodeWithMessage(_, body) => body.as_str(),
        _ => return None,
    };
    provider_rejection(body)
}

/// Turns a failed `OpenRouter` response body into the provider error, for the
/// utility calls that do not stream and read the body themselves.
///
/// Returns `None` for any other failure, which stays an
/// [`LlmError::Http`] carrying the status and the API's own message.
fn rejection_error(body: &str) -> Option<LlmError> {
    let rejection = provider_rejection(body)?;
    Some(LlmError::ProviderRejected {
        model: rejection.model,
        available: rejection.available.join(", "),
    })
}

/// The most choices Discord shows in an autocomplete response, the cap on both
/// the model-id and the provider-slug lists.
const MAX_MODEL_CHOICES: usize = 25;

/// Filters the catalog to the model ids matching `partial` (case-insensitive
/// substring), optionally restricted to models accepting `modality` as input,
/// sorted alphabetically and capped at Discord's autocomplete choice limit.
pub fn matching_model_ids(
    catalog: &[ModelEntry],
    partial: &str,
    modality: Option<&str>,
) -> Vec<String> {
    let needle = partial.to_lowercase();
    let mut ids = catalog
        .iter()
        .filter(|entry| {
            modality.is_none_or(|wanted| {
                entry
                    .architecture
                    .input_modalities
                    .iter()
                    .any(|listed| listed == wanted)
            })
        })
        .filter(|entry| entry.id.to_lowercase().contains(&needle))
        .map(|entry| entry.id.clone())
        .collect::<Vec<String>>();
    ids.sort_unstable();
    ids.truncate(MAX_MODEL_CHOICES);
    ids
}

/// Filters `endpoints` to those whose slug or display name matches `partial`
/// (case-insensitive substring), capped at Discord's autocomplete choice limit.
///
/// The provider a user wants is named either way round ("baidu" or "Baidu"), so
/// both are matched, and an exact slug match sorts first so pinning a specific
/// endpoint is one keystroke away.
pub fn matching_endpoints(endpoints: &[EndpointEntry], partial: &str) -> Vec<EndpointEntry> {
    let needle = partial.to_lowercase();
    let mut matched = endpoints
        .iter()
        .filter(|endpoint| {
            endpoint.tag.to_lowercase().contains(&needle)
                || endpoint.provider_name.to_lowercase().contains(&needle)
        })
        .cloned()
        .collect::<Vec<EndpointEntry>>();
    matched.sort_by(|left, right| {
        let exact = |entry: &EndpointEntry| i32::from(!entry.tag.eq_ignore_ascii_case(partial));
        exact(right)
            .cmp(&exact(left))
            .then_with(|| left.tag.cmp(&right.tag))
    });
    matched.truncate(MAX_MODEL_CHOICES);
    matched
}

/// The subset of `OpenRouter`'s model-list response we care about.
#[derive(Deserialize)]
struct ModelsResponse {
    /// The listed models.
    #[serde(default)]
    data: Vec<ModelEntry>,
}

/// One model in `OpenRouter`'s model list.
#[derive(Clone, Deserialize)]
pub struct ModelEntry {
    /// The model identifier, for example `deepseek/deepseek-v3.2`.
    id: String,
    /// The model's input/output modality metadata.
    #[serde(default)]
    architecture: Architecture,
    /// The model's reasoning metadata.
    #[serde(default)]
    reasoning: Reasoning,
}

/// A model's modality metadata.
#[derive(Clone, Default, Deserialize)]
struct Architecture {
    /// The input modalities the model accepts, for example `text` and `image`.
    #[serde(default)]
    input_modalities: Vec<String>,
}

/// The subset of a model's endpoint listing we care about.
#[derive(Deserialize)]
struct EndpointsResponse {
    /// The model and the providers serving it.
    data: EndpointsData,
}

/// One model's endpoint listing.
#[derive(Deserialize)]
struct EndpointsData {
    /// The providers currently serving the model.
    #[serde(default)]
    endpoints: Vec<EndpointEntry>,
}

/// One provider endpoint serving a model.
#[derive(Clone, Deserialize)]
pub struct EndpointEntry {
    /// The slug identifying this endpoint in the `provider` routing fields, for
    /// example `baidu/fp8` or `google-vertex/us-east5`.
    tag: String,
    /// The provider's display name, for example `Baidu`.
    provider_name: String,
    /// The quantization the endpoint serves, for example `fp8`.
    #[serde(default)]
    quantization: String,
}

impl EndpointEntry {
    /// The slug identifying this endpoint in the `provider` routing fields.
    #[must_use]
    pub const fn tag(&self) -> &str {
        self.tag.as_str()
    }

    /// The choice label: the provider name and its quantization, falling back to
    /// the slug alone when the quantization is uninteresting.
    pub fn label(&self) -> String {
        if self.quantization.is_empty() || self.quantization == "unknown" {
            self.tag.clone()
        } else {
            format!("{} ({})", self.provider_name, self.quantization)
        }
    }
}

/// A model's reasoning metadata, describing whether reasoning can be turned off.
#[derive(Clone, Default, Deserialize)]
struct Reasoning {
    /// Whether the model always reasons and rejects `effort: "none"`.
    #[serde(default)]
    mandatory: bool,
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

/// Tests for the `OpenRouter` response parsing helpers.
#[cfg(test)]
mod tests {
    use super::{
        Architecture, ChatResponse, EndpointsResponse, LlmError, MAX_MODEL_CHOICES, ModelEntry,
        ModelsResponse, Reasoning, api_error_message, extract_description, fields_insert_provider,
        matching_endpoints, matching_model_ids, model_reasoning_mandatory, model_supports_modality,
        parse_dialogue_turns, pin_for_model, provider_rejection, rejection_error, rejection_of,
    };
    use crate::llm::ModelSettings;
    use reqwest::StatusCode;
    use rig::agent::StreamingError;
    use rig::completion::CompletionError;
    use rig::http_client::{Error as RigError, HeaderMap};

    /// Builds an endpoint entry with the given slug, provider name, and quantization.
    fn endpoint(tag: &str, provider_name: &str, quantization: &str) -> super::EndpointEntry {
        super::EndpointEntry {
            tag: tag.to_owned(),
            provider_name: provider_name.to_owned(),
            quantization: quantization.to_owned(),
        }
    }

    /// Builds a settings value pinned to `provider` on `model`.
    fn pinned(model: &str, provider: &str) -> ModelSettings {
        ModelSettings {
            model: model.to_owned(),
            provider: Some(provider.to_owned()),
            ..ModelSettings::default()
        }
    }

    /// Builds a catalog entry with the given id and input modalities.
    fn entry(id: &str, modalities: &[&str]) -> ModelEntry {
        ModelEntry {
            id: id.to_owned(),
            architecture: Architecture {
                input_modalities: modalities
                    .iter()
                    .map(|&modality| modality.to_owned())
                    .collect(),
            },
            reasoning: Reasoning::default(),
        }
    }

    /// Builds a catalog entry for a model that cannot turn reasoning off.
    fn mandatory_entry(id: &str) -> ModelEntry {
        let mut model = entry(id, &["text"]);
        model.reasoning.mandatory = true;
        model
    }

    /// An empty partial matches every model, sorted alphabetically.
    #[test]
    fn matching_model_ids_lists_all_on_empty_partial() {
        let catalog = vec![
            entry("vendor/zebra", &["text"]),
            entry("vendor/alpha", &["text"]),
        ];
        assert_eq!(
            matching_model_ids(&catalog, "", None),
            vec!["vendor/alpha".to_owned(), "vendor/zebra".to_owned()],
            "an empty partial lists the whole catalog alphabetically"
        );
    }

    /// The partial is matched as a case-insensitive substring anywhere in the id.
    #[test]
    fn matching_model_ids_matches_substring_case_insensitively() {
        let catalog = vec![
            entry("deepseek/deepseek-v3.2", &["text"]),
            entry("openai/gpt-6", &["text"]),
        ];
        assert_eq!(
            matching_model_ids(&catalog, "DeepSeek", None),
            vec!["deepseek/deepseek-v3.2".to_owned()],
            "a differently-cased partial still matches by substring"
        );
        assert_eq!(
            matching_model_ids(&catalog, "gpt", None),
            vec!["openai/gpt-6".to_owned()],
            "a partial matching mid-id still matches"
        );
        assert!(
            matching_model_ids(&catalog, "claude", None).is_empty(),
            "a partial matching nothing yields no choices"
        );
    }

    /// A required modality keeps only models listing it among their inputs.
    #[test]
    fn matching_model_ids_filters_by_modality() {
        let catalog = vec![
            entry("vendor/sees", &["text", "image"]),
            entry("vendor/hears", &["text", "audio"]),
            entry("vendor/text", &["text"]),
        ];
        assert_eq!(
            matching_model_ids(&catalog, "", Some("image")),
            vec!["vendor/sees".to_owned()],
            "an image filter keeps only vision-capable models"
        );
        assert_eq!(
            matching_model_ids(&catalog, "", Some("audio")),
            vec!["vendor/hears".to_owned()],
            "an audio filter keeps only audio-capable models"
        );
    }

    /// The choice list is capped at Discord's autocomplete limit.
    #[test]
    fn matching_model_ids_caps_at_the_choice_limit() {
        let catalog = (0_u8..30_u8)
            .map(|index| entry(&format!("vendor/model-{index:02}"), &["text"]))
            .collect::<Vec<_>>();
        let ids = matching_model_ids(&catalog, "", None);
        assert_eq!(
            ids.len(),
            MAX_MODEL_CHOICES,
            "the list is truncated to Discord's choice limit"
        );
        assert_eq!(
            ids.first().map(String::as_str),
            Some("vendor/model-00"),
            "truncation keeps the alphabetically first ids"
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

    /// The API's own message is pulled out of the error envelope, with a
    /// generic fallback for anything else.
    #[test]
    fn api_error_message_reads_the_envelope() {
        assert_eq!(
            api_error_message(r#"{"error":{"message":"Insufficient credits","code":402}}"#),
            "Insufficient credits",
            "the envelope's message is used"
        );
        assert_eq!(
            api_error_message(r#"{"error":{"message":"  "}}"#),
            "okänt fel",
            "a blank message falls back to the generic phrase"
        );
        assert_eq!(
            api_error_message("<html>gateway timeout</html>"),
            "okänt fel",
            "a non-JSON body falls back to the generic phrase"
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
            model_supports_modality(&models.data, "vendor/sees", "image"),
            "a model listing image input supports vision"
        );
        assert!(
            !model_supports_modality(&models.data, "vendor/text", "image"),
            "a model without image input does not support vision"
        );
        assert!(
            model_supports_modality(&models.data, "vendor/multi", "audio"),
            "a model listing audio input supports audio"
        );
        assert!(
            !model_supports_modality(&models.data, "vendor/sees", "audio"),
            "a vision-only model does not support audio"
        );
        assert!(
            !model_supports_modality(&models.data, "vendor/missing", "audio"),
            "an unlisted model is treated as supporting no modality"
        );
    }

    /// Only a model the catalog flags mandatory counts; unlisted and regular
    /// models can turn reasoning off.
    #[test]
    fn model_reasoning_mandatory_checks_the_catalog_flag() {
        let catalog = vec![
            mandatory_entry("openai/gpt-6-astra"),
            entry("deepseek/deepseek-v3.2", &["text"]),
        ];

        assert!(
            model_reasoning_mandatory(&catalog, "openai/gpt-6-astra"),
            "a mandatory model cannot turn reasoning off"
        );
        assert!(
            !model_reasoning_mandatory(&catalog, "deepseek/deepseek-v3.2"),
            "a regular model can turn reasoning off"
        );
        assert!(
            !model_reasoning_mandatory(&catalog, "vendor/unlisted"),
            "an unlisted model is optimistically able to turn reasoning off"
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

    /// The endpoint listing parses, and a pin survives only for a model that
    /// actually serves that slug.
    #[test]
    fn pin_for_model_keeps_only_an_offered_slug() {
        let json = r#"{"data":{"id":"vendor/model","endpoints":[
            {"tag":"baidu/fp8","provider_name":"Baidu","quantization":"fp8"},
            {"tag":"wafer","provider_name":"Wafer","quantization":"unknown"}
        ]}}"#;
        let parsed = serde_json::from_str::<EndpointsResponse>(json);
        assert!(parsed.is_ok(), "the endpoint listing should parse");
        let Ok(listing) = parsed else { return };
        let endpoints = &listing.data.endpoints;

        assert_eq!(
            pin_for_model(Some("baidu/fp8"), endpoints),
            Some("baidu/fp8"),
            "a slug the model serves is kept"
        );
        assert_eq!(
            pin_for_model(Some("vendor/elsewhere"), endpoints),
            None,
            "a slug the model does not serve is dropped, since it would fail every request"
        );
        assert_eq!(
            pin_for_model(None, endpoints),
            None,
            "an unpinned model stays unpinned"
        );
        assert_eq!(
            pin_for_model(Some("baidu/fp8"), &[]),
            None,
            "a pin is dropped when the lookup yields nothing, since it cannot be confirmed"
        );
    }

    /// A pin rides along only for the pinned model: the utility calls run the
    /// vision, audio, and tag models, which the slug does not name.
    #[test]
    fn provider_field_is_added_only_for_the_pinned_model() {
        let settings = pinned("vendor/main", "vendor/fp8");

        let mut own = serde_json::json!({ "model": "vendor/main" });
        fields_insert_provider(&mut own, &settings);
        assert_eq!(
            own.get("provider"),
            Some(&serde_json::json!({
                "only": ["vendor/fp8"],
                "allow_fallbacks": false,
            })),
            "a body naming the pinned model carries the hard pin"
        );

        let mut other = serde_json::json!({ "model": "vendor/vision" });
        fields_insert_provider(&mut other, &settings);
        assert!(
            other.get("provider").is_none(),
            "a body naming another model is left unrouted, since the slug names \
             one endpoint of the pinned model only"
        );

        let mut unpinned = serde_json::json!({ "model": "vendor/main" });
        fields_insert_provider(&mut unpinned, &ModelSettings::default());
        assert!(
            unpinned.get("provider").is_none(),
            "an unpinned model sends no provider field at all"
        );
    }

    /// The provider picker matches a partial against the slug and the display
    /// name either way round, and an exact slug sorts first.
    #[test]
    fn matching_endpoints_filters_and_ranks() {
        let endpoints = vec![
            endpoint("deepinfra/fp8", "DeepInfra", "fp8"),
            endpoint("baidu/fp8", "Baidu", "fp8"),
            endpoint("wafer", "Wafer", "unknown"),
        ];

        assert_eq!(
            matching_endpoints(&endpoints, "baidu")
                .into_iter()
                .map(|entry| entry.tag().to_owned())
                .collect::<Vec<String>>(),
            vec!["baidu/fp8".to_owned()],
            "a partial matching the slug finds that endpoint"
        );
        assert_eq!(
            matching_endpoints(&endpoints, "deepinfra").len(),
            1,
            "a partial matching the slug mid-string still matches"
        );
        assert_eq!(
            matching_endpoints(&endpoints, "Baidu").len(),
            1,
            "the display name matches case-insensitively too"
        );
        assert!(
            matching_endpoints(&endpoints, "nonesuch").is_empty(),
            "a partial matching nothing yields no choices"
        );

        let ambiguous = vec![
            endpoint("vendor/zulu", "Vendor", "fp8"),
            endpoint("vendor/alpha", "Vendor", "fp8"),
        ];
        let ranked = matching_endpoints(&ambiguous, "vendor")
            .into_iter()
            .map(|entry| entry.tag().to_owned())
            .collect::<Vec<String>>();
        assert_eq!(
            ranked,
            vec!["vendor/alpha".to_owned(), "vendor/zulu".to_owned()],
            "an exact-slug-less partial falls back to alphabetical order"
        );
    }

    /// The picker is capped at Discord's autocomplete choice limit, like the
    /// model-id picker.
    #[test]
    fn matching_endpoints_caps_at_the_choice_limit() {
        let endpoints = (0_u8..30_u8)
            .map(|index| endpoint(&format!("vendor/provider-{index:02}"), "Vendor", "fp8"))
            .collect::<Vec<_>>();
        assert_eq!(
            matching_endpoints(&endpoints, "vendor").len(),
            MAX_MODEL_CHOICES,
            "the list is truncated to Discord's choice limit"
        );
    }

    /// The label names the provider and its quantization, and falls back to the
    /// bare slug when the quantization says nothing useful.
    #[test]
    fn endpoint_label_names_the_provider_and_quantization() {
        assert_eq!(
            endpoint("baidu/fp8", "Baidu", "fp8").label(),
            "Baidu (fp8)",
            "a known quantization is shown beside the provider"
        );
        assert_eq!(
            endpoint("wafer", "Wafer", "unknown").label(),
            "wafer",
            "an unknown quantization falls back to the slug alone"
        );
    }

    /// The exact body `OpenRouter` returns when a pin cannot be satisfied is
    /// recognized, and the providers that do serve the model are extracted, so
    /// the user can be told which slug to pick instead.
    #[test]
    fn provider_rejection_names_the_providers_that_could_serve() {
        let body = r#"{"error":{"message":"No allowed providers are available for the selected model. Providers serving openai/gpt-3.5-turbo: openai, but your request's provider.only preference permits only: definitely-not-a-real-provider/xyz.","code":404,"metadata":{"available_providers":["openai"],"requested_providers":["definitely-not-a-real-provider/xyz"],"failed_routing_step":"Filter by Allowed Providers"}}}"#;
        let rejection = provider_rejection(body);
        assert_eq!(
            rejection.as_ref().map(|found| found.model.as_str()),
            Some("openai/gpt-3.5-turbo"),
            "the rejected model is named, so the user knows which pin to change"
        );
        assert_eq!(
            rejection.as_ref().map(|found| found.available.as_slice()),
            Some(["openai".to_owned()].as_slice()),
            "the providers that do serve the model are extracted from the routing metadata"
        );
    }

    /// A rejection listing several alternatives keeps them all, so the user is
    /// not left guessing which slug would work.
    #[test]
    fn provider_rejection_keeps_every_available_provider() {
        let body = r#"{"error":{"message":"No allowed providers are available.","code":404,"metadata":{"available_providers":["baidu/fp8","wafer","novita/fp8"],"requested_providers":["gone"],"failed_routing_step":"Filter by Allowed Providers"}}}"#;
        let rejection = provider_rejection(body);
        assert_eq!(
            rejection.as_ref().map(|found| found.available.len()),
            Some(3),
            "each provider that could serve the model is kept"
        );
    }

    /// An ordinary `OpenRouter` failure is not a provider rejection, and a body
    /// that is not that envelope at all parses to nothing rather than panicking.
    #[test]
    fn provider_rejection_ignores_unrelated_failures() {
        for body in [
            r#"{"error":{"message":"No credits","code":402}}"#,
            r#"{"error":{"message":"No allowed providers are available."}}"#,
            "not json at all",
            "",
        ] {
            assert!(
                provider_rejection(body).is_none(),
                "a body without routing metadata is not a provider rejection"
            );
        }
    }

    /// Builds a rig streaming error carrying `body` as a failed `OpenRouter`
    /// response, the shape a provider rejection arrives in.
    fn rejected_stream(body: &str) -> StreamingError {
        StreamingError::Completion(CompletionError::HttpError(
            RigError::InvalidStatusCodeWithDetails {
                status: StatusCode::NOT_FOUND,
                body: body.to_owned(),
                headers: Box::new(HeaderMap::new()),
            },
        ))
    }

    /// A `StreamingError` carrying the real 404 body is recognized as a rejected
    /// pin, and the slugs that would work are extracted from it. This is the glue
    /// between rig's error and the user-facing notice.
    #[test]
    fn a_rejected_pin_is_recognized_from_a_streaming_error() {
        let body = r#"{"error":{"message":"No allowed providers are available for the selected model. Providers serving vendor/main: wafer, novita/fp8, but your request's provider.only preference permits only: gone/now.","code":404,"metadata":{"available_providers":["wafer","novita/fp8"],"requested_providers":["gone/now"],"failed_routing_step":"Filter by Allowed Providers"}}}"#;
        let rejection = rejection_of(&rejected_stream(body));
        assert_eq!(
            rejection.map(|found| (found.model, found.available.join(", "))),
            Some(("vendor/main".to_owned(), "wafer, novita/fp8".to_owned())),
            "the rejection is read out of the streaming error, ready for display"
        );
    }

    /// A stream error that is not a routing rejection yields nothing, so an
    /// ordinary failure keeps its existing generic notice.
    #[test]
    fn an_ordinary_stream_error_is_not_a_rejection() {
        assert!(
            rejection_of(&rejected_stream(
                r#"{"error":{"message":"No credits","code":402}}"#
            ))
            .is_none(),
            "a non-routing failure is not reported as a rejection"
        );
        assert!(
            rejection_of(&StreamingError::Completion(CompletionError::ResponseError(
                "Response did not contain a valid message or tool call".into()
            )))
            .is_none(),
            "a malformed response is not reported as a rejection"
        );
    }

    /// A failed utility call whose body is a routing rejection becomes the
    /// dedicated provider error, carrying the slugs that would have worked, so
    /// a vision or tag model pinned to a dead provider says so.
    #[test]
    fn a_utility_call_rejection_becomes_the_provider_error() {
        let body = r#"{"error":{"message":"No allowed providers are available for the selected model. Providers serving vendor/main: wafer, but your request's provider.only preference permits only: gone/now.","code":404,"metadata":{"available_providers":["wafer","novita/fp8"],"requested_providers":["gone/now"]}}}"#;
        let error = rejection_error(body);
        assert!(
            matches!(
                error,
                Some(LlmError::ProviderRejected { ref model, ref available })
                    if model == "vendor/main" && available == "wafer, novita/fp8"
            ),
            "a routing rejection becomes the provider error naming the model and \
             the slugs that would work, got {error:?}"
        );
    }

    /// An ordinary non-success response stays the generic HTTP error, so a bad
    /// key or an empty balance is not misreported as a provider problem.
    #[test]
    fn an_ordinary_utility_failure_stays_an_http_error() {
        for body in [
            r#"{"error":{"message":"No credits","code":402}}"#,
            r#"{"error":{"message":"No allowed providers are available."}}"#,
        ] {
            assert!(
                rejection_error(body).is_none(),
                "a failure without routing metadata keeps the generic HTTP error"
            );
        }
    }
}
