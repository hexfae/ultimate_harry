//! The dropdown that speaks a reply aloud, either in a chosen palette voice or
//! with voices auto-assigned per speaker via `ElevenLabs` text-to-dialogue.

use super::speak;
use crate::{
    AppResult,
    database::Database,
    llm::{LlmManager, VoiceChoice},
    models::{character::Character, history::History},
    tts::{DialogueTurn, TtsError, TtsManager, TtsSettings},
    util::report_error,
};
use serenity::all::{ComponentInteraction, ComponentInteractionDataKind, Context};
use tracing::warn;

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

    let requested_at = speak::requested_now();
    let (settings, manager, text) = speak::setup(db, &history).await;
    let fallback = manager
        .voice_for(&character)
        .or_else(|| settings.voices().first().map(|voice| voice.voice_id.clone()))
        .ok_or(TtsError::NoVoice)?;

    speak::defer(ctx, interaction).await?;

    let audio = if selection == AUTO_VALUE {
        synthesize_auto(db, &settings, &manager, &character, &fallback, text).await?
    } else {
        let model = settings.solo_model(selection).to_owned();
        let speak_text = speak::enrich(db, &settings, &model, text).await;
        manager.synthesize(&speak_text, selection, &model).await?
    };

    speak::post_followup(ctx, interaction, audio, &character, &requested_at).await
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
        let speak_text = speak::enrich(db, settings, &settings.model, text).await;
        return manager.synthesize(&speak_text, fallback, &settings.model).await;
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
        return manager.synthesize(&joined, voice, &settings.model).await;
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
