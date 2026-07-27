//! The button that speaks a character's reply aloud via `ElevenLabs` text-to-speech.

use super::speak;
use crate::{
    AppResult,
    database::Database,
    models::{character::Character, history::History},
    read_aloud,
    tts::TtsError,
};
use serenity::all::{ComponentInteraction, Context};

/// Speak the chosen reply aloud, posting it as an MP3 followup attachment.
///
/// The voice is the character's own linked voice, falling back to the configured
/// generic default; with neither, it fails with a notice rather than staying silent.
/// An empty reply has nothing to speak, but the button is disabled in that case (see
/// the render in `history/render.rs`), so this is never reached for blank text.
/// When a tag model is configured and the synthesis model is audio-tag aware, the
/// spoken text (not the visible reply) is first enriched with `ElevenLabs` v3 audio
/// tags; a failed enrichment falls back to the plain reply rather than blocking audio.
/// Synthesis can take longer than Discord's three-second window, so the interaction
/// is deferred before the (slow) request is made.
pub async fn tts(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    history: History,
    character: Character,
) -> AppResult {
    let requested_at = read_aloud::requested_now();
    let (settings, manager, text) = speak::setup(db, &history).await;
    let voice = manager.voice_for(&character).ok_or(TtsError::NoVoice)?;
    let model = settings.solo_model(&voice).to_owned();

    speak::defer(ctx, interaction).await?;

    let audio =
        read_aloud::synthesize_single(db, &settings, &manager, &voice, &model, text).await?;
    speak::post_followup(ctx, interaction, audio, &character, &requested_at).await
}
