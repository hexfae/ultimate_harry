//! The bot's Discord slash command for deleting characters.

use crate::{
    AppResult, Context,
    commands::autocomplete,
    constants::{CANCEL, DELETE, NEXT, PREVIOUS},
    error::{DeleteMessageSnafu, DeleteResponseSnafu, SendMessageSnafu, SendResponseSnafu},
    events::interaction::{Interaction, InteractionKind},
    models::character::Character,
    phrases::{ask_delete, cancelled, deleted, no_character},
    traits::{RespondToWith as _, SayEphemeral as _},
};
use alloc::borrow::Cow;
use core::time::Duration;
use nonempty::NonEmpty;
use poise::{
    CreateReply, ReplyHandle,
    serenity_prelude::{
        ButtonStyle, ComponentInteraction, ComponentInteractionCollector, CreateActionRow,
        CreateButton, CreateComponent, CreateInteractionResponse, CreateInteractionResponseMessage,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use serenity::all::ReactionType;
use snafu::ResultExt as _;
use surrealdb::RecordId;
use tokio::time::sleep;

/// The 4 buttons that are visible on a character deletion embed.
const BUTTONS: [&str; 4] = ["conf", "canc", "prev", "next"];

#[poise::command(slash_command, rename = "döda")]
pub async fn delete(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
) -> AppResult {
    let characters: Vec<Character> = ctx.data().db.characters_by_similarity(&name).await?;

    let pages = characters.len();
    let characters_and_footers = characters
        .into_iter()
        .enumerate()
        .map(|(index, character)| {
            let similarity = character.similarity();
            let conversations_had = character.conversations_had();
            let footer_text = format!(
                "{}/{pages} | {conversations_had} konversationer{similarity}",
                index.saturating_add(1)
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

    display_pagination(ctx, nonempty_characters_and_footers).await?;
    Ok(())
}

/// Sends a message containing an embed with the character(s) and 4 buttons to manipulate them.
async fn display_pagination(
    ctx: Context<'_>,
    characters_and_footers: NonEmpty<(Character, String)>,
) -> AppResult {
    let mut current_page: usize = 0;
    let pages = characters_and_footers.len();

    send_initial_embed(ctx, characters_and_footers.first().to_owned()).await?;

    while let Some(interaction) = create_collector(ctx).await {
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
                delete_cancelled(ctx, interaction).await?;
                return Ok(());
            }
            InteractionKind::Confirm => {
                let (character, _footer_text) = characters_and_footers
                    .get(current_page)
                    .unwrap_or_else(|| characters_and_footers.first())
                    .clone();

                let confirm_id = format!("{}conf", ctx.id());

                let Some(response) = ask_for_confirmation(ctx, interaction).await? else {
                    continue;
                };
                let returned_id = response.data.custom_id.clone();

                if returned_id == confirm_id {
                    delete_confirmed(ctx, response, character.id()).await?;
                } else {
                    delete_cancelled(ctx, response).await?;
                }
                return Ok(());
            }
            _ => {} // no more kinds possible on this type of message
        }

        let (character, footer_text) = characters_and_footers
            .get(current_page)
            .unwrap_or_else(|| characters_and_footers.first())
            .to_owned();
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

/// Create a collector that listens for a button press with the context's ID.
#[must_use]
async fn create_collector(ctx: Context<'_>) -> Option<ComponentInteraction> {
    let custom_ids = FixedArray::from_vec_trunc(
        BUTTONS
            .map(|suffix| FixedString::from_string_trunc(format!("{}{suffix}", ctx.id())))
            .to_vec(),
    );
    ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(custom_ids)
        .await
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
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(
                    DELETE,
                )))
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

/// Sends a message about the cancellation of deleting a character, and deletes the message 5 seconds later.
async fn delete_cancelled(ctx: Context<'_>, interaction: ComponentInteraction) -> AppResult {
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

/// Sends a message asking the user for confirmation on deleting the character.
async fn ask_for_confirmation(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> AppResult<Option<ComponentInteraction>> {
    let reply = Character::to_confirm_interaction_response(ctx.id(), ask_delete());
    interaction
        .create_response(ctx.http(), reply)
        .await
        .context(SendResponseSnafu)?;

    let id = ctx.id();
    let confirm_id = format!("{id}conf");
    let cancel_id = format!("{id}canc");

    Ok(ComponentInteractionCollector::new(ctx.serenity_context())
        .author_id(ctx.author().id)
        .custom_ids(FixedArray::from_vec_trunc(vec![
            FixedString::from_string_trunc(confirm_id.clone()),
            FixedString::from_string_trunc(cancel_id.clone()),
        ]))
        .await)
}

/// Deletes the character, sends a message stating it was deleted, and deletes the message 5 seconds later.
async fn delete_confirmed(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    id: &RecordId,
) -> AppResult {
    ctx.data().db.delete_character(id, ctx.author()).await?;

    ctx.respond_to_with(&interaction, deleted())
        .await
        .context(SendMessageSnafu)?;
    sleep(Duration::from_secs(5)).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
}
