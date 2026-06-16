//! The bot's Discord slash commands for manipulating characters.

pub mod create;
pub mod delete;
pub mod edit;
pub mod model;
pub mod paginate;
pub mod view;

use crate::AppResult;
use create::create;
use delete::delete;
use edit::edit;
use model::model;
use view::view;

#[poise::command(
    slash_command,
    subcommands("create", "edit", "view", "delete", "model"),
    subcommand_required,
    rename = "gubbe"
)]
#[expect(clippy::unused_async, reason = "poise requires commands to be async")]
pub async fn character(_: crate::Context<'_>) -> AppResult {
    Ok(())
}
