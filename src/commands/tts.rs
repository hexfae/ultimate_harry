//! The bot's Discord slash command for setting text-to-speech (`ElevenLabs`) settings.

use crate::{
    AppResult, Context, error::SendMessageSnafu, traits::SayEphemeral as _, tts::TtsOverrides,
};
use snafu::ResultExt as _;

/// Ställer in inställningar för uppläsning (text-till-tal).
#[poise::command(slash_command, rename = "tal")]
pub async fn tts(
    ctx: Context<'_>,
    #[rename = "api-nyckel"]
    #[description = "ElevenLabs API-nyckeln att använda"]
    api_key: Option<String>,
    #[rename = "standardröst"]
    #[description = "Röst-ID:t för karaktärer utan egen röst"]
    default_voice: Option<String>,
    #[rename = "röst-modell"]
    #[description = "ElevenLabs-modellen att använda"]
    model: Option<String>,
) -> AppResult {
    let overrides = TtsOverrides {
        api_key,
        default_voice,
        model,
    };
    let mut tts_settings = ctx.data().db.tts_settings().await;
    if overrides.is_empty() {
        ctx.say_ephemeral(tts_settings.summary())
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    }
    tts_settings.apply_overrides(overrides);
    ctx.data().db.upsert_tts_settings(tts_settings).await?;
    ctx.say_ephemeral("Klart!").await.context(SendMessageSnafu)?;
    Ok(())
}
