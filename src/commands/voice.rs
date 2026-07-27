//! The bot's Discord slash commands for managing the speak-aloud voice palette.

use crate::{
    AppResult, Context, commands::autocomplete_from_names, error::SendMessageSnafu, phrases,
    traits::SayEphemeral as _, tts::VoiceEntry,
};
use poise::serenity_prelude::CreateAutocompleteResponse;
use snafu::ResultExt as _;

/// Hanterar röster för uppläsning.
#[poise::command(
    slash_command,
    subcommands("create", "delete", "view"),
    subcommand_required,
    rename = "röst"
)]
#[expect(clippy::unused_async, reason = "poise requires commands to be async")]
pub async fn voice(_: Context<'_>) -> AppResult {
    Ok(())
}

/// Skapar en röst i paletten.
#[poise::command(slash_command, rename = "skapa")]
pub async fn create(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Namnet som visas i menyn"]
    name: String,
    #[rename = "röst-id"]
    #[description = "ElevenLabs röst-ID"]
    voice_id: String,
    #[rename = "emoji"]
    #[description = "Emoji som visas bredvid namnet"]
    emoji: String,
    #[rename = "beskrivning"]
    #[description = "Kort beskrivning som styr automatiskt röstval"]
    description: String,
    #[rename = "modell"]
    #[description = "ElevenLabs-modell vid enskild uppläsning (t.ex. eleven_multilingual_v2)"]
    model: Option<String>,
) -> AppResult {
    let db = &ctx.data().db;
    let mut settings = db.tts_settings().await;
    settings.add_voice(VoiceEntry {
        name,
        voice_id,
        emoji,
        description,
        model: model.filter(|value| !value.is_empty()),
    });
    db.upsert_tts_settings(settings).await?;
    ctx.say_ephemeral(phrases::done())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}

/// Dödar en röst i paletten.
#[poise::command(slash_command, rename = "döda")]
pub async fn delete(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Namnet på rösten att döda"]
    #[autocomplete = autocomplete_voice]
    name: String,
) -> AppResult {
    let db = &ctx.data().db;
    let mut settings = db.tts_settings().await;
    if settings.remove_voice(&name) {
        db.upsert_tts_settings(settings).await?;
        ctx.say_ephemeral(phrases::done())
            .await
            .context(SendMessageSnafu)?;
    } else {
        ctx.say_ephemeral(phrases::no_voice(&name))
            .await
            .context(SendMessageSnafu)?;
    }
    Ok(())
}

/// Visar rösterna i paletten.
#[poise::command(slash_command, rename = "visa")]
pub async fn view(ctx: Context<'_>) -> AppResult {
    let settings = ctx.data().db.tts_settings().await;
    let message = if settings.voices().is_empty() {
        "Det finns inga röster än. Tyst som i graven.".to_owned()
    } else {
        settings
            .voices()
            .iter()
            .map(|voice| {
                let model = voice
                    .model
                    .as_deref()
                    .map(|model| format!(" [{model}]"))
                    .unwrap_or_default();
                format!(
                    "{} {} - {} ({}){model}",
                    voice.emoji, voice.name, voice.description, voice.voice_id
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    ctx.say_ephemeral(message).await.context(SendMessageSnafu)?;
    Ok(())
}

/// Autocompletes palette voice names, sorted by their match against the input.
async fn autocomplete_voice<'a>(ctx: Context<'_>, partial: &str) -> CreateAutocompleteResponse<'a> {
    let settings = ctx.data().db.tts_settings().await;
    let lowered = partial.to_lowercase();
    let names = settings
        .voices()
        .iter()
        .filter(|voice| voice.name.to_lowercase().contains(&lowered))
        .map(|voice| voice.name.clone())
        .collect::<Vec<String>>();
    autocomplete_from_names(names)
}
