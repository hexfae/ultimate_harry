//! The bot's Discord slash commands.

pub mod character;
pub mod chat;
pub mod emoji;
pub mod model;
pub mod name;
pub mod pin_channel;
pub mod say;
pub mod stats;
pub mod tts;
pub mod voice;

pub use character::character;
pub use chat::chat;
pub use emoji::emoji;
pub use model::model;
pub use name::name;
pub use pin_channel::pin_channel;
pub use say::say;
pub use stats::stats;
pub use tts::tts;
pub use voice::voice;

use poise::serenity_prelude::{
    AutocompleteChoice, CreateAutocompleteResponse, InstallationContext,
};
use snafu::ResultExt as _;
use tokio::time::sleep;
use tracing::warn;

use crate::{
    AppResult, ApplicationContext, Context,
    app_state::AppState,
    constants::TRANSIENT_LINGER,
    database::DatabaseError,
    error::{AppError, DeleteMessageSnafu, SendMessageSnafu},
    llm::{matching_model_ids, model_catalog},
    models::character::Character,
    phrases::no_character,
    shortcodes::strip_custom_emoji,
    traits::SayEphemeral as _,
    tts::{TtsManager, filter_voices},
    util::report_error,
};

/// Returns the full list of the bot's slash commands.
///
/// Declared once here so framework setup and per-guild registration cannot drift apart.
pub fn commands() -> Vec<poise::Command<AppState, AppError>> {
    vec![
        character(),
        chat(),
        emoji(),
        model(),
        pin_channel(),
        name(),
        say(),
        stats(),
        tts(),
        voice(),
    ]
}

/// A command list split for registration: the globally registered commands and
/// the guild-scoped commands.
type CommandSplit = (
    Vec<poise::Command<AppState, AppError>>,
    Vec<poise::Command<AppState, AppError>>,
);

/// Splits [`commands`] into the user-installable commands, which must be
/// registered globally so they work in servers and DMs where the bot is absent,
/// and the rest, which are registered per guild.
pub fn partitioned_commands() -> CommandSplit {
    commands().into_iter().partition(is_user_installable)
}

/// Whether a command is user-installable, i.e. its install context includes
/// [`InstallationContext::User`].
fn is_user_installable(command: &poise::Command<AppState, AppError>) -> bool {
    command
        .install_context
        .as_ref()
        .is_some_and(|contexts| contexts.contains(&InstallationContext::User))
}

/// Sends the shared "no character found" notice ephemerally, then deletes it
/// after a short linger, so every command that finds no character responds the
/// same transient way.
pub async fn notify_no_character(ctx: Context<'_>) -> AppResult {
    let message = ctx
        .say_ephemeral(no_character())
        .await
        .context(SendMessageSnafu)?;
    sleep(TRANSIENT_LINGER).await;
    message.delete(ctx).await.context(DeleteMessageSnafu)?;
    Ok(())
}

/// Returns the visible character most similar to `name`, or sends the shared
/// "no character found" notice and returns `None` when the search finds nothing.
/// The single search-then-pick-first-or-notify policy shared by the commands that
/// act on one named character.
pub async fn first_character_or_notify(
    ctx: Context<'_>,
    name: String,
) -> AppResult<Option<Character>> {
    let Some(character) = ctx
        .data()
        .db
        .characters_by_similarity(name)
        .await?
        .into_iter()
        .next()
    else {
        notify_no_character(ctx).await?;
        return Ok(None);
    };
    Ok(Some(character))
}

/// Sends `text` as an ephemeral reply, lets it linger briefly, then deletes it,
/// the transient success notice shared by the create and edit commands (the
/// [`ApplicationContext`] sibling of the paginator's `respond_then_clear`).
pub async fn say_transient<T: Into<String>>(ctx: ApplicationContext<'_>, text: T) -> AppResult {
    let message = ctx
        .say_ephemeral(text.into())
        .await
        .context(SendMessageSnafu)?;
    sleep(TRANSIENT_LINGER).await;
    message
        .delete(Context::Application(ctx))
        .await
        .context(DeleteMessageSnafu)?;
    Ok(())
}

