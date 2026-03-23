use create::create;
use delete::delete;
use edit::edit;
use view::view;

use crate::Error;
use serenity::all::ComponentInteraction;

mod create;
mod delete;
mod edit;
mod view;

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

pub enum InteractionType {
    Prev,
    Next,
    Confirm,
    Cancel,
}

impl TryFrom<&ComponentInteraction> for InteractionType {
    type Error = Error;

    fn try_from(input: &ComponentInteraction) -> Result<Self, Self::Error> {
        match input.data.custom_id.as_str() {
            i if i.ends_with("prev") => Ok(Self::Prev),
            i if i.ends_with("next") => Ok(Self::Next),
            i if i.ends_with("confirm") => Ok(Self::Confirm),
            i if i.ends_with("cancel") => Ok(Self::Cancel),
            i => Err(Error::UnknownInteraction {
                found: i.to_string(),
            }),
        }
    }
}
