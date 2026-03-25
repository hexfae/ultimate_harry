use miette::{Diagnostic, Report};
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::{ResultExt, Snafu};

use crate::{db::Database, events::message::history_and_character_of};

#[derive(Debug, Snafu, Diagnostic)]
enum RedoEditError {
    #[snafu(display("Kunde inte skicka interaktionssvar: {source}"))]
    #[diagnostic(help("Försök igen eller kontrollera att interaktionen fortfarande är giltig"), code(events::interaction::redo::send_response))]
    SendResponse { source: serenity::Error },
}

pub async fn redo(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let Some((mut history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    history.redo();

    let response = history.to_interaction(&character, id, db).await;

    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    db.update_history(history).await?;

    Ok(())
}
