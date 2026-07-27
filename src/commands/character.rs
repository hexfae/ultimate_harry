//! The bot's Discord slash commands for manipulating characters.

pub mod clone;
pub mod color;
pub mod create;
pub mod delete;
pub mod edit;
pub mod example;
pub mod model;
pub mod paginate;
pub mod prompt;
pub mod render;
pub mod restore;
pub mod two_modals;
pub mod view;
pub mod voice;

use crate::AppResult;
use clone::clone;
use color::color;
use create::create;
use delete::delete;
use edit::edit;
use example::example;
use model::model;
use prompt::prompt;
use restore::restore;
use view::view;
use voice::voice;

/// Hanterar gubbar.
#[poise::command(
    slash_command,
    subcommands(
        "create", "clone", "color", "edit", "view", "delete", "model", "restore", "voice",
        "example", "prompt"
    ),
    subcommand_required,
    rename = "gubbe"
)]
#[expect(clippy::unused_async, reason = "poise requires commands to be async")]
pub async fn character(_: crate::Context<'_>) -> AppResult {
    Ok(())
}
