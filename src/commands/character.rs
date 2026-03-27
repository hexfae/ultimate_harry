//! The bot's Discord slash commands for manipulating characters.

mod create;
mod delete;
mod edit;
mod interaction;
mod view;

use create::create;
use delete::delete;
use edit::edit;
use view::view;

#[poise::command(
    slash_command,
    subcommands("create", "edit", "view", "delete"),
    subcommand_required,
    rename = "gubbe"
)]
#[expect(clippy::unused_async, reason = "poise requires commands to be async")]
pub async fn character(_: crate::Context<'_>) -> Result<(), miette::Report> {
    Ok(())
}
