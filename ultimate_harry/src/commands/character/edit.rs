use crate::{
    Context, DeferEphemeralOrBroadcast, DeleteInvokingMessageIfPrefix, DeleteResponseSnafu,
    EditResponseSnafu, Error, FIVE_SECONDS, ONE_HOUR, RespondToWith, Result, SendMessageSnafu,
    SendResponseSnafu, ShowModalSnafu, TEN_MINUTES,
    traits::{DeleteSelfAndInvokingMessageIfPrefix, SayWith},
};
use miette::Report;
use poise::{
    CreateReply, Modal, ReplyHandle, execute_modal_on_component_interaction,
    serenity_prelude::{
        ButtonStyle, ComponentInteraction, ComponentInteractionCollector, CreateActionRow,
        CreateButton, CreateInteractionResponse, CreateInteractionResponseMessage,
        EditInteractionResponse, ReactionType,
    },
};
use snafu::ResultExt;
use tokio::time::sleep;
use ultimate_character::{CHARACTERS, Character};
use ultimate_modals::{EditCharacterModal, SecondEditCharacterModal};
use ultimate_phrases::{
    CANCELLED_PHRASES, CLICK_BELOW_PHRASES, CLICK_ME_PHRASES, EDITED_PHRASES, NO_CHARACTER_PHRASES,
    sample, sample_name,
};
use ultimate_statistics::STATISTICS;

enum InteractionType {
    Prev,
    Next,
    Confirm,
    Cancel,
}

#[poise::command(slash_command, prefix_command, rename = "ändra")]
pub async fn edit(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<(), Report> {
    ctx.defer_ephemeral_or_broadcast().await?;

    let characters = CHARACTERS.read().get_all_sorted_by_similarity(name);

    if characters.is_empty() {
        let response = sample(NO_CHARACTER_PHRASES);
        let msg = ctx.say_with(response).await?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    }

    let pages = characters.len();
    let characters_and_footer_text = characters
        .into_iter()
        .enumerate()
        .map(|(index, (similarity, character))| {
            let index = index + 1;
            let similarity = format!("{:.0}", similarity * 100.0);
            let conversations_had = character.conversations_had();
            let footer_text = format!(
                "{index}/{pages} | {conversations_had} konversationer | {similarity}% namnlikhet"
            );
            (character, footer_text)
        })
        .collect();

    display_pagination(ctx, characters_and_footer_text).await?;
    Ok(())
}

async fn display_pagination(
    ctx: Context<'_>,
    characters_and_footer_text: Vec<(Character, String)>,
) -> Result<()> {
    let id = ctx.id();
    let custom_ids = ["prev", "next", "confirm", "cancel"]
        .map(|s| format!("{id}{s}"))
        .to_vec();
    let mut current_page: usize = 0;
    let pages = characters_and_footer_text.len();

    // index into 0 is safe because we checked `is_empty()` earlier
    let msg = send_initial_embed(ctx, characters_and_footer_text[0].clone()).await?;

    while let Some(interaction) = ComponentInteractionCollector::new(ctx)
        .custom_ids(custom_ids.clone())
        .timeout(TEN_MINUTES)
        .await
    {
        let interaction_type = InteractionType::try_from(&interaction)?;
        match interaction_type {
            InteractionType::Prev => {
                current_page = (current_page + pages - 1) % pages;
            }
            InteractionType::Next => {
                current_page = (current_page + 1) % pages;
            }
            InteractionType::Confirm => {
                STATISTICS.write().character_edited_by(ctx.author());
                edit_confirmed(
                    ctx,
                    interaction.clone(),
                    characters_and_footer_text[current_page].0.clone(),
                )
                .await?;
                return Ok(());
            }
            InteractionType::Cancel => {
                edit_cancelled(ctx, interaction).await?;
                return Ok(());
            }
        }

        let (character, footer_text) = characters_and_footer_text[current_page].clone();
        let embed = character.to_embed_with_footer_text(footer_text);

        interaction
            .create_response(
                ctx,
                CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new().embed(embed),
                ),
            )
            .await
            .context(SendResponseSnafu)?;
    }

    msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
    Ok(())
}