/// Builds the plain-text autocomplete label for a character: its name preceded
/// by any unicode emoji, with custom server emoji dropped. Discord renders only
/// unicode emoji in autocomplete choices and would show `<:name:id>` markup as
/// raw text, so we strip it here.
fn autocomplete_label(character: &Character) -> String {
    let Some(emoji) = character.emoji() else {
        return character.name().to_owned();
    };
    let visible = strip_custom_emoji(emoji);
    let trimmed = visible.trim();
    if trimmed.is_empty() {
        character.name().to_owned()
    } else {
        format!("{trimmed} {}", character.name())
    }
}

/// Builds an autocomplete response listing the given characters by name.
fn choices_from<'a>(characters: Vec<Character>) -> CreateAutocompleteResponse<'a> {
    let character_names = characters
        .into_iter()
        .map(|character| {
            AutocompleteChoice::new(autocomplete_label(&character), character.name().to_owned())
        })
        .collect::<Vec<AutocompleteChoice<'_>>>();
    CreateAutocompleteResponse::new().set_choices(character_names)
}

/// Builds an autocomplete response from the given names, using each as both the
/// displayed label and the submitted value.
pub fn autocomplete_from_names<'a>(names: Vec<String>) -> CreateAutocompleteResponse<'a> {
    let choices = names
        .into_iter()
        .map(|name| AutocompleteChoice::new(name.clone(), name))
        .collect::<Vec<AutocompleteChoice<'_>>>();
    CreateAutocompleteResponse::new().set_choices(choices)
}

/// Builds an autocomplete response from a similarity-ranked character query,
/// swallowing a ranking failure into an empty list (logged) so the picker stays
/// responsive. `what` names the queried set for the warning.
fn autocomplete_from<'a>(
    ranked: Result<Vec<Character>, DatabaseError>,
    what: &str,
) -> CreateAutocompleteResponse<'a> {
    let characters = match ranked {
        Ok(characters) => characters,
        Err(why) => {
            warn!("failed to rank {what} for autocomplete, returning none");
            report_error(why);
            Vec::new()
        }
    };
    choices_from(characters)
}

/// Builds a model-id autocomplete response from the `OpenRouter` catalog,
/// optionally restricted to models accepting `modality` as input, swallowing a
/// catalog fetch failure into an empty list (logged) so the picker stays responsive.
async fn autocomplete_models<'a>(
    partial: &str,
    modality: Option<&str>,
) -> CreateAutocompleteResponse<'a> {
    let catalog = match model_catalog().await {
        Ok(catalog) => catalog,
        Err(why) => {
            warn!("failed to fetch the model catalog for autocomplete, returning none");
            report_error(why);
            Vec::new()
        }
    };
    autocomplete_from_names(matching_model_ids(&catalog, partial, modality))
}

/// Returns an autocomplete response of `OpenRouter` model ids matching the input.
pub async fn autocomplete_model<'a>(
    _ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    autocomplete_models(partial, None).await
}

/// Returns an autocomplete response of vision-capable (image input) `OpenRouter`
/// model ids matching the input.
pub async fn autocomplete_vision_model<'a>(
    _ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    autocomplete_models(partial, Some("image")).await
}

/// Returns an autocomplete response of audio-capable `OpenRouter` model ids
/// matching the input.
pub async fn autocomplete_audio_model<'a>(
    _ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    autocomplete_models(partial, Some("audio")).await
}

/// Returns an auto completion response from characters found in the database, sorted by similarity to the input.
pub async fn autocomplete<'a>(ctx: Context<'_>, partial: &str) -> CreateAutocompleteResponse<'a> {
    autocomplete_from(
        ctx.data().db.characters_by_similarity(partial).await,
        "characters",
    )
}

