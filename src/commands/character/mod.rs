use create::create;
use delete::delete;
use edit::edit;
use view::view;

pub mod create;
pub mod delete;
pub mod edit;
pub mod interaction;
pub mod view;

#[poise::command(
    slash_command,
    subcommands("create", "edit", "view", "delete"),
    subcommand_required,
    rename = "gubbe"
)]
// poise requires this to be async
#[expect(clippy::unused_async)]
pub async fn character(_: crate::Context<'_>) -> Result<(), miette::Report> {
    Ok(())
}
