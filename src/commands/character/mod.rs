use create::create;
use delete::delete;
use edit::edit;
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
// poise requires this to be async
#[allow(clippy::unused_async)]
pub async fn character(_: crate::Context<'_>) -> Result<(), miette::Report> {
    Ok(())
}
