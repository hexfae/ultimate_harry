//! The bot's Discord slash commands.

pub mod character;
pub mod chat;
pub mod emoji;
pub mod model;
pub mod name;
pub mod pin_channel;
pub mod stats;
pub mod tts;

pub use character::character;
pub use chat::chat;
pub use emoji::emoji;
pub use model::model;
pub use name::name;
pub use pin_channel::pin_channel;
pub use stats::stats;
pub use tts::tts;

use poise::serenity_prelude::{AutocompleteChoice, CreateAutocompleteResponse};
use snafu::ResultExt as _;
use tokio::time::sleep;
use tracing::warn;

use crate::{
    AppResult, Context,
    app_state::AppState,
    constants::TRANSIENT_LINGER,
    error::{AppError, DeleteMessageSnafu, SendMessageSnafu},
    models::character::Character,
    phrases::no_character,
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
        stats(),
        tts(),
    ]
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

/// Builds an autocomplete response listing the given characters by name.
// TODO: having written e.g. ":microphone:" displays that text, not the emoji
fn choices_from<'a>(characters: Vec<Character>) -> CreateAutocompleteResponse<'a> {
    let character_names = characters
        .into_iter()
        .map(|character| {
            AutocompleteChoice::new(character.to_string(), character.name().to_owned())
        })
        .collect::<Vec<AutocompleteChoice<'_>>>();
    CreateAutocompleteResponse::new().set_choices(character_names)
}

/// Returns an auto completion response from characters found in the database, sorted by similarity to the input.
pub async fn autocomplete<'a>(ctx: Context<'_>, partial: &str) -> CreateAutocompleteResponse<'a> {
    let characters = match ctx.data().db.characters_by_similarity(partial).await {
        Ok(characters) => characters,
        Err(why) => {
            warn!("failed to rank characters for autocomplete, returning none");
            report_error(why);
            Vec::new()
        }
    };
    choices_from(characters)
}

/// Returns an auto completion response from soft-deleted characters, sorted by
/// similarity to the input. Used by the restore command, since deleted
/// characters are hidden from the regular autocomplete.
pub async fn autocomplete_deleted<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    let characters = match ctx.data().db.deleted_characters_by_similarity(partial).await {
        Ok(characters) => characters,
        Err(why) => {
            warn!("failed to rank deleted characters for autocomplete, returning none");
            report_error(why);
            Vec::new()
        }
    };
    choices_from(characters)
}
