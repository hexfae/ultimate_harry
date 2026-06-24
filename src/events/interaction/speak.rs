//! Helpers shared by the two read-aloud handlers (the speak button in `tts` and
//! the voice dropdown in `voice`): the filename timestamp, audio-tag enrichment,
//! and posting the synthesized MP3 as a followup attachment.

use crate::{
    AppResult,
    database::Database,
    error::SendResponseSnafu,
    llm::LlmManager,
    models::character::Character,
    tts::{TtsSettings, audio_filename},
    util::report_error,
};
use jiff::{Timestamp, Zoned};
use serenity::all::{
    ComponentInteraction, Context, CreateAttachment, CreateInteractionResponseFollowup,
};
use snafu::ResultExt as _;
use tracing::warn;

/// The timezone the audio filename's request timestamp is rendered in.
const FILENAME_TIMEZONE: &str = "Europe/Stockholm";

/// The moment the read-aloud was requested, in the filename timezone, used to
/// name the MP3; falls back to the system zone if the named zone is unavailable.
pub(super) fn requested_now() -> Zoned {
    Timestamp::now()
        .in_tz(FILENAME_TIMEZONE)
        .unwrap_or_else(|_| Zoned::now())
}

/// Enriches `text` with v3 audio tags when a tag model is enabled for the
/// effective synthesis `model`, falling back to the plain text on failure. Keyed
/// off `model` so a voice pinned to a non-v3 model skips the tags.
pub(super) async fn enrich(
    db: &Database,
    settings: &TtsSettings,
    model: &str,
    text: String,
) -> String {
    let Some(tag_model) = settings.tag_model_for(model) else {
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
