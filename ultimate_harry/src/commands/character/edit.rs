use poise::{
    Modal, execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, CreateActionRow, CreateButton,
        EditInteractionResponse,
    },
};
use snafu::Snafu;
use ultimate_character::{CHARACTERS, Character};
use ultimate_harry::{
    Context, FIVE_SECONDS, ONE_HOUR, ONE_MINUTE, RespondWith, Result,
    delete_invoking_message_if_prefix,
};
use ultimate_modals::{EditCharacterModal, SecondEditCharacterModal};
use ultimate_phrases::{
    ASK_EDIT_PHRASES, CANCELLED_PHRASES, CLICK_BELOW_PHRASES, CLICK_ME_PHRASES, EDITED_PHRASES,
    NO_CHARACTER_PHRASES, sample,
};
use ultimate_statistics::STATISTICS;

#[derive(Debug, Snafu)]
enum EditError {
    #[snafu(display("Fick ingen modal tillbaka"))]
    NoModalReturned,
}

#[poise::command(slash_command, prefix_command, rename = "ändra")]
pub async fn edit(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<()> {
    let Ok(character) = CHARACTERS.read().get_closest(name) else {
        let msg = ctx.say(sample(NO_CHARACTER_PHRASES)).await?;
        std::thread::sleep(FIVE_SECONDS);
        msg.delete(ctx).await?;
        return Ok(());
    };

    ctx.send(character.to_confirm_reply(ctx.id(), sample(ASK_EDIT_PHRASES)))
        .await?;

    let id = ctx.id();
    let confirm_id = format!("{id}confirm");
    let cancel_id = format!("{id}cancel");

    let interaction = show_cancel_confirm_buttons(ctx).await?;
    let returned_id = interaction.data.custom_id.clone();

    if returned_id == confirm_id {
        confirm_edit(ctx, interaction, character).await?;
    } else if returned_id == cancel_id {
        cancel(ctx, interaction).await?;
    }
    Ok(())
}

async fn show_cancel_confirm_buttons(ctx: Context<'_>) -> Result<ComponentInteraction> {
    let id = ctx.id();
    let confirm_id = format!("{id}confirm");
    let cancel_id = format!("{id}cancel");

    ComponentInteractionCollector::new(ctx)
        .author_id(ctx.author().id)
        .custom_ids(vec![confirm_id.clone(), cancel_id.clone()])
        .timeout(ONE_MINUTE)
        .await
        .ok_or(EditError::NoModalReturned.into())
}

async fn confirm_edit(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    mut character: Character,
) -> Result<()> {
    let modal = show_first_modal::<EditCharacterModal>(ctx, interaction.clone()).await?;
    let second_modal =
        show_second_modal::<SecondEditCharacterModal>(ctx, interaction.clone()).await?;

    let old_id = character.id();

    character.edit_from_modals(ctx.author(), modal, second_modal);
    let character_name: String = character.name();
    CHARACTERS.write().supersede_by_id(old_id, character.id());

    CHARACTERS.write().insert(character);
    STATISTICS.write().character_edited_by(ctx.author());

    interaction
        .edit_response(
            ctx,
            EditInteractionResponse::new()
                .content(sample(EDITED_PHRASES).replace("{character}", &character_name))
                .components(vec![]),
        )
        .await?;
    std::thread::sleep(FIVE_SECONDS);
    interaction.delete_response(ctx).await?;
    delete_invoking_message_if_prefix(ctx).await?;
    Ok(())
}

async fn cancel(ctx: Context<'_>, interaction: ComponentInteraction) -> Result<()> {
    interaction
        .respond_with(ctx, sample(CANCELLED_PHRASES))
        .await?;
    std::thread::sleep(FIVE_SECONDS);
    interaction.delete_response(ctx).await?;
    delete_invoking_message_if_prefix(ctx).await?;
    Ok(())
}

async fn show_first_modal<M: Modal>(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> Result<M> {
    execute_modal_on_component_interaction::<M>(ctx, interaction, None, Some(ONE_HOUR))
        .await?
        .ok_or(EditError::NoModalReturned.into())
}

async fn show_second_modal<M: Modal>(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> Result<M> {
    send_first_tempting_button(ctx, interaction).await?;
    let id = ctx.id().to_string();
    let author = ctx.author().id;
    let collector = ComponentInteractionCollector::new(ctx.serenity_context())
        .author_id(author)
        .custom_ids(vec![id.clone()])
        .timeout(ONE_HOUR)
        .await;

    if let Some(interaction) = collector {
        execute_modal_on_component_interaction::<M>(ctx, interaction, None, None)
            .await?
            .ok_or(EditError::NoModalReturned.into())
    } else {
        Err(EditError::NoModalReturned.into())
    }
}

async fn send_first_tempting_button(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> Result<()> {
    let id = ctx.id().to_string();
    let click_me = sample(CLICK_ME_PHRASES);
    let click_below = sample(CLICK_BELOW_PHRASES);
    let button = vec![CreateButton::new(id).label(click_me)];
    let component = vec![CreateActionRow::Buttons(button)];
    interaction
        .edit_response(
            ctx,
            EditInteractionResponse::new()
                .content(click_below)
                .embeds(vec![])
                .components(component),
        )
        .await
        .map(|_| ())
        .map_err(Into::into)
}
