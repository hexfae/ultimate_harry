use std::{borrow::Cow, time::Duration};

use crate::{
    Context, DeferSnafu, DeleteResponseSnafu, EditResponseSnafu, RespondToWith, Result,
    SendMessageSnafu, SendResponseSnafu, ShowModalSnafu,
    commands::character::InteractionType,
    constants::{
        CANCEL, CANCELLED_PHRASES, CLICK_BELOW_PHRASES, CLICK_ME_PHRASES, EDIT, EDITED_PHRASES,
        NEXT, NO_CHARACTER_PHRASES, PREVIOUS, sample, sample_name,
    },
    models::{
        character::Character,
        modals::{EditCharacterModal, SecondEditCharacterModal},
    },
    traits::SayWith,
};
use miette::Report;
use poise::{
    CreateReply, Modal, ReplyHandle, execute_modal_on_component_interaction,
    serenity_prelude::{
        ButtonStyle, ComponentInteraction, ComponentInteractionCollector, CreateActionRow,
        CreateButton, CreateComponent, CreateInteractionResponse, CreateInteractionResponseMessage,
        EditInteractionResponse, ReactionType,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt;
use tokio::time::sleep;

#[poise::command(slash_command, rename = "ändra")]
pub async fn edit(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<(), Report> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;

    let characters: Vec<Character> = ctx.data().db.characters_by_similarity(name).await?;

    if characters.is_empty() {
        let response = sample(NO_CHARACTER_PHRASES);
        ctx.say_with(response).await?;
        return Ok(());
    }

    let pages = characters.len();
    let characters_and_footer_text = characters
        .into_iter()
        .enumerate()
        .map(|(index, character)| {
            let index = index + 1;
            let similarity = character.similarity();
            let conversations_had = character.conversations_had();
            let footer_text =
                format!("{index}/{pages} | {conversations_had} konversationer{similarity}");
            (character, footer_text)
        })
        .collect();

    Box::pin(display_pagination(ctx, characters_and_footer_text)).await?;
    Ok(())
}

async fn display_pagination(
    ctx: Context<'_>,
    characters_and_footer_text: Vec<(Character, String)>,
) -> Result<(), Report> {
    let id = ctx.id();
    let custom_ids = FixedArray::from_vec_trunc(
        ["prev", "next", "confirm", "cancel"]
            .map(|s| FixedString::from_string_trunc(format!("{id}{s}")))
            .to_vec(),
    );
    let mut current_page: usize = 0;
    let pages = characters_and_footer_text.len();

    // index into 0 is safe because we checked `is_empty()` earlier
    send_initial_embed(ctx, characters_and_footer_text[0].clone()).await?;

    while let Some(interaction) = ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(custom_ids.clone())
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
            InteractionType::Cancel => {
                edit_cancelled(ctx, interaction).await?;
                return Ok(());
            }
            InteractionType::Confirm => {
                ctx.data().stats.character_edited_by(ctx.author());
                edit_confirmed(
                    ctx,
                    interaction.clone(),
                    characters_and_footer_text[current_page].0.clone(),
                )
                .await?;
                return Ok(());
            }
        }

        let (character, footer_text) = characters_and_footer_text[current_page].clone();
        let embed = character
            .into_embed_with_footer_text(footer_text, &ctx.data().db)
            .await;

        interaction
            .create_response(
                ctx.http(),
                CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new().embed(embed),
                ),
            )
            .await
            .context(SendResponseSnafu)?;
    }

    Ok(())
}

async fn send_initial_embed(
    ctx: Context<'_>,
    (character, footer_text): (Character, String),
) -> Result<ReplyHandle<'_>> {
    let id = ctx.id();
    let embed = character
        .into_embed_with_footer_text(footer_text, &ctx.data().db)
        .await;
    let buttons = create_buttons(id);
    ctx.send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)
}

#[must_use]
pub fn create_buttons(id: u64) -> Cow<'static, [CreateComponent<'static>]> {
    let confirm = format!("{id}confirm");
    let cancel = format!("{id}cancel");
    let prev = format!("{id}prev");
    let next = format!("{id}next");
    Cow::Owned(vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        Cow::Owned(vec![
            CreateButton::new(confirm)
                .emoji(EDIT.parse::<ReactionType>().expect("valid emoji"))
                .style(ButtonStyle::Secondary),
            CreateButton::new(cancel)
                .emoji(CANCEL.parse::<ReactionType>().expect("valid emoji"))
                .style(ButtonStyle::Secondary),
            CreateButton::new(prev)
                .emoji(PREVIOUS.parse::<ReactionType>().expect("valid emoji"))
                .style(ButtonStyle::Secondary),
            CreateButton::new(next)
                .emoji(NEXT.parse::<ReactionType>().expect("valid emoji"))
                .style(ButtonStyle::Secondary),
        ]),
    ))])
}

async fn edit_confirmed(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    mut character: Character,
) -> Result<(), Report> {
    let Some(modal) = show_first_modal::<EditCharacterModal>(ctx, interaction.clone()).await?
    else {
        return Ok(());
    };
    let Some(second_modal) =
        show_second_modal::<SecondEditCharacterModal>(ctx, interaction.clone()).await?
    else {
        return Ok(());
    };

    let old_id = character.id().clone();

    character.edit_from_modals(ctx.author(), modal, second_modal);
    let character_name = character.name().to_owned();

    ctx.data()
        .db
        .supersede_character(character.id().to_owned(), &old_id)
        .await?;
    ctx.data().db.insert_character(character).await?;

    ctx.data().stats.character_edited_by(ctx.author());

    interaction
        .edit_response(
            ctx.http(),
            EditInteractionResponse::new()
                .content(sample_name(EDITED_PHRASES, character_name))
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

async fn edit_cancelled(ctx: Context<'_>, interaction: ComponentInteraction) -> Result<()> {
    let response = sample(CANCELLED_PHRASES);
    ctx.respond_to_with(&interaction, response).await?;
    sleep(Duration::from_secs(5)).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
}

async fn show_first_modal<M: Modal>(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> Result<Option<M>> {
    execute_modal_on_component_interaction::<M>(ctx.serenity_context(), interaction, None, None)
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
        .custom_ids(FixedArray::from_vec_trunc(vec![
            FixedString::from_string_trunc(id.clone()),
        ]))
        .await;

    if let Some(interaction) = collector {
        execute_modal_on_component_interaction::<M>(ctx.serenity_context(), interaction, None, None)
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
    let button = Cow::Owned(vec![CreateButton::new(id).label(click_me)]);
    let component = Cow::Owned(vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        button,
    ))]);
    interaction
        .edit_response(
            ctx.http(),
            EditInteractionResponse::new()
                .content(click_below)
                .embeds(vec![])
                .components(component),
        )
        .await
        .context(EditResponseSnafu)
        .map(|_| ())
}
