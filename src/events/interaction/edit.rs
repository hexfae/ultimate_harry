use crate::{
    db::Database, events::message::history_and_character_of, models::modals::EditMessageModal,
    traits::ShowModal,
};
use miette::{Diagnostic, Report};
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::{ResultExt, Snafu};

#[derive(Debug, Snafu, Diagnostic)]
enum EditReplyError {
    #[snafu(display("Kunde inte skicka interaktionssvar: {source}"))]
    #[diagnostic(help("Försök igen eller kontrollera att interaktionen fortfarande är giltig"), code(events::interaction::edit::send_response))]
    SendResponse { source: serenity::Error },
}

pub async fn edit(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let Some((mut history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    let Some(modal): Option<EditMessageModal> = ctx.show_modal(interaction.to_owned()).await?
    else {
        return Ok(());
    };

    history.edit_content(character.name(), modal.content, Some(interaction.user.id));

    let response = history.to_edit_interaction(&character, id, db).await;

    interaction
        .edit_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    db.update_history(history).await?;

    Ok(())
}
