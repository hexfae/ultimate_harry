use crate::{SendResponseSnafu, event_handler::HistoryCharacter, traits::ShowModal};
use miette::Report;
use poise::serenity_prelude::{ComponentInteraction, Context, EditInteractionResponse, MessageId};
use snafu::ResultExt;
use ultimate_database::DB;
use ultimate_modals::EditMessageModal;

pub async fn edit(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
) -> Result<(), Report> {
    let Some((mut history, character)) = id.history_character().await? else {
        return Ok(());
    };

    let Some(modal): Option<EditMessageModal> = ctx.show_modal(interaction.to_owned()).await?
    else {
        return Ok(());
    };

    history.edit_content(character.name(), modal.content, interaction.user.id);

    DB.update_history(history.clone()).await?;

    let response = history
        .to_response(&character, id)
        .to_slash_initial_response_edit(EditInteractionResponse::new());

    interaction
        .edit_response(ctx, response)
        .await
        .context(SendResponseSnafu)?;
    Ok(())
}
