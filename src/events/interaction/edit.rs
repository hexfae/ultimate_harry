//! The button that edits a character's chat message's contents.

use crate::{
    AppResult,
    database::Database,
    error::{EditResponseSnafu, ShowModalSnafu},
    models::{character::Character, history::History, modals::EditMessageModal},
};
use poise::{
    execute_modal_on_component_interaction,
    serenity_prelude::{ComponentInteraction, Context, MessageId},
};
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
    let Some(modal): Option<EditMessageModal> = execute_modal_on_component_interaction(
        ctx,
        interaction.to_owned(),
        Some(history.edit_modal_default()),
        None,
    )
    .await
    .context(ShowModalSnafu)?
    else {
        return Ok(());
    };

    history.edit_content(character.name(), modal.content, Some(interaction.user.id));

    let options = db.character_menu_options().await?;
    let voices = db.voice_options().await;

    // persist the edit before rendering it, so a save failure surfaces as an error
    // rather than showing an edit that was never stored.
    db.upsert_history(history.clone()).await?;

    let response = history
        .to_edit_interaction(&character, id, db, &options, &voices)
        .await;
    interaction
        .edit_response(&ctx.http, response)
        .await
        .context(EditResponseSnafu)?;

    Ok(())
}
