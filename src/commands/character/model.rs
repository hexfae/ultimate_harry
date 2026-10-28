//! The bot's Discord slash command for setting various AI model settings for a specific character.

use crate::{
    AppResult, Context,
    commands::{
        autocomplete, autocomplete_character_provider, autocomplete_model,
        first_character_or_notify,
    },
    error::SendMessageSnafu,
    llm::{CharacterModelSettings, model_endpoints, pin_for_model},
    phrases,
    traits::SayEphemeral as _,
    util::unless_cleared,
};
use snafu::ResultExt as _;

/// Ställer in en gubbes AI-modellinställningar.
///
/// Only the model, its provider pin, and the temperature can be overridden per
/// character; the API key and the vision/audio fallback models always come from
/// the global `/modell` settings.
#[poise::command(slash_command, rename = "modell")]
pub async fn model(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
    #[rename = "modell"]
    #[description = "Modellen att använda (ingen = den globala)"]
    #[autocomplete = autocomplete_model]
    model: Option<String>,
    #[rename = "leverantör"]
    #[description = "Leverantören att låsa modellen till (ingen = ingen låsning)"]
    #[autocomplete = autocomplete_character_provider]
    provider: Option<String>,
    #[rename = "temperatur"]
    #[description = "Temperaturen (högre = mer slumpmässig)"]
    temperature: Option<f32>,
    #[rename = "återställ"]
    #[description = "Ta bort gubbens egna inställningar så att de globala gäller"]
    reset: Option<bool>,
) -> AppResult {
    let db = &ctx.data().db;
    let Some(character) = first_character_or_notify(ctx, name).await? else {
        return Ok(());
    };

    if reset.unwrap_or_default() {
        db.set_character_model_settings(character.id(), CharacterModelSettings::default())
            .await?;
        ctx.say_ephemeral(phrases::done())
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    }

    if model.is_none() && temperature.is_none() && provider.is_none() {
        let model_settings = db.resolved_model_settings(&character).await;
        let scope = if character.has_model_settings() {
            "egna inställningar"
        } else {
            "ärvda globala inställningar"
        };
        ctx.say_ephemeral(format!("{}\n*({scope})*", model_settings.summary()))
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    }

    let switched_model = model.is_some();
    let mut overrides = with_choices(
        character.model_settings().cloned().unwrap_or_default(),
        model,
        provider,
        temperature,
    );
    // a provider slug names one endpoint of one model, so switching models
    // leaves a pin the new model does not serve behind, failing every request
    if switched_model {
        let new_model = match overrides.model.clone() {
            Some(own) => own,
            None => db.model_settings().await.model,
        };
        let endpoints = model_endpoints(&new_model).await;
        overrides.provider =
            pin_for_model(overrides.provider.as_deref(), &endpoints).map(str::to_owned);
    }
    db.set_character_model_settings(character.id(), overrides)
        .await?;
    ctx.say_ephemeral(phrases::done())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}

/// Applies the supplied `/gubbe modell` options to a character's `overrides`,
/// leaving the omitted ones untouched. The clear keyword removes the model
/// override (falling back to the global model) or the provider pin.
fn with_choices(
    mut overrides: CharacterModelSettings,
    model: Option<String>,
    provider: Option<String>,
    temperature: Option<f32>,
) -> CharacterModelSettings {
    if let Some(chosen) = model {
        overrides.model = unless_cleared(chosen);
    }
    if let Some(chosen) = provider {
        overrides.provider = unless_cleared(chosen);
    }
    overrides.temperature = temperature.or(overrides.temperature);
    overrides
}

/// Tests for applying the `/gubbe modell` options.
#[cfg(test)]
mod tests {
    use super::with_choices;
    use crate::{llm::CharacterModelSettings, util::CLEAR_KEYWORD};

    /// The character's current overrides, with every field set.
    fn current() -> CharacterModelSettings {
        CharacterModelSettings {
            model: Some("own/model".to_owned()),
            provider: Some("own/fp8".to_owned()),
            temperature: Some(0.5),
        }
    }

    /// Omitted options leave the current overrides as they are.
    #[test]
    fn omitted_options_keep_the_current_overrides() {
        let kept = with_choices(current(), None, None, None);
        assert_eq!(
            kept.model.as_deref(),
            Some("own/model"),
            "the model is kept"
        );
        assert_eq!(kept.provider.as_deref(), Some("own/fp8"), "the pin is kept");
        assert_eq!(
            kept.temperature.map(f32::to_bits),
            Some(0.5_f32.to_bits()),
            "the temperature is kept"
        );
    }

    /// Supplied options replace the current overrides.
    #[test]
    fn supplied_options_replace_the_current_overrides() {
        let replaced = with_choices(
            current(),
            Some("new/model".to_owned()),
            Some("new/fp8".to_owned()),
            Some(1.5),
        );
        assert_eq!(
            replaced.model.as_deref(),
            Some("new/model"),
            "the model is replaced"
        );
        assert_eq!(
            replaced.provider.as_deref(),
            Some("new/fp8"),
            "the pin is replaced"
        );
        assert_eq!(
            replaced.temperature.map(f32::to_bits),
            Some(1.5_f32.to_bits()),
            "the temperature is replaced"
        );
    }

    /// The clear keyword removes the model override and the pin.
    #[test]
    fn the_clear_keyword_removes_the_model_and_the_pin() {
        let cleared = with_choices(
            current(),
            Some(CLEAR_KEYWORD.to_owned()),
            Some(CLEAR_KEYWORD.to_owned()),
            None,
        );
        assert_eq!(
            cleared.model, None,
            "the model falls back to the global one"
        );
        assert_eq!(cleared.provider, None, "the pin is removed");
    }
}
