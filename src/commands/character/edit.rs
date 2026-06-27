//! The bot's Discord slash command for editing characters.

use crate::{
    AppResult, ApplicationContext, Context,
    commands::{
        autocomplete, character::two_modals::prompt_two_modals, first_character_or_notify,
        say_transient,
    },
    models::modals::resolve_edit_modals,
    phrases::edited,
    shortcodes::guild_emojis,
};

/// Ändrar en gubbe.
#[poise::command(slash_command, rename = "ändra")]
pub async fn edit(
    ctx: ApplicationContext<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
) -> AppResult {
    let Some(mut character) = first_character_or_notify(Context::Application(ctx), name).await?
    else {
        return Ok(());
    };

    // the pre-filled first modal shows the current values, so it doubles as the
    // "is this the right gubbe?" check the confirmation step used to provide.
    let (first_default, second_default) = character.edit_modal_defaults();
    let Some((mut first_modal, mut second_modal)) =
        prompt_two_modals(ctx, Some(first_default), Some(second_default)).await?
    else {
        return Ok(());
    };

    let guild_emojis = guild_emojis(ctx).await;
    resolve_edit_modals(&mut first_modal, &mut second_modal, &guild_emojis);

    let old_id = character.id().to_owned();

    character.edit_from_modals(ctx.author(), first_modal, second_modal);
    let character_name = character.to_string();

    ctx.data()
        .db
        .supersede_character(character.id().to_owned(), &old_id)
        .await?;
    ctx.data().db.insert_character(character).await?;

    say_transient(ctx, edited(character_name)).await
}
