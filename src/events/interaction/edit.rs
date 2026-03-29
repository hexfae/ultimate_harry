//! The button that edits a character's chat message's contents.

use crate::{
    AppResult,
    database::Database,
    error::{SendResponseSnafu, ShowModalSnafu},
    events::message::history_and_character_of,
    models::modals::EditMessageModal,
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
) -> AppResult {
    let Some((mut history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    let Some(modal): Option<EditMessageModal> = ctx
        .show_modal(interaction.to_owned())
        .await
        .context(ShowModalSnafu)?
    else {
        return Ok(());
    };

    history.edit_content(character.name(), modal.content, Some(interaction.user.id));

    let response = history.to_edit_interaction(&character, id, db).await;

    interaction
        .edit_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    db.upsert_history(history).await?;

    Ok(())
}
