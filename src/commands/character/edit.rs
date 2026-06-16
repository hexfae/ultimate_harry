//! The bot's Discord slash command for editing characters.

use crate::{
    AppResult, Context,
    commands::autocomplete,
    constants::{CANCEL, EDIT, NEXT, PREVIOUS},
    error::{
        DeleteMessageSnafu, DeleteResponseSnafu, EditResponseSnafu, SendMessageSnafu,
        SendResponseSnafu, ShowModalSnafu,
    },
    events::interaction::{Interaction, InteractionKind},
    models::{
        character::Character,
        modals::{EditCharacterModal, SecondEditCharacterModal},
    },
    phrases::{cancelled, click_below, click_me, edited, no_character},
    traits::{RespondToWith as _, SayEphemeral as _},
};
use alloc::borrow::Cow;
use core::time::Duration;
use nonempty::NonEmpty;
use poise::{
    CreateReply, Modal, ReplyHandle, execute_modal_on_component_interaction,
    serenity_prelude::{
        ButtonStyle, ComponentInteraction, ComponentInteractionCollector, CreateActionRow,
        CreateButton, CreateComponent, CreateInteractionResponse, CreateInteractionResponseMessage,
        EditInteractionResponse, ReactionType,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt as _;
use tokio::time::sleep;

/// The 4 buttons that are visible on a character deletion embed.
const BUTTONS: [&str; 4] = ["conf", "canc", "prev", "next"];

#[poise::command(slash_command, rename = "ändra")]
pub async fn edit(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
) -> AppResult {
    let characters: Vec<Character> = ctx.data().db.characters_by_similarity(name).await?;

    let pages = characters.len();
    let characters_and_footers = characters
        .into_iter()
        .enumerate()
        .map(|(index, character)| {
            let similarity = character.similarity();
            let conversations_had = character.conversations_had();
            let footer_text = format!(
                "{}/{pages} | {conversations_had} konversationer{similarity}",
                index.saturating_sub(1)
            );
            (character, footer_text)
        })
        .collect();

    let Some(nonempty_characters_and_footers) = NonEmpty::from_vec(characters_and_footers) else {
        let msg = ctx
            .say_ephemeral(no_character())
            .await
            .context(SendMessageSnafu)?;
        sleep(Duration::from_secs(5)).await;
        msg.delete(ctx).await.context(DeleteMessageSnafu)?;
        return Ok(());
    };

    Box::pin(display_pagination(ctx, nonempty_characters_and_footers)).await?;
    Ok(())
}

/// Sends a message containing an embed with the character(s) and 4 buttons to manipulate them.
async fn display_pagination(
    ctx: Context<'_>,
    characters_and_footers: NonEmpty<(Character, String)>,
) -> AppResult {
    let id = ctx.id();
    let custom_ids = FixedArray::from_vec_trunc(
        BUTTONS
            .map(|suffix| FixedString::from_string_trunc(format!("{id}{suffix}")))
            .to_vec(),
    );
    let mut current_page: usize = 0;
    let pages = characters_and_footers.len();

    send_initial_embed(ctx, characters_and_footers.first().to_owned()).await?;

    while let Some(interaction) = ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(custom_ids.clone())
        .await
    {
        let interaction_type = Interaction::try_from(&interaction)?;
        match interaction_type.kind {
            InteractionKind::Previous => {
                current_page = current_page
                    .saturating_add(pages)
                    .saturating_sub(1)
                    .strict_rem(pages);
            }
            InteractionKind::Next => {
                current_page = current_page.saturating_add(1).strict_rem(pages);
            }
            InteractionKind::Cancel => {
                edit_cancelled(ctx, interaction).await?;
                return Ok(());
            }
            InteractionKind::Confirm => {
                edit_confirmed(
                    ctx,
                    interaction.clone(),
                    characters_and_footers
                        .get(current_page)
                        .unwrap_or_else(|| characters_and_footers.first())
                        .0
                        .clone(),
                )
                .await?;
                return Ok(());
            }
            _ => {} // no more kinds possible on this type of message
        }

        let (character, footer_text) = characters_and_footers
            .get(current_page)
            .unwrap_or_else(|| characters_and_footers.first())
            .clone();
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

/// Send the first message, containing the embed of a character and buttons for manipulating it.
async fn send_initial_embed(
    ctx: Context<'_>,
    (character, footer_text): (Character, String),
) -> AppResult<ReplyHandle<'_>> {
    let id = ctx.id();
    let embed = character
        .into_embed_with_footer_text(footer_text, &ctx.data().db)
        .await;
    let buttons = create_buttons(id);
    ctx.send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)
}

/// Returns a component action row containing 4 buttons for manipulating characters.
#[must_use]
pub fn create_buttons(id: u64) -> Cow<'static, [CreateComponent<'static>]> {
    let confirm = format!("{id}conf");
    let cancel = format!("{id}canc");
    let prev = format!("{id}prev");
    let next = format!("{id}next");
    vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        vec![
            CreateButton::new(confirm)
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(EDIT)))
                .style(ButtonStyle::Secondary),
            CreateButton::new(cancel)
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(
                    CANCEL,
                )))
                .style(ButtonStyle::Secondary),
            CreateButton::new(prev)
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(
                    PREVIOUS,
                )))
                .style(ButtonStyle::Secondary),
            CreateButton::new(next)
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(NEXT)))
                .style(ButtonStyle::Secondary),
        ]
        .into(),
    ))]
    .into()
}

/// Sends a message about the cancellation of editing a character, and deletes the message 5 seconds later.
async fn edit_cancelled(ctx: Context<'_>, interaction: ComponentInteraction) -> AppResult {
    ctx.respond_to_with(&interaction, cancelled())
        .await
        .context(SendMessageSnafu)?;
    sleep(Duration::from_secs(5)).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
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
    let button = vec![CreateButton::new(id).label(click_me())].into();
    let component = vec![CreateComponent::ActionRow(CreateActionRow::Buttons(button))];
    interaction
        .edit_response(
            ctx.http(),
            EditInteractionResponse::new()
                .content(click_below())
                .embeds(vec![])
                .components(component),
        )
        .await
        .context(EditResponseSnafu)
        .map(|_| ())
}
