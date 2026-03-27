//! The button that shows the next revision of a message.

use miette::{Diagnostic, Result};
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::{ResultExt as _, Snafu};

use crate::{database::Database, events::message::history_and_character_of};

/// Show the next revision of this message.
pub async fn redo(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<()> {
    let Some((mut history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    history.redo();

    let response = history.to_interaction(&character, id, db).await;

    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    db.upsert_history(history).await?;

    Ok(())
}

/// All errors that can happen when redoing the message edit.
#[derive(Debug, Snafu, Diagnostic)]
enum RedoEditError {
    /// Sending a response failed.
    #[snafu(display("Kunde inte skicka interaktionssvar: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att interaktionen fortfarande är giltig"),
        code(events::interaction::redo::send_response)
    )]
    SendResponse {
        ///The source of the error.
        source: serenity::Error,
    },
}
