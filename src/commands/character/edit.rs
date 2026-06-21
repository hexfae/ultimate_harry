//! The bot's Discord slash command for editing characters.

use crate::{
    AppResult, Context,
    commands::{autocomplete, character::paginate::paginate},
    components::single_button_row,
    constants::EDIT,
    error::{DeleteResponseSnafu, EditResponseSnafu, ShowModalSnafu},
    models::{
        character::Character,
        modals::{EditCharacterModal, SecondEditCharacterModal},
    },
    phrases::{click_below, click_me, edited},
};
use core::time::Duration;
use poise::{
    Modal, execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, EditInteractionResponse,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt as _;
use tokio::time::sleep;

/// Ändrar en gubbe.
#[poise::command(slash_command, rename = "ändra")]
pub async fn edit(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
) -> AppResult {
    paginate(ctx, name, EDIT, edit_confirmed).await
}

/// Sends 2 buttons back-to-back that show modals, edits the character, sends
/// a message stating it was edit, and deletes the message 5 seconds later.
async fn edit_confirmed(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    mut character: Character,
) -> AppResult {
    let Some(modal) = show_first_modal::<EditCharacterModal>(ctx, interaction.clone()).await?
    else {
        return Ok(());
    };
    let Some(second_modal) =
        show_second_modal::<SecondEditCharacterModal>(ctx, interaction.clone()).await?
    else {
        return Ok(());
    };

    let old_id = character.id().to_owned();

    character.edit_from_modals(ctx.author(), modal, second_modal);
    let character_name = character.to_string();

    ctx.data()
        .db
        .supersede_character(character.id().to_owned(), &old_id)
        .await?;
    ctx.data().db.insert_character(character).await?;

    interaction
        .edit_response(
            ctx.http(),
            EditInteractionResponse::new()
                .content(edited(character_name))
                .components(vec![]),
        )
        .await
        .context(EditResponseSnafu)?;
    sleep(Duration::from_secs(5)).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
}

/// Immediately shows the first modal to the user.
async fn show_first_modal<M: Modal>(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> AppResult<Option<M>> {
    execute_modal_on_component_interaction::<M>(ctx.serenity_context(), interaction, None, None)
        .await
        .context(ShowModalSnafu)
}

/// Sends a button that attempts to tempt the user into pressing it, then shows them a modal.
async fn show_second_modal<M: Modal>(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> AppResult<Option<M>> {
    send_first_tempting_button(ctx, interaction).await?;
    let id = ctx.id().to_string();
    let author = ctx.author().id;
    let collector = ComponentInteractionCollector::new(ctx.serenity_context())
        .author_id(author)
        .custom_ids(FixedArray::from_vec_trunc(vec![
            FixedString::from_string_trunc(id.clone()),
        ]))
        .await;

    if let Some(second_interaction) = collector {
        execute_modal_on_component_interaction::<M>(
            ctx.serenity_context(),
            second_interaction,
            None,
            None,
        )
        .await
        .context(ShowModalSnafu)
    } else {
        Ok(None)
    }
}

/// Sends a message that attempts to tempt the user into pressing it.
async fn send_first_tempting_button(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> AppResult {
    let id = ctx.id().to_string();
    interaction
        .edit_response(
            ctx.http(),
            EditInteractionResponse::new()
                .content(click_below())
                .embeds(vec![])
                .components(single_button_row(id, click_me())),
        )
        .await
        .context(EditResponseSnafu)
        .map(|_| ())
}
