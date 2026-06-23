//! The button that speaks a character's reply aloud via `ElevenLabs` text-to-speech.

use crate::{
    AppResult,
    database::Database,
    error::SendResponseSnafu,
    models::{character::Character, history::History},
    tts::{TtsError, TtsManager},
};
use serenity::all::{
    ComponentInteraction, Context, CreateAttachment, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateInteractionResponseMessage,
};
use snafu::ResultExt as _;

/// The filename of the synthesized audio attachment.
const AUDIO_FILENAME: &str = "uppläsning.mp3";

/// Speak the chosen reply aloud, posting it as an MP3 followup attachment.
///
/// The voice is the character's own linked voice, falling back to the configured
/// generic default; with neither, it fails with a notice rather than staying silent.
/// An empty reply has nothing to speak, but the button is disabled in that case (see
/// the render in `history/render.rs`), so this is never reached for blank text.
/// Synthesis can take longer than Discord's three-second window, so the interaction
/// is deferred before the (slow) request is made.
pub async fn tts(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    history: History,
    character: Character,
) -> AppResult {
    let manager = TtsManager::new(db.tts_settings().await);
    let voice = manager.voice_for(&character).ok_or(TtsError::NoVoice)?;
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

    let audio = manager.synthesize(&text, &voice).await?;
    let attachment = CreateAttachment::bytes(audio, AUDIO_FILENAME);

    interaction
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new().add_file(attachment),
        )
        .await
        .context(SendResponseSnafu)?;
    Ok(())
}
