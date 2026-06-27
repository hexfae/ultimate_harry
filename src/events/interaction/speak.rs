//! Helpers shared by the two read-aloud handlers (the speak button in `tts` and
//! the voice dropdown in `voice`): the setup preamble, deferring the interaction,
//! and posting the synthesized MP3 as a followup attachment.

use crate::{
    AppResult,
    database::Database,
    error::SendResponseSnafu,
    models::{character::Character, history::History},
    tts::{TtsManager, TtsSettings, audio_filename},
};
use jiff::Zoned;
use serenity::all::{
    ComponentInteraction, Context, CreateAttachment, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateInteractionResponseMessage,
};
use snafu::ResultExt as _;

/// The TTS settings, a manager over them, and the chosen reply's spoken text,
/// the shared preamble both read-aloud handlers need before resolving a voice.
pub(super) async fn setup(db: &Database, history: &History) -> (TtsSettings, TtsManager, String) {
    let settings = db.tts_settings().await;
    let manager = TtsManager::new(settings.clone());
    let text = history.chosen_content().to_owned();
    (settings, manager, text)
}

/// Defers the interaction so the slow synthesis request fits within Discord's
/// three-second response window.
pub(super) async fn defer(ctx: &Context, interaction: &ComponentInteraction) -> AppResult {
    interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(CreateInteractionResponseMessage::new()),
        )
        .await
        .context(SendResponseSnafu)?;
    Ok(())
}

/// Posts the synthesized `audio` as an MP3 followup attachment named after the
/// character and `requested_at`.
pub(super) async fn post_followup(
    ctx: &Context,
    interaction: &ComponentInteraction,
    audio: Vec<u8>,
    character: &Character,
    requested_at: &Zoned,
) -> AppResult {
    let attachment = CreateAttachment::bytes(audio, audio_filename(character.name(), requested_at));
    interaction
        .create_followup(
            &ctx.http,
            CreateInteractionResponseFollowup::new().add_file(attachment),
        )
        .await
        .context(SendResponseSnafu)?;
    Ok(())
}
