//! Shared read-aloud orchestration used by the `/säg` command and the speak/voice
//! interaction handlers: audio-tag enrichment, single-voice synthesis, and the
//! auto-assigned multi-voice path.

use crate::{
    database::Database,
    llm::{LlmManager, VoiceChoice},
    models::character::Character,
    tts::{DialogueTurn, TtsError, TtsManager, TtsSettings, enforce_allowed_voices, plan_dialogue},
    util::report_error,
};
use jiff::{Timestamp, Zoned};
use tracing::warn;

/// The select/sentinel value that requests auto voice assignment rather than a fixed voice.
pub const AUTO_VALUE: &str = "auto";

/// The user-facing label for [`AUTO_VALUE`] in the dropdown and the `/säg` autocomplete.
pub const AUTO_LABEL: &str = "Automatiskt";

/// The voice auto reading falls back to when assignment yields nothing usable:
/// `character`'s own voice, then the configured generic default, then the first
/// palette voice, and `None` when none of those is set.
#[must_use]
pub fn auto_fallback(settings: &TtsSettings, character: Option<&Character>) -> Option<String> {
    character
        .and_then(Character::voice)
        .map(str::to_owned)
        .or_else(|| settings.default_voice.clone())
        .or_else(|| {
            settings
                .voices()
                .first()
                .map(|voice| voice.voice_id.clone())
        })
}

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

/// A synthesized reading: the MP3 bytes plus the exact text sent to `ElevenLabs`,
/// which is the input text after audio-tag enrichment (identical to the input when
/// no tags were added).
pub struct Synthesized {
    /// The synthesized MP3 audio.
    pub audio: Vec<u8>,
    /// The text that was spoken, as sent to the synthesis endpoint.
    pub spoken: String,
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
) -> Result<Synthesized, TtsError> {
    let spoken = enrich(db, settings, model, text).await;
    let audio = manager.synthesize(&spoken, voice_id, model).await?;
    Ok(Synthesized { audio, spoken })
}

/// Synthesizes `text` with auto-assigned voices, falling back to a single voice
/// (the `fallback` voice in the configured model) when assignment yields nothing
/// usable or only one distinct voice.
///
/// `extra` is the main character's own voice choice, offered to the enricher
/// alongside the palette: `Some` for the dropdown path (the character wins its own
/// lines), `None` for the free-text command.
///
/// The returned [`Synthesized::spoken`] is the assigned turns' texts joined by
/// newlines, or the single-voice text when assignment yields nothing.
pub async fn synthesize_auto(
    db: &Database,
    settings: &TtsSettings,
    manager: &TtsManager,
    fallback: &str,
    extra: Option<VoiceChoice>,
    text: String,
) -> Result<Synthesized, TtsError> {
    let Some(turns) = assign_turns(db, settings, fallback, extra, &text).await else {
        return synthesize_single(db, settings, manager, fallback, &settings.model, text).await;
    };
    let spoken = turns
        .iter()
        .map(|turn| turn.text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let audio = manager.synthesize_plan(plan_dialogue(turns, fallback)).await?;
    Ok(Synthesized { audio, spoken })
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
    let model = settings
        .tag_model
        .as_deref()
        .filter(|model| !model.is_empty())?;
    let mut choices: Vec<VoiceChoice> = settings
        .voices()
        .iter()
        .map(VoiceChoice::from_entry)
        .collect();
    if let Some(character_voice) = extra {
        choices.push(character_voice);
    }
    if choices.is_empty() {
        return None;
    }
    let mut allowed: Vec<String> = choices
        .iter()
        .map(|choice| choice.voice_id.clone())
        .collect();
    if !allowed.iter().any(|id| id == fallback) {
        allowed.push(fallback.to_owned());
    }

    let llm = LlmManager::new(db.model_settings().await);
    let add_tags = settings.tag_model_if_enabled().is_some();
    match llm.assign_voices(text, model, &choices, add_tags).await {
        Ok(turns) => Some(enforce_allowed_voices(turns, &allowed, fallback)),
        Err(why) => {
            warn!("voice assignment failed, speaking a single voice");
            report_error(why);
            None
        }
    }
}

/// Tests for the shared auto-reading fallback resolution.
#[cfg(test)]
mod tests {
    use super::auto_fallback;
    use crate::{
        models::character::Character,
        tts::{TtsSettings, VoiceEntry},
    };
    use serenity::all::UserId;

    /// Builds a character with the given linked voice, if any.
    fn character(voice: Option<&str>) -> Character {
        let builder = Character::builder()
            .id("id".to_owned())
            .name("Harry")
            .greeting("hi")
            .creator(UserId::new(1));
        match voice {
            Some(linked) => builder.voice(linked.to_owned()).build(),
            None => builder.build(),
        }
    }

    /// Builds settings carrying the given palette voice and generic default.
    fn settings(palette: Option<&str>, default_voice: Option<&str>) -> TtsSettings {
        let mut settings = TtsSettings {
            default_voice: default_voice.map(str::to_owned),
            ..TtsSettings::default()
        };
        if let Some(voice_id) = palette {
            settings.add_voice(VoiceEntry {
                name: "Adam".to_owned(),
                voice_id: voice_id.to_owned(),
                emoji: "🎙️".to_owned(),
                description: "a test voice".to_owned(),
                model: None,
            });
        }
        settings
    }

    /// A character's own linked voice outranks both the generic default and the palette.
    #[test]
    fn auto_fallback_prefers_the_character_voice() {
        assert_eq!(
            auto_fallback(
                &settings(Some("palette-id"), Some("default-id")),
                Some(&character(Some("harry-id")))
            )
            .as_deref(),
            Some("harry-id"),
            "the character's own voice wins"
        );
    }

    /// Without a character voice the generic default wins, then the first palette voice.
    #[test]
    fn auto_fallback_falls_back_to_default_then_palette() {
        assert_eq!(
            auto_fallback(&settings(Some("palette-id"), Some("default-id")), None).as_deref(),
            Some("default-id"),
            "the configured default outranks the palette"
        );
        assert_eq!(
            auto_fallback(&settings(Some("palette-id"), None), Some(&character(None))).as_deref(),
            Some("palette-id"),
            "the first palette voice is used when nothing else is set"
        );
    }

    /// Nothing configured anywhere leaves no fallback voice.
    #[test]
    fn auto_fallback_is_none_without_any_voice() {
        assert_eq!(
            auto_fallback(&settings(None, None), Some(&character(None))),
            None,
            "no character voice, no default, and no palette leaves nothing"
        );
    }
}
