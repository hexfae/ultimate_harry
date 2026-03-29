//! The bot's Discord slash commands.

pub mod character;
pub mod chat;
pub mod emoji;
pub mod model;
pub mod name;
pub mod pin_channel;

pub use character::character;
pub use chat::chat;
pub use emoji::emoji;
pub use model::model;
pub use name::name;
pub use pin_channel::pin_channel;

use poise::serenity_prelude::{AutocompleteChoice, CreateAutocompleteResponse};

use crate::{Context, models::character::Character};

/// Returns an auto completion response from characters found in the database, sorted by similarity to the input.
pub async fn autocomplete<'a>(ctx: Context<'_>, partial: &str) -> CreateAutocompleteResponse<'a> {
    let characters: Vec<Character> = ctx
        .data()
        .db
        .characters_by_similarity(partial)
        .await
        .unwrap_or_default();

    let character_names = characters
        .into_iter()
        // TODO: having written e.g. ":microphone:" displays that text, not the emoji
        .map(|character| {
            AutocompleteChoice::new(character.to_string(), character.name().to_owned())
        })
        .collect::<Vec<AutocompleteChoice<'_>>>();
    CreateAutocompleteResponse::new().set_choices(character_names)
}
