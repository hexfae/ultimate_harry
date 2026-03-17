use crate::{
    SendResponseSnafu, db::Database, events::interaction::HistoryCharacter,
    models::modals::EditMessageModal, traits::ShowModal,
};
use miette::Report;
use poise::serenity_prelude::{ComponentInteraction, Context, EditInteractionResponse, MessageId};
use snafu::ResultExt;

pub async fn edit(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<(), Report> {
    let Some((mut history, character)) = id.history_character(db).await? else {
        return Ok(());
    };

    let Some(modal): Option<EditMessageModal> = ctx.show_modal(interaction.to_owned()).await?
    else {
        return Ok(());
    };

    history.edit_content(character.name(), modal.content, Some(interaction.user.id));

    db.update_history(history.clone()).await?;

    let response = history
        .to_response(&character, id, db)
        .await
        .to_slash_initial_response_edit(EditInteractionResponse::new());

    interaction
        .edit_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;
    Ok(())
}
