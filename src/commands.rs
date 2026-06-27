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

use poise::serenity_prelude::{AutocompleteChoice, CreateAutocompleteResponse, InstallationContext};
use snafu::ResultExt as _;
use tokio::time::sleep;
use tracing::warn;

use crate::{
    AppResult, Context,
    app_state::AppState,
    constants::TRANSIENT_LINGER,
    database::DatabaseError,
    error::{AppError, DeleteMessageSnafu, SendMessageSnafu},
    models::character::Character,
    phrases::no_character,
    shortcodes::strip_custom_emoji,
    traits::SayEphemeral as _,
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

/// Returns an auto completion response from characters found in the database, sorted by similarity to the input.
pub async fn autocomplete<'a>(ctx: Context<'_>, partial: &str) -> CreateAutocompleteResponse<'a> {
    autocomplete_from(ctx.data().db.characters_by_similarity(partial).await, "characters")
}

/// Returns an auto completion response from soft-deleted characters, sorted by
/// similarity to the input. Used by the restore command, since deleted
/// characters are hidden from the regular autocomplete.
pub async fn autocomplete_deleted<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    autocomplete_from(
        ctx.data().db.deleted_characters_by_similarity(partial).await,
        "deleted characters",
    )
}

/// Tests for the registration split.
#[cfg(test)]
mod tests {
    use super::{is_user_installable, partitioned_commands};
    use crate::commands::{model, say};

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
