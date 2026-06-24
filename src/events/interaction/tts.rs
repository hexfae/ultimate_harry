//! The button that speaks a character's reply aloud via `ElevenLabs` text-to-speech.

use crate::{
    AppResult,
    database::Database,
    error::SendResponseSnafu,
    llm::LlmManager,
    models::{character::Character, history::History},
    tts::{TtsError, TtsManager, audio_filename},
    util::report_error,
};
use jiff::{Timestamp, Zoned};
use serenity::all::{
    ComponentInteraction, Context, CreateAttachment, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateInteractionResponseMessage,
};
use snafu::ResultExt as _;
use tracing::warn;

/// The timezone the audio filename's request timestamp is rendered in.
const FILENAME_TIMEZONE: &str = "Europe/Stockholm";

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
    let requested_at = Timestamp::now()
        .in_tz(FILENAME_TIMEZONE)
        .unwrap_or_else(|_| Zoned::now());
    let settings = db.tts_settings().await;
    let manager = TtsManager::new(settings.clone());
    let voice = manager.voice_for(&character).ok_or(TtsError::NoVoice)?;
    let model = settings.solo_model(&voice).to_owned();
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

    let speak_text = match settings.tag_model_for(&model) {
        Some(tag_model) => {
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
        None => text,
    };

    let audio = manager.synthesize(&speak_text, &voice, &model).await?;
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
