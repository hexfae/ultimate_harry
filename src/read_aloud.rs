//! Shared read-aloud orchestration used by the `/säg` command and the speak/voice
//! interaction handlers: audio-tag enrichment, single-voice synthesis, and the
//! auto-assigned multi-voice path.

use crate::{
    database::Database,
    llm::{LlmManager, VoiceChoice},
    tts::{
        DialogueTurn, TtsError, TtsManager, TtsSettings, enforce_allowed_voices, plan_dialogue,
    },
    util::report_error,
};
use jiff::{Timestamp, Zoned};
use tracing::warn;

/// The select/sentinel value that requests auto voice assignment rather than a fixed voice.
pub const AUTO_VALUE: &str = "auto";

/// The timezone the audio filename's request timestamp is rendered in.
const FILENAME_TIMEZONE: &str = "Europe/Stockholm";

/// The moment the read-aloud was requested, in the filename timezone, used to name
/// the MP3; falls back to the system zone if the named zone is unavailable.
pub fn requested_now() -> Zoned {
    Timestamp::now()
        .in_tz(FILENAME_TIMEZONE)
        .unwrap_or_else(|_| Zoned::now())
}

/// Enriches `text` with v3 audio tags when a tag model is enabled for the effective
/// synthesis `model`, falling back to the plain text on failure. Keyed off `model`
/// so a voice pinned to a non-v3 model skips the tags.
async fn enrich(db: &Database, settings: &TtsSettings, model: &str, text: String) -> String {
    let Some(tag_model) = settings.tag_model_for(model) else {
        return text;
    };
    let llm = LlmManager::new(db.model_settings().await);
    match llm.add_audio_tags(&text, tag_model).await {
        Ok(tagged) => tagged,
        Err(why) => {
            warn!("audio-tag enhancement failed, speaking the plain text");
            report_error(why);
            text
        }
    }
}

/// Enriches `text` (when applicable) and synthesizes the whole of it in a single
/// `voice_id` spoken with `model`.
pub async fn synthesize_single(
    db: &Database,
    settings: &TtsSettings,
    manager: &TtsManager,
    voice_id: &str,
    model: &str,
    text: String,
) -> Result<Vec<u8>, TtsError> {
    let speak_text = enrich(db, settings, model, text).await;
    manager.synthesize(&speak_text, voice_id, model).await
}

/// Synthesizes `text` with auto-assigned voices, falling back to a single voice
/// (the `fallback` voice in the configured model) when assignment yields nothing
/// usable or only one distinct voice.
///
/// `extra` is the main character's own voice choice, offered to the enricher
/// alongside the palette: `Some` for the dropdown path (the character wins its own
/// lines), `None` for the free-text command.
pub async fn synthesize_auto(
    db: &Database,
    settings: &TtsSettings,
    manager: &TtsManager,
    fallback: &str,
    extra: Option<VoiceChoice>,
    text: String,
) -> Result<Vec<u8>, TtsError> {
    let Some(turns) = assign_turns(db, settings, fallback, extra, &text).await else {
        return synthesize_single(db, settings, manager, fallback, &settings.model, text).await;
    };
    manager.synthesize_plan(plan_dialogue(turns, fallback)).await
}

/// Asks the enricher to split `text` into per-voice turns over the palette (plus the
/// optional `extra` character voice), validating the returned voice IDs against the
/// allowed set (the offered voices plus `fallback`) and replacing any unknown ID with
/// the fallback. Returns `None` (so the caller speaks a single voice) when no enricher
/// model is set, no voices are offered, or assignment fails.
async fn assign_turns(
    db: &Database,
    settings: &TtsSettings,
    fallback: &str,
    extra: Option<VoiceChoice>,
    text: &str,
) -> Option<Vec<DialogueTurn>> {
    let model = settings.tag_model.clone().filter(|model| !model.is_empty())?;
    let mut choices: Vec<VoiceChoice> =
        settings.voices().iter().map(VoiceChoice::from_entry).collect();
    if let Some(character_voice) = extra {
        choices.push(character_voice);
    }
    if choices.is_empty() {
        return None;
    }
    let mut allowed: Vec<String> = choices.iter().map(|choice| choice.voice_id.clone()).collect();
    if !allowed.iter().any(|id| id == fallback) {
        allowed.push(fallback.to_owned());
    }

    let llm = LlmManager::new(db.model_settings().await);
    let add_tags = settings.tag_model_if_enabled().is_some();
    match llm.assign_voices(text, &model, &choices, add_tags).await {
        Ok(turns) => Some(enforce_allowed_voices(turns, &allowed, fallback)),
        Err(why) => {
            warn!("voice assignment failed, speaking a single voice");
            report_error(why);
            None
        }
    }
}
