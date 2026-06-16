//! Shared character paginator for the edit and delete slash commands.
//!
//! Both commands run a similarity search, render the matching characters as a
//! swipeable embed with Confirm/Cancel/Previous/Next buttons, and act on the
//! page the user confirms. They differ only in the Confirm button's emoji and
//! in what confirming does, so those two pieces are passed in by the caller.

use crate::{
    AppResult, Context,
    constants::{CANCEL, NEXT, PREVIOUS},
    error::{
        DeleteMessageSnafu, DeleteResponseSnafu, SendMessageSnafu, SendResponseSnafu,
    },
    events::interaction::{Interaction, InteractionKind},
    models::character::Character,
    phrases::{cancelled, no_character},
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
        ReactionType,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt as _;
use tokio::time::sleep;

/// The 4 button tags visible on a character paginator embed.
const BUTTONS: [&str; 4] = [
    InteractionKind::Confirm.as_tag(),
    InteractionKind::Cancel.as_tag(),
    InteractionKind::Previous.as_tag(),
    InteractionKind::Next.as_tag(),
];

/// Runs a similarity search for `name`, builds the paginated embeds, and drives
/// the swipe/cancel/confirm UI. `action_emoji` is the Confirm button's emoji and
/// `on_confirm` is invoked with the character on the confirmed page.
pub async fn paginate<'a, F, Fut>(
    ctx: Context<'a>,
    name: String,
    action_emoji: &'static str,
    on_confirm: F,
) -> AppResult
where
    F: FnOnce(Context<'a>, ComponentInteraction, Character) -> Fut,
    Fut: Future<Output = AppResult>,
{
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

    Box::pin(display_pagination(
        ctx,
        nonempty_characters_and_footers,
        action_emoji,
        on_confirm,
    ))
    .await
}

/// Sends the paginator embed and loops on button presses until the user
/// confirms or cancels, re-rendering the embed on each page flip.
async fn display_pagination<'a, F, Fut>(
    ctx: Context<'a>,
    characters_and_footers: NonEmpty<(Character, String)>,
    action_emoji: &'static str,
    on_confirm: F,
) -> AppResult
where
    F: FnOnce(Context<'a>, ComponentInteraction, Character) -> Fut,
    Fut: Future<Output = AppResult>,
{
    let mut current_page: usize = 0;
    let pages = characters_and_footers.len();

    send_initial_embed(ctx, characters_and_footers.first().to_owned(), action_emoji).await?;

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
                return notify_cancelled(ctx, interaction).await;
            }
            InteractionKind::Confirm => {
                let (character, _footer_text) = characters_and_footers
                    .get(current_page)
                    .unwrap_or_else(|| characters_and_footers.first())
                    .clone();
                return on_confirm(ctx, interaction, character).await;
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

/// Creates a collector that listens for a paginator button press with the context's ID.
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

/// Sends the first message, containing the embed of a character and buttons for manipulating it.
async fn send_initial_embed<'a>(
    ctx: Context<'a>,
    (character, footer_text): (Character, String),
    action_emoji: &'static str,
) -> AppResult<ReplyHandle<'a>> {
    let id = ctx.id();
    let embed = character
        .into_embed_with_footer_text(footer_text, &ctx.data().db)
        .await;
    let buttons = create_buttons(id, action_emoji);
    ctx.send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)
}

/// Returns a component action row of 4 buttons; `action_emoji` is the Confirm button's emoji.
#[must_use]
fn create_buttons(id: u64, action_emoji: &'static str) -> Cow<'static, [CreateComponent<'static>]> {
    let confirm = format!("{id}{}", InteractionKind::Confirm);
    let cancel = format!("{id}{}", InteractionKind::Cancel);
    let prev = format!("{id}{}", InteractionKind::Previous);
    let next = format!("{id}{}", InteractionKind::Next);
    vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        vec![
            CreateButton::new(confirm)
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(
                    action_emoji,
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

/// Sends a cancellation message in response to `interaction`, then deletes it 5 seconds later.
pub async fn notify_cancelled(ctx: Context<'_>, interaction: ComponentInteraction) -> AppResult {
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
