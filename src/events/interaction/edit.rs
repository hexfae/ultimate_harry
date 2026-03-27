//! The button that edits a character's chat message's contents.

use crate::{
    database::Database, events::message::history_and_character_of,
    models::modals::EditMessageModal, traits::ShowModal as _,
};
use miette::{Diagnostic, Result};
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::{ResultExt as _, Snafu};

/// Edit the contents of a character's chat message.
pub async fn edit(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<()> {
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

/// All errors that can happen when editing an answer.
#[derive(Debug, Snafu, Diagnostic)]
enum EditAnswerError {
    /// Sending a response failed.
    #[snafu(display("Kunde inte skicka interaktionssvar: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att interaktionen fortfarande är giltig"),
        code(events::interaction::edit::send_response)
    )]
    SendResponse {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Showing a modal failed.
    #[snafu(display("Kunde inte visa modal: {source}"))]
    #[diagnostic(
        help("Försök igen om en stund"),
        code(events::interaction::edit::show_modal)
    )]
    ShowModal {
        /// The source of the error.
        source: serenity::Error,
    },
}
