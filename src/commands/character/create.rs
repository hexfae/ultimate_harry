//! The bot's Discord slash command for creating characters.

use crate::{
    AppResult, ApplicationContext, Context,
    commands::character::two_modals::prompt_two_modals,
    constants::TRANSIENT_LINGER,
    error::{DeleteMessageSnafu, SendMessageSnafu},
    models::{
        character::Character,
        modals::{CreateCharacterModal, SecondCreateCharacterModal},
    },
    phrases::created,
    traits::SayEphemeral as _,
};
use snafu::ResultExt as _;
use tokio::time::sleep;

/// Skapar en ny gubbe.
#[poise::command(slash_command, rename = "skapa")]
pub async fn create(ctx: ApplicationContext<'_>) -> AppResult {
    let Some((first_modal, second_modal)) =
        prompt_two_modals::<CreateCharacterModal, SecondCreateCharacterModal>(ctx, None, None)
            .await?
    else {
        return Ok(());
    };

    let character = Character::from((first_modal, second_modal, ctx.author().id));

    // persist the character before announcing it, so the user is never told it was
    // created when the insert (or the notice teardown) failed.
    let notice = created(&character);
    ctx.data().db.insert_character(character).await?;

    let success_message = ctx.say_ephemeral(notice).await.context(SendMessageSnafu)?;
    sleep(TRANSIENT_LINGER).await;
    success_message
        .delete(Context::Application(ctx))
        .await
        .context(DeleteMessageSnafu)?;

    Ok(())
}
