//! The button that edits a character's chat message's contents.

use crate::{
    AppResult,
    database::Database,
    error::{EditResponseSnafu, ShowModalSnafu},
    models::{character::Character, history::History, modals::EditMessageModal},
    traits::ShowModal as _,
};
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::ResultExt as _;

/// Edit the contents of a character's chat message.
pub async fn edit(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
    mut history: History,
    character: Character,
) -> AppResult {
    let Some(modal): Option<EditMessageModal> = ctx
        .show_modal(interaction.to_owned())
        .await
        .context(ShowModalSnafu)?
    else {
        return Ok(());
    };

    history.edit_content(character.name(), modal.content, Some(interaction.user.id));

    let options = db.character_menu_options().await?;
    let response = history
        .to_edit_interaction(&character, id, db, &options)
        .await;

    interaction
        .edit_response(&ctx.http, response)
        .await
        .context(EditResponseSnafu)?;

    db.upsert_history(history).await?;

    Ok(())
}
