use poise::{
    CreateReply, Modal, ReplyHandle, execute_modal_on_component_interaction,
    serenity_prelude::{ComponentInteractionCollector, CreateActionRow, CreateButton},
};
use snafu::Snafu;
use ultimate_character::{CHARACTERS, Character};
use ultimate_harry::{Context, FIVE_SECONDS, ONE_HOUR, Result, delete_invoking_message_if_prefix};
use ultimate_modals::{CreateCharacterModal, SecondCreateCharacterModal};
use ultimate_phrases::{CLICK_BELOW_PHRASES, CLICK_ME_PHRASES, CREATED_PHRASES, sample};
use ultimate_statistics::STATISTICS;

#[derive(Debug, Snafu)]
enum CreateError {
    #[snafu(display("Fick ingen modal tillbaka"))]
    NoModalReturned,
}

#[poise::command(slash_command, prefix_command, rename = "skapa")]
pub async fn create(ctx: Context<'_>) -> Result<()> {
    let (modal, second_modal) =
        show_two_modals::<CreateCharacterModal, SecondCreateCharacterModal>(ctx).await?;

    let character = Character::builder()
        .name(modal.name)
        .greeting(modal.greeting)
        .maybe_nickname(modal.nickname)
        .maybe_description(modal.description)
        .maybe_personality(modal.personality)
        .maybe_avatar(second_modal.avatar)
        .maybe_emoji(second_modal.emoji)
        .creator(ctx.author().id)
        .maybe_system_prompt(second_modal.system_prompt)
        .maybe_prompt(second_modal.prompt)
        .maybe_scenario(second_modal.scenario)
        .build();

    let msg = ctx
        .say(sample(CREATED_PHRASES).replace("{character}", &character.name()))
        .await?;

    CHARACTERS.write().insert(character);
    STATISTICS.write().character_created_by(ctx.author());

    std::thread::sleep(FIVE_SECONDS);
    msg.delete(ctx).await?;
    delete_invoking_message_if_prefix(ctx).await?;

    Ok(())
}

async fn show_two_modals<M: Modal, S: Modal>(ctx: Context<'_>) -> Result<(M, S)> {
    Ok((show_modal::<M>(ctx).await?, show_modal::<S>(ctx).await?))
}

async fn show_modal<M: Modal>(ctx: Context<'_>) -> Result<M> {
    let msg = send_tempting_button(ctx).await?;
    let id = ctx.id().to_string();
    let author = ctx.author().id;
    let collector = ComponentInteractionCollector::new(ctx.serenity_context())
        .author_id(author)
        .custom_ids(vec![id.clone()])
        .timeout(ONE_HOUR)
        .await;

    if let Some(interaction) = collector {
        msg.delete(ctx).await?;
        execute_modal_on_component_interaction::<M>(ctx, interaction, None, None)
            .await?
            .ok_or(CreateError::NoModalReturned.into())
    } else {
        Err(CreateError::NoModalReturned.into())
    }
}

async fn send_tempting_button(ctx: Context<'_>) -> Result<ReplyHandle<'_>> {
    let id = ctx.id().to_string();
    let click_me = sample(CLICK_ME_PHRASES);
    let click_below = sample(CLICK_BELOW_PHRASES);
    let button = vec![CreateButton::new(id).label(click_me)];
    let component = vec![CreateActionRow::Buttons(button)];
    ctx.send(
        CreateReply::default()
            .content(click_below)
            .components(component),
    )
    .await
    .map_err(Into::into)
}