async fn send_initial_embed(
    ctx: Context<'_>,
    (character, footer_text): (Character, String),
) -> Result<ReplyHandle<'_>> {
    let id = ctx.id();
    let embed = character.to_embed_with_footer_text(footer_text);
    let buttons = create_buttons(id);
    ctx.send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)
}

#[must_use]
pub fn create_buttons(id: u64) -> Vec<CreateActionRow> {
    let confirm = format!("{id}confirm");
    let cancel = format!("{id}cancel");
    let prev = format!("{id}prev");
    let next = format!("{id}next");
    let components = CreateActionRow::Buttons(vec![
        CreateButton::new(&confirm)
            .emoji("✏️".parse::<ReactionType>().expect("valid emoji"))
            .style(ButtonStyle::Secondary),
        CreateButton::new(&cancel)
            .emoji('❌')
            .style(ButtonStyle::Secondary),
        CreateButton::new(&prev)
            .emoji('◀')
            .style(ButtonStyle::Secondary),
        CreateButton::new(&next)
            .emoji('▶')
            .style(ButtonStyle::Secondary),
    ]);
    vec![components]
}

async fn edit_confirmed(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    mut character: Character,
) -> Result<()> {
    let Some(modal) = show_first_modal::<EditCharacterModal>(ctx, interaction.clone()).await?
    else {
        return Ok(());
    };
    let Some(second_modal) =
        show_second_modal::<SecondEditCharacterModal>(ctx, interaction.clone()).await?
    else {
        return Ok(());
    };

    let old_id = character.id();

    character.edit_from_modals(ctx.author(), modal, second_modal);
    let character_name = character.name().to_owned();
    CHARACTERS.write().supersede_by_id(old_id, character.id());

    CHARACTERS.write().insert(character);
    STATISTICS.write().character_edited_by(ctx.author());

    interaction
        .edit_response(
            ctx,
            EditInteractionResponse::new()
                .content(sample_name(EDITED_PHRASES, character_name))
                .components(vec![]),
        )
        .await
        .context(EditResponseSnafu)?;
    sleep(FIVE_SECONDS).await;
    interaction
        .delete_response(ctx)
        .await
        .context(DeleteResponseSnafu)?;
    ctx.delete_invoking_message_if_prefix().await?;
    Ok(())
}

async fn edit_cancelled(ctx: Context<'_>, interaction: ComponentInteraction) -> Result<()> {
    let response = sample(CANCELLED_PHRASES);
    ctx.respond_to_with(&interaction, response).await?;
    sleep(FIVE_SECONDS).await;
    interaction
        .delete_response(ctx)
        .await
        .context(DeleteResponseSnafu)?;
    ctx.delete_invoking_message_if_prefix().await?;
    Ok(())
}

async fn show_first_modal<M: Modal>(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> Result<Option<M>> {
    execute_modal_on_component_interaction::<M>(ctx, interaction, None, Some(ONE_HOUR))
        .await
        .context(ShowModalSnafu)
}

async fn show_second_modal<M: Modal>(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> Result<Option<M>> {
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
            .await
            .context(ShowModalSnafu)
    } else {
        Ok(None)
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
        .context(EditResponseSnafu)
        .map(|_| ())
}

impl TryFrom<&ComponentInteraction> for InteractionType {
    type Error = Error;

    fn try_from(input: &ComponentInteraction) -> Result<Self, Self::Error> {
        match input.data.custom_id.as_str() {
            i if i.ends_with("prev") => Ok(Self::Prev),
            i if i.ends_with("next") => Ok(Self::Next),
            i if i.ends_with("confirm") => Ok(Self::Confirm),
            i if i.ends_with("cancel") => Ok(Self::Cancel),
            i => Err(Error::UnknownInteraction {
                found: i.to_string(),
            }),
        }
    }
}
