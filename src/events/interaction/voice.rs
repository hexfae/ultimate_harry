//! The dropdown that speaks a reply aloud, either in a chosen palette voice or
//! with voices auto-assigned per speaker via `ElevenLabs` text-to-dialogue.

use crate::{
    AppResult,
    database::Database,
    error::SendResponseSnafu,
    llm::{LlmManager, VoiceChoice},
    models::{character::Character, history::History},
    tts::{DialogueTurn, TtsError, TtsManager, TtsSettings, audio_filename},
    util::report_error,
};
use jiff::{Timestamp, Zoned};
use serenity::all::{
    ComponentInteraction, ComponentInteractionDataKind, Context, CreateAttachment,
    CreateInteractionResponse, CreateInteractionResponseFollowup, CreateInteractionResponseMessage,
};
use snafu::ResultExt as _;
use tracing::warn;

/// The timezone the audio filename's request timestamp is rendered in.
const FILENAME_TIMEZONE: &str = "Europe/Stockholm";

/// The select value that requests auto voice assignment rather than a fixed voice.
const AUTO_VALUE: &str = "auto";

/// Speak the chosen reply aloud using the voice picked in the dropdown.
///
/// `auto` segments the reply into speaker turns, assigns each a palette voice by
/// description (the character's own voice wins for its own lines), and speaks
/// them via the multi-voice text-to-dialogue endpoint; a specific palette voice
/// instead speaks the whole reply in that one voice. Audio is posted as an MP3
/// followup, like the plain speak button. Synthesis can outlast Discord's
/// three-second window, so the interaction is deferred first.
pub async fn voice(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    history: History,
    character: Character,
) -> AppResult {
    let ComponentInteractionDataKind::StringSelect { ref values } = interaction.data.kind else {
        return Ok(());
    };
    let Some(selection) = values.first() else {
        return Ok(());
    };

    let requested_at = Timestamp::now()
        .in_tz(FILENAME_TIMEZONE)
        .unwrap_or_else(|_| Zoned::now());
    let settings = db.tts_settings().await;
    let manager = TtsManager::new(settings.clone());
    let fallback = manager
        .voice_for(&character)
        .or_else(|| settings.voices().first().map(|voice| voice.voice_id.clone()))
        .ok_or(TtsError::NoVoice)?;
    let text = history
        .chosen_message()
        .chosen_revision()
        .head()
        .content()
        .to_owned();

    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new()),
        )
        .await
        .context(SendResponseSnafu)?;

    let audio = if selection == AUTO_VALUE {
        synthesize_auto(db, &settings, &manager, &character, &fallback, text).await?
    } else {
        let speak_text = enrich(db, &settings, text).await;
        manager.synthesize(&speak_text, selection).await?
    };

    let attachment = CreateAttachment::bytes(audio, audio_filename(character.name(), &requested_at));
    interaction
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new().add_file(attachment),
        )
        .await
        .context(SendResponseSnafu)?;
    Ok(())
}

/// Synthesizes the reply with auto-assigned voices, falling back to a single
/// voice when assignment yields nothing usable or only one distinct voice.
async fn synthesize_auto(
    db: &Database,
    settings: &TtsSettings,
    manager: &TtsManager,
    character: &Character,
    fallback: &str,
    text: String,
) -> Result<Vec<u8>, TtsError> {
    let Some(turns) = assign_turns(db, settings, character, fallback, &text).await else {
        let speak_text = enrich(db, settings, text).await;
        return manager.synthesize(&speak_text, fallback).await;
    };
    let distinct = {
        let mut ids: Vec<&str> = turns.iter().map(|turn| turn.voice_id.as_str()).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    };
    if distinct <= 1 {
        let joined = turns
            .iter()
            .map(|turn| turn.text.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let voice = turns
            .first()
            .map_or(fallback, |turn| turn.voice_id.as_str());
        return manager.synthesize(&joined, voice).await;
    }
    manager.synthesize_dialogue(&turns).await
}

/// Asks the enricher to split `text` into per-voice turns, validating the
/// returned voice IDs against the allowed set (palette + fallback) and replacing
/// any unknown ID with the fallback. Returns `None` (so the caller speaks a
/// single voice) when no enricher model is configured or assignment fails.
async fn assign_turns(
    db: &Database,
    settings: &TtsSettings,
    character: &Character,
    fallback: &str,
    text: &str,
) -> Option<Vec<DialogueTurn>> {
    let model = settings.tag_model.clone().filter(|model| !model.is_empty())?;
    let mut choices: Vec<VoiceChoice> = settings.voices().iter().map(VoiceChoice::from_entry).collect();
    choices.push(VoiceChoice {
        voice_id: fallback.to_owned(),
        name: character.name().to_owned(),
        description: format!("the main character {} speaking", character.name()),
    });
    let allowed: Vec<String> = choices.iter().map(|choice| choice.voice_id.clone()).collect();

    let llm = LlmManager::new(db.model_settings().await);
    let add_tags = settings.tag_model_if_enabled().is_some();
    match llm.assign_voices(text, &model, &choices, add_tags).await {
        Ok(turns) => Some(
            turns
                .into_iter()
                .map(|mut turn| {
                    if !allowed.contains(&turn.voice_id) {
                        fallback.clone_into(&mut turn.voice_id);
                    }
                    turn
                })
                .collect(),
        ),
        Err(why) => {
            warn!("voice assignment failed, speaking a single voice");
            report_error(why);
            None
        }
    }
}

/// Enriches `text` with v3 audio tags when a tag model is enabled, falling back
/// to the plain text on failure. Mirrors the plain speak button's enrichment.
async fn enrich(db: &Database, settings: &TtsSettings, text: String) -> String {
    let Some(tag_model) = settings.tag_model_if_enabled() else {
        return text;
    };
    let llm = LlmManager::new(db.model_settings().await);
    match llm.add_audio_tags(&text, tag_model).await {
        Ok(tagged) => tagged,
        Err(why) => {
            warn!("audio-tag enhancement failed, speaking the plain reply");
            report_error(why);
            text
        }
    }
}
