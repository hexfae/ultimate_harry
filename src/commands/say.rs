//! The bot's Discord slash command for reading a message aloud in a chosen voice.

use crate::{
    AppResult, Context,
    commands::autocomplete_from_names,
    error::{SendMessageSnafu, SendResponseSnafu},
    phrases, read_aloud,
    traits::SayEphemeral as _,
    tts::{TtsError, TtsManager, TtsSettings, audio_filename, is_speakable},
    util::ellipsize,
};
use poise::{
    CreateReply,
    serenity_prelude::{CreateAttachment, CreateAutocompleteResponse},
};
use snafu::ResultExt as _;

/// How a message should be read aloud, resolved from the chosen voice.
#[derive(Debug, PartialEq, Eq)]
enum Reading {
    /// Split the message into per-speaker turns and read it in multiple voices.
    Auto,
    /// Read the whole message in a single palette voice with its own model.
    Solo {
        /// The `ElevenLabs` voice ID to read in.
        voice_id: String,
        /// The `ElevenLabs` model to read with.
        model: String,
    },
}

/// Läser upp ett meddelande med en vald röst (eller flera röster automatiskt).
#[poise::command(
    slash_command,
    rename = "säg",
    install_context = "User",
    interaction_context = "Guild|BotDm|PrivateChannel",
    check = "require_guild_member"
)]
pub async fn say(
    ctx: Context<'_>,
    #[rename = "röst"]
    #[description = "Rösten att läsa upp med (eller Automatiskt för flera röster)"]
    #[autocomplete = autocomplete_reading_voice]
    voice: String,
    #[rename = "meddelande"]
    #[description = "Texten som ska läsas upp"]
    message: String,
) -> AppResult {
    if !is_speakable(&message) {
        ctx.say_ephemeral("Du gav mig ju ingenting att läsa upp.")
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    }

    let db = &ctx.data().db;
    let settings = db.tts_settings().await;
    let Some(reading) = resolve_reading(&settings, &voice) else {
        ctx.say_ephemeral(phrases::no_voice(&voice))
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };

    ctx.defer().await.context(SendResponseSnafu)?;

    let manager = TtsManager::new(settings.clone());
    let original = message.clone();
    let synthesized = match reading {
        Reading::Solo { voice_id, model } => {
            read_aloud::synthesize_single(db, &settings, &manager, &voice_id, &model, message)
                .await?
        }
        Reading::Auto => {
            let fallback = read_aloud::auto_fallback(&settings, None).ok_or(TtsError::NoVoice)?;
            read_aloud::synthesize_auto(db, &settings, &manager, &fallback, None, message).await?
        }
    };

    let attachment = CreateAttachment::bytes(
        synthesized.audio,
        audio_filename(&voice, &read_aloud::requested_now()),
    );
    let mut reply = CreateReply::default().attachment(attachment);
    if let Some(tagged) = tagged_spoiler(&synthesized.spoken, &original) {
        reply = reply.content(tagged);
    }
    ctx.send(reply).await.context(SendMessageSnafu)?;
    Ok(())
}

/// The most characters Discord accepts in a message's content, in characters.
const CONTENT_LIMIT: usize = 2000;

/// The Discord marker opening and closing a spoiler (`||text||`).
const SPOILER_MARKER: &str = "||";

/// Wraps the tag-enriched `spoken` text in a Discord spoiler to show beside the
/// audio, so the user can see what the tag model added to the synthesis request;
/// `None` when `spoken` is unchanged from the original `text`, so nothing was
/// added and the reply is only the audio. The spoilered text is cut to fit
/// Discord's content limit, counting the markers against it.
fn tagged_spoiler(spoken: &str, text: &str) -> Option<String> {
    if spoken == text {
        return None;
    }
    let markers = SPOILER_MARKER.len().saturating_mul(2);
    let room = CONTENT_LIMIT.saturating_sub(markers);
    Some(format!(
        "{SPOILER_MARKER}{}{SPOILER_MARKER}",
        ellipsize(spoken, room)
    ))
}

/// Resolves the chosen voice into a [`Reading`]: the auto label or sentinel into
/// [`Reading::Auto`], a palette voice name (case-insensitive) into a
/// [`Reading::Solo`] with its solo model, and anything else into `None`.
fn resolve_reading(settings: &TtsSettings, choice: &str) -> Option<Reading> {
    let trimmed = choice.trim();
    if trimmed.eq_ignore_ascii_case(read_aloud::AUTO_LABEL)
        || trimmed.eq_ignore_ascii_case(read_aloud::AUTO_VALUE)
    {
        return Some(Reading::Auto);
    }
    let lowered = trimmed.to_lowercase();
    let voice = settings
        .voices()
        .iter()
        .find(|voice| voice.name.to_lowercase() == lowered)?;
    Some(Reading::Solo {
        voice_id: voice.voice_id.clone(),
        model: settings.solo_model(&voice.voice_id).to_owned(),
    })
}

