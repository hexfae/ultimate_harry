//! Shared character paginator for the view, edit, and delete slash commands.
//!
//! All three run a similarity search (or usage sort), render the matching
//! characters as a swipeable embed, and let the user flip pages with the
//! Previous/Next buttons. The edit and delete commands additionally show a
//! Confirm button and act on the page the user confirms; the view command is
//! read-only and shows only the navigation buttons. The Confirm emoji and the
//! action it triggers are passed in by the caller; `None` produces the
//! read-only browse UI.

use crate::{
    AppResult, Context,
    components::emoji_button,
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
use core::future::Ready;
use core::time::Duration;
use nonempty::NonEmpty;
use poise::{
    CreateReply, ReplyHandle,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, CreateActionRow, CreateComponent,
        CreateInteractionResponse, CreateInteractionResponseMessage,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt as _;
use tokio::time::sleep;

/// The 4 button tags visible on a Confirm/Cancel character paginator embed.
const BUTTONS: [&str; 4] = [
    InteractionKind::Confirm.as_tag(),
    InteractionKind::Cancel.as_tag(),
    InteractionKind::Previous.as_tag(),
    InteractionKind::Next.as_tag(),
];

/// The 2 navigation button tags visible on a read-only browse embed.
const NAV_BUTTONS: [&str; 2] = [
    InteractionKind::Previous.as_tag(),
    InteractionKind::Next.as_tag(),
];

/// Concrete instantiation of the generic confirm callback used by [`browse`],
/// which never supplies a confirm action and therefore never calls it.
type NoConfirm = fn(Context<'_>, ComponentInteraction, Character) -> Ready<AppResult>;

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
    let Some(pages) = build_pages(ctx, characters).await? else {
        return Ok(());
    };
    Box::pin(display_pagination(ctx, pages, Some((action_emoji, on_confirm)))).await
}

/// Builds the paginated embeds for an already-fetched character list and drives
/// the read-only browse UI (Previous/Next only, no Confirm action).
pub async fn browse(ctx: Context<'_>, characters: Vec<Character>) -> AppResult {
    let Some(pages) = build_pages(ctx, characters).await? else {
        return Ok(());
    };
    Box::pin(display_pagination::<NoConfirm, Ready<AppResult>>(
        ctx, pages, None,
    ))
    .await
}

/// Builds the per-character footer text and wraps the results in a `NonEmpty`,
/// or sends the "no character" notice and returns `None` when the search came
/// back empty.
async fn build_pages(
    ctx: Context<'_>,
    characters: Vec<Character>,
) -> AppResult<Option<NonEmpty<(Character, String)>>> {
    let pages = characters.len();
    let characters_and_footers: Vec<(Character, String)> = characters
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

    if let Some(nonempty) = NonEmpty::from_vec(characters_and_footers) {
        Ok(Some(nonempty))
    } else {
        let msg = ctx
            .say_ephemeral(no_character())
            .await
            .context(SendMessageSnafu)?;
        sleep(Duration::from_secs(5)).await;
        msg.delete(ctx).await.context(DeleteMessageSnafu)?;
        Ok(None)
    }
}

/// Sends the paginator embed and loops on button presses until the user
/// confirms or cancels, re-rendering the embed on each page flip. A `None`
/// `action` renders only the navigation buttons and never confirms.
async fn display_pagination<'a, F, Fut>(
    ctx: Context<'a>,
    characters_and_footers: NonEmpty<(Character, String)>,
    action: Option<(&'static str, F)>,
) -> AppResult
where
    F: FnOnce(Context<'a>, ComponentInteraction, Character) -> Fut,
    Fut: Future<Output = AppResult>,
{
    let mut current_page: usize = 0;
    let pages = characters_and_footers.len();
    let action_emoji = action.as_ref().map(|(emoji, _)| *emoji);
    let mut on_confirm = action.map(|(_, on_confirm)| on_confirm);

    send_initial_embed(
        ctx,
        characters_and_footers.first().to_owned(),
        action_emoji,
        pages < 2,
    )
    .await?;

    while let Some(interaction) = create_collector(ctx, action_emoji.is_some()).await {
        match Interaction::try_from(&interaction)?.kind {
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
                if let Some(confirm) = on_confirm.take() {
                    let (character, _footer_text) = characters_and_footers
                        .get(current_page)
                        .unwrap_or_else(|| characters_and_footers.first())
                        .clone();
                    return confirm(ctx, interaction, character).await;
                }
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

/// Creates a collector that listens for a paginator button press with the
/// context's ID. `has_action` includes the Confirm/Cancel buttons.
#[must_use]
async fn create_collector(ctx: Context<'_>, has_action: bool) -> Option<ComponentInteraction> {
    let tags: &[&str] = if has_action { &BUTTONS } else { &NAV_BUTTONS };
    let custom_ids = FixedArray::from_vec_trunc(
        tags.iter()
            .map(|suffix| FixedString::from_string_trunc(format!("{}{suffix}", ctx.id())))
            .collect(),
    );
    ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(custom_ids)
        .await
}

/// Sends the first message, containing the embed of a character and buttons for manipulating it.
async fn send_initial_embed<'a>(
    ctx: Context<'a>,
    (character, footer_text): (Character, String),
    action_emoji: Option<&'static str>,
    nav_disabled: bool,
) -> AppResult<ReplyHandle<'a>> {
    let id = ctx.id();
    let embed = character
        .into_embed_with_footer_text(footer_text, &ctx.data().db)
        .await;
    let buttons = create_buttons(id, action_emoji, nav_disabled);
    ctx.send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)
}

/// Returns a component action row for the paginator. With a `Some` `action_emoji`
/// it prepends Confirm/Cancel buttons; otherwise only Previous/Next are shown.
/// `nav_disabled` greys out the Previous/Next buttons (used on single-page results).
#[must_use]
fn create_buttons(
    id: u64,
    action_emoji: Option<&'static str>,
    nav_disabled: bool,
) -> Cow<'static, [CreateComponent<'static>]> {
    let mut buttons = Vec::new();
    if let Some(emoji) = action_emoji {
        buttons.push(emoji_button(InteractionKind::Confirm.custom_id(id), emoji));
        buttons.push(emoji_button(InteractionKind::Cancel.custom_id(id), CANCEL));
    }
    buttons.push(
        emoji_button(InteractionKind::Previous.custom_id(id), PREVIOUS).disabled(nav_disabled),
    );
    buttons.push(emoji_button(InteractionKind::Next.custom_id(id), NEXT).disabled(nav_disabled));
    vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        buttons.into(),
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
