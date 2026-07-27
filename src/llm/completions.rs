//! The hand-rolled `OpenRouter` REST layer: model-capability probing, image
//! description, audio transcription, audio-tag enrichment, and per-voice
//! assignment, plus their request/response DTOs.
//!
//! Split out from `llm.rs`; a child module so its `impl LlmManager` block can
//! reach the manager's private settings while staying separate from the rig
//! streaming core.

use crate::http::http;
use crate::models::message::{EncodedAudio, audio_format_from_url};
use crate::tts::{DialogueTurn, VoiceEntry};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use rig::message::AudioMediaType;
use serde::Deserialize;
use snafu::{IntoError, OptionExt as _, ResultExt as _};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

use super::{
    AddTagsSnafu, AssignVoicesSnafu, DescribeImageSnafu, EmptyDescriptionSnafu, EmptyTagsSnafu,
    EmptyTranscriptionSnafu, EmptyVoicesSnafu, FetchAudioHttpSnafu, FetchAudioSnafu, HttpSnafu,
    ListModelsSnafu, LlmError, LlmManager, NoAudioModelSnafu, NoVisionModelSnafu,
    TranscribeAudioSnafu,
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

/// The instruction given to the tag model when enriching a reply with audio tags.
const TAG_PROMPT: &str = "You are given a single line of dialogue from a roleplay. Insert ElevenLabs v3 audio tags (square-bracketed, e.g. [laughs], [sighs], [whispers], [angry], [sad]) at fitting points so it sounds expressive when read aloud. Keep all of the original text and its language exactly as given: do not translate, rephrase, or change any words; only add tags. The tags themselves must always be in English, even when the dialogue is in another language. Reply with only the tagged text, no explanation.";

/// The instruction given to the model when splitting a reply into per-voice turns.
const SEGMENT_PROMPT: &str = "You are given a roleplay reply and a list of available voices. Split the reply into an ordered sequence of speaker turns and assign each turn one of the available voices by its id, choosing the voice whose description best matches that speaker. Cover the entire reply in order, keeping every word and its original language exactly as given: do not translate, rephrase, drop, or reorder any text. Use only voice ids from the provided list. Reply with ONLY a JSON array of objects with the keys \"voice_id\" and \"text\", and nothing else (no prose, no code fences).";

/// The extra instruction folded into [`SEGMENT_PROMPT`] when the synthesis model
/// is audio-tag aware, so each turn's text is also enriched with v3 tags.
const SEGMENT_TAG_CLAUSE: &str = " Additionally, insert ElevenLabs v3 audio tags (square-bracketed and always in English, e.g. [laughs], [sighs], [whispers]) at fitting points within each turn's text so it sounds expressive when read aloud.";

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

    /// POSTs `body` to the `OpenRouter` chat-completions endpoint and parses the
    /// reply, attaching `request_error` as the snafu context for both the request
    /// and the JSON decode (they share a failure class per caller). An error
    /// status becomes [`LlmError::Http`] carrying the API's own error message,
    /// so a bad key or empty balance is not mistaken for an empty completion.
    async fn chat_completion<E>(
        &self,
        body: serde_json::Value,
        request_error: E,
    ) -> Result<ChatResponse, LlmError>
    where
        E: IntoError<LlmError, Source = reqwest::Error> + Copy,
    {
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

/// The most choices Discord shows in an autocomplete response.
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
}

/// A model's modality metadata.
#[derive(Clone, Default, Deserialize)]
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

/// Tests for the `OpenRouter` response parsing helpers.
#[cfg(test)]
mod tests {
    use super::{
        Architecture, ChatResponse, MAX_MODEL_CHOICES, ModelEntry, ModelsResponse,
        api_error_message, extract_description, matching_model_ids, model_supports_modality,
        parse_dialogue_turns,
    };

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
        }
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
