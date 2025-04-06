use create::create;
use delete::delete;
use edit::edit;
use ultimate_harry::{Context, Result};
use view::view;

mod create;
mod delete;
mod edit;
mod view;

#[poise::command(
    slash_command,
    prefix_command,
    subcommands("create", "edit", "view", "delete"),
    subcommand_required,
    rename = "gubbe"
)]
pub async fn character(_: Context<'_>) -> Result<()> {
    Ok(())
}