/// Returns an auto completion response from soft-deleted characters, sorted by
/// similarity to the input. Used by the restore command, since deleted
/// characters are hidden from the regular autocomplete.
pub async fn autocomplete_deleted<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    autocomplete_from(
        ctx.data()
            .db
            .deleted_characters_by_similarity(partial)
            .await,
        "deleted characters",
    )
}

/// Autocompletes `ElevenLabs` voices for the röst-id parameters, fetched live
/// from the account's voice list: the choice shows the voice's name and
/// submits its voice ID. A fetch failure (or a missing API key) is swallowed
/// into an empty list (logged) so the picker stays responsive and a raw ID can
/// still be pasted.
pub async fn autocomplete_elevenlabs_voice<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    let settings = ctx.data().db.tts_settings().await;
    let voices = match TtsManager::new(settings).list_voices().await {
        Ok(voices) => voices,
        Err(why) => {
            warn!("failed to list ElevenLabs voices for autocomplete, returning none");
            report_error(why);
            Vec::new()
        }
    };
    let choices = filter_voices(voices, partial)
        .into_iter()
        .map(|voice| AutocompleteChoice::new(voice.name, voice.voice_id))
        .collect::<Vec<AutocompleteChoice<'_>>>();
    CreateAutocompleteResponse::new().set_choices(choices)
}

/// Tests for the registration split.
#[cfg(test)]
mod tests {
    use super::{is_user_installable, partitioned_commands};
    use crate::app_state::AppState;
    use crate::commands::{character, model, say, tts};
    use crate::error::AppError;

    /// Whether `command` has a parameter named `parameter` with an autocomplete callback.
    fn has_autocomplete(command: &poise::Command<AppState, AppError>, parameter: &str) -> bool {
        command
            .parameters
            .iter()
            .find(|param| param.name == parameter)
            .is_some_and(|param| param.autocomplete_callback.is_some())
    }

    /// Every `OpenRouter` model-id parameter offers model autocomplete.
    #[test]
    fn model_id_parameters_offer_autocomplete() {
        let global = model();
        assert!(
            has_autocomplete(&global, "modell"),
            "/modell's modell parameter offers model autocomplete"
        );
        assert!(
            has_autocomplete(&global, "syn-modell"),
            "/modell's syn-modell parameter offers model autocomplete"
        );
        assert!(
            has_autocomplete(&global, "ljud-modell"),
            "/modell's ljud-modell parameter offers model autocomplete"
        );
        assert!(
            has_autocomplete(&tts(), "tagg-modell"),
            "/tal's tagg-modell parameter offers model autocomplete"
        );
        let character_model = character()
            .subcommands
            .into_iter()
            .find(|subcommand| subcommand.name == "modell");
        assert!(
            character_model.is_some_and(|subcommand| has_autocomplete(&subcommand, "modell")),
            "/gubbe modell's modell parameter offers model autocomplete"
        );
    }

    /// `/säg` is user-installable so it can be invoked in other servers and DMs.
    #[test]
    fn say_is_user_installable() {
        assert!(
            is_user_installable(&say()),
            "/säg must be user-installable to work outside the bot's guild"
        );
    }

    /// A guild-only command like `/model` is not user-installable.
    #[test]
    fn other_commands_are_not_user_installable() {
        assert!(
            !is_user_installable(&model()),
            "/model is guild-only and must not be user-installable"
        );
    }

    /// Only `/säg` registers globally; every other command stays guild-scoped.
    #[test]
    fn only_say_registers_globally() {
        let (global, guild_scoped) = partitioned_commands();
        assert_eq!(
            global.len(),
            1,
            "exactly one command, /säg, is registered globally"
        );
        assert_eq!(
            global.first().map(|command| command.name.as_ref()),
            Some("säg"),
            "the single global command is /säg"
        );
        assert!(
            guild_scoped
                .iter()
                .all(|command| command.name.as_ref() != "säg"),
            "/säg must not also be registered per guild"
        );
    }
}