/// The voice names offered in the autocomplete: the auto label first, then every
/// palette voice, each kept only when it contains `partial` (case-insensitive).
fn reading_candidates(settings: &TtsSettings, partial: &str) -> Vec<String> {
    let lowered = partial.to_lowercase();
    let mut names = Vec::new();
    if read_aloud::AUTO_LABEL.to_lowercase().contains(&lowered) {
        names.push(read_aloud::AUTO_LABEL.to_owned());
    }
    for voice in settings.voices() {
        if voice.name.to_lowercase().contains(&lowered) {
            names.push(voice.name.clone());
        }
    }
    names
}

/// Autocompletes the voice argument with the auto label and the palette voices.
async fn autocomplete_reading_voice<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    let settings = ctx.data().db.tts_settings().await;
    autocomplete_from_names(reading_candidates(&settings, partial))
}

/// Whether the user is a member of at least one of the bot's guilds, given the
/// outcome of looking them up in each guild. A successful lookup grants access;
/// a failed lookup (the user is absent from that guild, or the lookup errored)
/// counts as not a member there. A user the bot shares no guild with is denied,
/// as is the empty case where the bot is in no guilds, so the gate fails closed.
fn is_member_of_any<MemberError>(
    lookups: impl IntoIterator<Item = Result<(), MemberError>>,
) -> bool {
    lookups.into_iter().any(|lookup| lookup.is_ok())
}

/// The poise check gating `/säg`. The command is user-installable, so Discord
/// offers it in servers, DMs, and group chats where the bot is not present; this
/// allows it only for users who share a guild with the bot, refusing everyone
/// else with an ephemeral notice (a failed check is otherwise silent).
async fn require_guild_member(ctx: Context<'_>) -> AppResult<bool> {
    let user = ctx.author().id;
    let serenity_ctx = ctx.serenity_context();
    let mut lookups = Vec::new();
    for guild in ctx.cache().guilds() {
        lookups.push(guild.member(serenity_ctx, user).await.map(|_member| ()));
    }
    if is_member_of_any(lookups) {
        return Ok(true);
    }
    ctx.say_ephemeral("Det här kommandot är bara för Harrys eget folk. Ut med dig.")
        .await
        .context(SendMessageSnafu)?;
    Ok(false)
}

/// Tests for the voice-resolution and autocomplete helpers.
#[cfg(test)]
mod tests {
    use super::{Reading, is_member_of_any, reading_candidates, resolve_reading, tagged_spoiler};
    use crate::tts::{TtsSettings, VoiceEntry};

    /// The spoiler is shown, wrapped in Discord's markers, only when the spoken
    /// text differs from the original, so a reply that got no tags stays audio-only.
    #[test]
    fn tagged_spoiler_wraps_only_added_tags() {
        assert_eq!(
            tagged_spoiler("[angry] Hej", "Hej").as_deref(),
            Some("||[angry] Hej||"),
            "tagged text is shown inside the spoiler markers"
        );
        assert_eq!(
            tagged_spoiler("Hej", "Hej"),
            None,
            "unchanged text is not shown at all"
        );
    }

    /// A long tagged text is cut so the spoiler, markers included, fits Discord's
    /// content limit.
    #[test]
    fn tagged_spoiler_fits_the_content_limit() {
        let long = "a".repeat(3000);
        let spoiled = tagged_spoiler(&long, "short");
        assert!(spoiled.is_some(), "changed text is still shown");
        let Some(spoilered) = spoiled else {
            return;
        };
        assert!(
            spoilered.starts_with("||") && spoilered.ends_with("…||"),
            "the markers survive the truncation, which ends in an ellipsis"
        );
        assert_eq!(
            spoilered.chars().count(),
            2000,
            "the text fills exactly the content limit, markers included"
        );
    }

    /// A user found in at least one guild is granted access, including when the
    /// successful lookup is not the first one checked.
    #[test]
    fn is_member_of_any_grants_on_any_success() {
        let single: [Result<(), ()>; 1] = [Ok(())];
        assert!(
            is_member_of_any(single),
            "a single successful lookup grants access"
        );
        let later: [Result<(), ()>; 2] = [Err(()), Ok(())];
        assert!(
            is_member_of_any(later),
            "a success after an earlier failure still grants access"
        );
    }

