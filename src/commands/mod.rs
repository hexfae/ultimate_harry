mod character;
mod chat;
mod emoji;
mod model;
mod pin;

pub use character::character;
pub use chat::chat;
pub use emoji::emoji;
pub use model::model;
pub use pin::pin;
use poise::serenity_prelude::{AutocompleteChoice, CreateAutocompleteResponse};

use crate::{Context, models::character::Character};

pub async fn autocomplete<'a>(ctx: Context<'_>, partial: &str) -> CreateAutocompleteResponse<'a> {
    let characters: Vec<Character> = ctx
        .data()
        .db
        .characters_by_similarity(partial)
        .await
        .unwrap_or_default();
    // characters.sort_unstable();
    // characters.reverse();

    let character_names = characters
        .into_iter()
        // .filter(|character| {
        //     character
        //         .name()
        //         .to_lowercase()
        //         .starts_with(&partial.to_lowercase())
        // })
        // .take(25)
        // TODO: having written e.g. ":microphone" displays that text, not the emoji
        // TODO: does this work correctly? i think so
        .map(|character| {
            AutocompleteChoice::new(character.to_string(), character.name().to_owned())
        })
        .collect::<Vec<AutocompleteChoice>>();
    CreateAutocompleteResponse::new().set_choices(character_names)
}
