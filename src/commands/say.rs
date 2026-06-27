//! The bot's Discord slash command for reading a message aloud in a chosen voice.

use crate::{
    AppResult, Context,
    error::{SendMessageSnafu, SendResponseSnafu},
    read_aloud,
    traits::SayEphemeral as _,
    tts::{TtsError, TtsManager, TtsSettings, audio_filename, is_speakable},
};
use poise::{
    CreateReply,
    serenity_prelude::{AutocompleteChoice, CreateAttachment, CreateAutocompleteResponse},
};
use snafu::ResultExt as _;

/// The autocomplete label that requests auto voice assignment.
const AUTO_LABEL: &str = "Automatiskt";

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
        ctx.say_ephemeral("Det finns inget att läsa upp.")
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    }

    let db = &ctx.data().db;
    let settings = db.tts_settings().await;
    let Some(reading) = resolve_reading(&settings, &voice) else {
        ctx.say_ephemeral(format!("Ingen röst med namnet {voice} hittades."))
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };

    ctx.defer().await.context(SendResponseSnafu)?;

    let manager = TtsManager::new(settings.clone());
    let audio = match reading {
        Reading::Solo { voice_id, model } => {
            read_aloud::synthesize_single(db, &settings, &manager, &voice_id, &model, message).await?
        }
        Reading::Auto => {
            let fallback = auto_fallback(&settings).ok_or(TtsError::NoVoice)?;
            read_aloud::synthesize_auto(db, &settings, &manager, &fallback, None, message).await?
        }
    };

    let attachment =
        CreateAttachment::bytes(audio, audio_filename(&voice, &read_aloud::requested_now()));
    ctx.send(CreateReply::default().attachment(attachment))
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}

/// Resolves the chosen voice into a [`Reading`]: the auto label or sentinel into
/// [`Reading::Auto`], a palette voice name (case-insensitive) into a
/// [`Reading::Solo`] with its solo model, and anything else into `None`.
fn resolve_reading(settings: &TtsSettings, choice: &str) -> Option<Reading> {
    let trimmed = choice.trim();
    if trimmed.eq_ignore_ascii_case(AUTO_LABEL)
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
    if AUTO_LABEL.to_lowercase().contains(&lowered) {
        names.push(AUTO_LABEL.to_owned());
    }
    for voice in settings.voices() {
        if voice.name.to_lowercase().contains(&lowered) {
            names.push(voice.name.clone());
        }
    }
    names
}

/// The fallback voice for auto reading: the first palette voice, then the generic
/// default, and nothing when neither is set.
fn auto_fallback(settings: &TtsSettings) -> Option<String> {
    settings
        .voices()
        .first()
        .map(|voice| voice.voice_id.clone())
        .or_else(|| settings.default_voice.clone())
}

/// Autocompletes the voice argument with the auto label and the palette voices.
async fn autocomplete_reading_voice<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    let settings = ctx.data().db.tts_settings().await;
    let choices = reading_candidates(&settings, partial)
        .into_iter()
        .map(|name| AutocompleteChoice::new(name.clone(), name))
        .collect::<Vec<AutocompleteChoice<'_>>>();
    CreateAutocompleteResponse::new().set_choices(choices)
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
    ctx.say_ephemeral("Du måste vara med i Harrys server för att använda detta kommando.")
        .await
        .context(SendMessageSnafu)?;
    Ok(false)
}

/// Tests for the voice-resolution and autocomplete helpers.
#[cfg(test)]
mod tests {
    use super::{Reading, auto_fallback, is_member_of_any, reading_candidates, resolve_reading};
    use crate::tts::{TtsSettings, VoiceEntry};

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
        assert!(
            !is_member_of_any(none),
            "all lookups failing denies access"
        );
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
        let configured = settings(vec![voice("Adam", "adam-id", None), voice("Eva", "eva-id", None)]);
        assert_eq!(
            reading_candidates(&configured, ""),
            vec!["Automatiskt".to_owned(), "Adam".to_owned(), "Eva".to_owned()],
            "an empty query lists the auto label first, then the palette"
        );
    }

    /// The candidates are filtered by the partial input, case-insensitively, and
    /// the auto label participates in the filtering.
    #[test]
    fn reading_candidates_filter_by_partial() {
        let configured = settings(vec![voice("Adam", "adam-id", None), voice("Eva", "eva-id", None)]);
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

    /// The auto fallback prefers the first palette voice, then the generic default,
    /// and is nothing when neither exists.
    #[test]
    fn auto_fallback_prefers_palette_then_default() {
        let with_palette = settings(vec![voice("Adam", "adam-id", None)]);
        assert_eq!(
            auto_fallback(&with_palette).as_deref(),
            Some("adam-id"),
            "the first palette voice is the preferred fallback"
        );

        let only_default = TtsSettings {
            default_voice: Some("default-id".to_owned()),
            ..TtsSettings::default()
        };
        assert_eq!(
            auto_fallback(&only_default).as_deref(),
            Some("default-id"),
            "the generic default is used when the palette is empty"
        );

        assert_eq!(
            auto_fallback(&TtsSettings::default()),
            None,
            "no palette and no default leaves nothing to fall back to"
        );
    }
}