    /// A user found in none of the guilds, and the empty case where the bot is in
    /// no guilds, are both denied so the gate fails closed.
    #[test]
    fn is_member_of_any_denies_without_success() {
        let none: [Result<(), ()>; 2] = [Err(()), Err(())];
        assert!(!is_member_of_any(none), "all lookups failing denies access");
        let empty: [Result<(), ()>; 0] = [];
        assert!(
            !is_member_of_any(empty),
            "no guilds to be a member of denies access"
        );
    }

    /// Builds a palette voice entry with the given name, ID, and optional solo model.
    fn voice(name: &str, voice_id: &str, model: Option<&str>) -> VoiceEntry {
        VoiceEntry {
            name: name.to_owned(),
            voice_id: voice_id.to_owned(),
            emoji: "🎙️".to_owned(),
            description: "a test voice".to_owned(),
            model: model.map(str::to_owned),
        }
    }

    /// Builds settings carrying the given palette voices over the defaults.
    fn settings(voices: Vec<VoiceEntry>) -> TtsSettings {
        let mut settings = TtsSettings::default();
        for entry in voices {
            settings.add_voice(entry);
        }
        settings
    }

    /// The auto label and the auto sentinel both resolve to the auto reading,
    /// case-insensitively.
    #[test]
    fn resolve_reading_recognizes_auto() {
        let configured = settings(vec![]);
        assert_eq!(
            resolve_reading(&configured, "Automatiskt"),
            Some(Reading::Auto),
            "the Swedish auto label resolves to the auto reading"
        );
        assert_eq!(
            resolve_reading(&configured, "automatiskt"),
            Some(Reading::Auto),
            "the auto label is matched case-insensitively"
        );
        assert_eq!(
            resolve_reading(&configured, "auto"),
            Some(Reading::Auto),
            "the auto sentinel resolves to the auto reading"
        );
    }

    /// A palette voice resolves by name (case-insensitively) to its voice ID and
    /// its solo model, falling back to the configured model without an override.
    #[test]
    fn resolve_reading_finds_palette_voice() {
        let configured = settings(vec![
            voice("Adam", "adam-id", Some("eleven_multilingual_v2")),
            voice("Eva", "eva-id", None),
        ]);
        assert_eq!(
            resolve_reading(&configured, "adam"),
            Some(Reading::Solo {
                voice_id: "adam-id".to_owned(),
                model: "eleven_multilingual_v2".to_owned(),
            }),
            "a pinned voice resolves to its own solo model"
        );
        assert_eq!(
            resolve_reading(&configured, "Eva"),
            Some(Reading::Solo {
                voice_id: "eva-id".to_owned(),
                model: configured.model.clone(),
            }),
            "a voice without an override falls back to the configured model"
        );
    }

    /// An unknown or empty voice name resolves to nothing, so the command can
    /// report that no such voice exists.
    #[test]
    fn resolve_reading_rejects_unknown_voice() {
        let configured = settings(vec![voice("Adam", "adam-id", None)]);
        assert_eq!(
            resolve_reading(&configured, "Bertil"),
            None,
            "an unknown name resolves to nothing"
        );
        assert_eq!(
            resolve_reading(&configured, ""),
            None,
            "an empty name resolves to nothing"
        );
    }

    /// The candidates list the auto label first, then every palette voice in order.
    #[test]
    fn reading_candidates_list_auto_first_then_voices() {
        let configured = settings(vec![
            voice("Adam", "adam-id", None),
            voice("Eva", "eva-id", None),
        ]);
        assert_eq!(
            reading_candidates(&configured, ""),
            vec![
                "Automatiskt".to_owned(),
                "Adam".to_owned(),
                "Eva".to_owned()
            ],
            "an empty query lists the auto label first, then the palette"
        );
    }

    /// The candidates are filtered by the partial input, case-insensitively, and
    /// the auto label participates in the filtering.
    #[test]
    fn reading_candidates_filter_by_partial() {
        let configured = settings(vec![
            voice("Adam", "adam-id", None),
            voice("Eva", "eva-id", None),
        ]);
        assert_eq!(
            reading_candidates(&configured, "au"),
            vec!["Automatiskt".to_owned()],
            "a query matching only the auto label lists just it"
        );
        assert_eq!(
            reading_candidates(&configured, "ev"),
            vec!["Eva".to_owned()],
            "a query matching a voice name lists just that voice"
        );
    }
}
