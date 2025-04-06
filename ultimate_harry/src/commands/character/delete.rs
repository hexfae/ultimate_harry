use poise::serenity_prelude::{ComponentInteraction, ComponentInteractionCollector};
use snafu::Snafu;
use ulid::Ulid;
use ultimate_character::CHARACTERS;
use ultimate_harry::{
    Context, FIVE_SECONDS, ONE_MINUTE, RespondWith, Result, delete_invoking_message_if_prefix,
};
use ultimate_phrases::{
    ASK_DELETE_PHRASES, CANCELLED_PHRASES, DELETED_PHRASES, NO_CHARACTER_PHRASES, sample,
};
use ultimate_statistics::STATISTICS;

#[derive(Debug, Snafu)]
enum DeleteError {
    #[snafu(display("Fick ingen modal tillbaka"))]
    NoModalReturned,
}

#[poise::command(slash_command, prefix_command, rename = "döda")]
pub async fn delete(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<()> {
    let Ok(character) = CHARACTERS.read().get_closest(&name) else {
        let msg = ctx.say(sample(NO_CHARACTER_PHRASES)).await?;
        std::thread::sleep(FIVE_SECONDS);
        msg.delete(ctx).await?;
        return Ok(());
    };

    ctx.send(character.to_confirm_reply(ctx.id(), sample(ASK_DELETE_PHRASES)))
        .await?;

    let id = ctx.id();
    let confirm_id = format!("{id}confirm");
    let cancel_id = format!("{id}cancel");

    let interaction = show_cancel_confirm_buttons(ctx).await?;
    let returned_id = interaction.data.custom_id.clone();

    if returned_id == confirm_id {
        confirm_delete(ctx, interaction, character.id()).await?;
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
        .ok_or(DeleteError::NoModalReturned.into())
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

async fn confirm_delete(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    id: impl Into<Ulid>,
) -> Result<()> {
    CHARACTERS.write().delete_by_id(id.into(), ctx.author());
    STATISTICS.write().character_deleted_by(ctx.author());

    interaction
        .respond_with(ctx, sample(DELETED_PHRASES))
        .await?;
    std::thread::sleep(FIVE_SECONDS);
    interaction.delete_response(ctx).await?;
    delete_invoking_message_if_prefix(ctx).await?;
    Ok(())
}
