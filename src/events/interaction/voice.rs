//! The dropdown that speaks a reply aloud, either in a chosen palette voice or
//! with voices auto-assigned per speaker via `ElevenLabs` text-to-dialogue.

use super::{UnknownInteraction, speak};
use crate::{
    AppResult,
    database::Database,
    llm::VoiceChoice,
    models::{character::Character, history::History},
    read_aloud,
    tts::{TtsError, TtsSettings},
};
use serenity::all::{ComponentInteraction, ComponentInteractionDataKind, Context};

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

    let requested_at = read_aloud::requested_now();
    let (settings, manager, text) = speak::setup(db, &history).await;
    ensure_offered(&settings, selection, &interaction.data.custom_id)?;
    let fallback =
        read_aloud::auto_fallback(&settings, Some(&character)).ok_or(TtsError::NoVoice)?;

    speak::defer(ctx, interaction).await?;

    let audio = if selection == read_aloud::AUTO_VALUE {
        let character_voice = VoiceChoice {
            voice_id: fallback.clone(),
            name: character.name().to_owned(),
            description: format!("the main character {} speaking", character.name()),
        };
        read_aloud::synthesize_auto(
            db,
            &settings,
            &manager,
            &fallback,
            Some(character_voice),
            text,
        )
        .await?
    } else {
        let model = settings.solo_model(selection).to_owned();
        read_aloud::synthesize_single(db, &settings, &manager, selection, &model, text).await?
    };

    speak::post_followup(ctx, interaction, audio.audio, &character, &requested_at).await
}

/// Refuses a dropdown `choice` the palette no longer offers as a stale button.
///
/// The choice is spliced into the synthesis URL, so only the auto entry or a
/// voice still in the palette is accepted; anything else is a dropdown from
/// before that voice was removed (or a forged value).
fn ensure_offered(
    settings: &TtsSettings,
    choice: &str,
    custom_id: &str,
) -> Result<(), UnknownInteraction> {
    if read_aloud::is_offered(settings, choice) {
        Ok(())
    } else {
        Err(UnknownInteraction::stale(custom_id))
    }
}

/// Tests for the dropdown choice check.
#[cfg(test)]
mod tests {
    use super::ensure_offered;
    use crate::{read_aloud::AUTO_VALUE, tts::TtsSettings};

    /// The auto entry passes, while a voice the palette does not offer is
    /// refused as stale.
    #[test]
    fn ensure_offered_refuses_a_voice_outside_the_palette() {
        let settings = TtsSettings::default();
        assert!(
            ensure_offered(&settings, AUTO_VALUE, "1voic").is_ok(),
            "the auto entry is always offered"
        );
        assert!(
            ensure_offered(&settings, "removed-id", "1voic").is_err(),
            "a voice outside the palette is refused"
        );
    }
}
