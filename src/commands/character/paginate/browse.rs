//! The read-only version browser used by the `/gubbe visa` command.
//!
//! Unlike the confirm paginator in the parent module, this flips between search
//! results with the page buttons, walks the shown character's version chain with
//! the version buttons, and can roll a character back to an older version. It
//! reuses the parent's `create_collector` and `respond_then_clear` helpers.

use super::{create_collector, respond_then_clear};
use crate::{
    AppResult, Context,
    commands::character::render::character_embed,
    commands::notify_no_character,
    components::emoji_button,
    constants::{NEWER_VERSION, NEXT, OLDER_VERSION, PREVIOUS, ROLLBACK},
    error::{SendMessageSnafu, SendResponseSnafu},
    events::interaction::{Interaction, InteractionKind},
    models::character::Character,
    phrases::rolled_back,
    util::wrapping_previous,
};
use alloc::borrow::Cow;
use nonempty::NonEmpty;
use poise::{
    CreateReply,
    serenity_prelude::{
        ComponentInteraction, CreateActionRow, CreateComponent, CreateEmbed,
        CreateInteractionResponse, CreateInteractionResponseMessage,
    },
};
use snafu::ResultExt as _;

/// The 5 button tags visible on the read-only browse embed: page navigation,
/// version navigation, and rollback.
const BROWSE_BUTTONS: [&str; 5] = [
    InteractionKind::Previous.as_tag(),
    InteractionKind::Next.as_tag(),
    InteractionKind::OlderVersion.as_tag(),
    InteractionKind::NewerVersion.as_tag(),
    InteractionKind::Rollback.as_tag(),
];

/// Builds the read-only view paginator for an already-fetched character list,
/// or sends the "no character" notice when the list is empty.
pub async fn browse(ctx: Context<'_>, characters: Vec<Character>) -> AppResult {
    let Some(nonempty) = NonEmpty::from_vec(characters) else {
        return notify_no_character(ctx).await;
    };
    Box::pin(display_browse(ctx, nonempty)).await
}

/// Sends the view embed and loops on button presses, flipping between search
/// results with the page buttons, walking the shown character's version chain
/// with the version buttons, and rolling it back to the shown older version with
/// the rollback button.
async fn display_browse(ctx: Context<'_>, characters: NonEmpty<Character>) -> AppResult {
    let pages = characters.len();
    let mut page: usize = 0;
    let mut chain = load_version_chain(ctx, characters.first()).await?;
    let mut version = chain.len().saturating_sub(1);

    send_browse_message(ctx, &characters, &chain, version, page).await?;

    while let Some(interaction) = create_collector(ctx, &BROWSE_BUTTONS).await {
        match Interaction::try_from(&interaction)?.kind {
            InteractionKind::Previous => {
                page = wrapping_previous(page, pages);
                chain = load_version_chain(ctx, page_character(&characters, page)).await?;
                version = chain.len().saturating_sub(1);
            }
            InteractionKind::Next => {
                page = page.saturating_add(1).strict_rem(pages);
                chain = load_version_chain(ctx, page_character(&characters, page)).await?;
                version = chain.len().saturating_sub(1);
            }
            InteractionKind::OlderVersion => {
                version = version.saturating_sub(1);
            }
            InteractionKind::NewerVersion => {
                version = version.saturating_add(1).min(chain.len().saturating_sub(1));
            }
            InteractionKind::Rollback => {
                return roll_back_to_shown(ctx, interaction, &chain, version).await;
            }
            _ => {} // no more kinds possible on this type of message
        }

        let (embed, buttons) = browse_components(ctx, &characters, &chain, version, page).await;
        interaction
            .create_response(
                ctx.http(),
                CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new()
                        .embed(embed)
                        .components(buttons),
                ),
            )
            .await
            .context(SendResponseSnafu)?;
    }

    Ok(())
}

/// Sends the initial view paginator message (a new ephemeral reply).
async fn send_browse_message(
    ctx: Context<'_>,
    characters: &NonEmpty<Character>,
    chain: &[Character],
    version: usize,
    page: usize,
) -> AppResult {
    let (embed, buttons) = browse_components(ctx, characters, chain, version, page).await;
    ctx.send(
        CreateReply::default()
            .embed(embed)
            .components(buttons)
            .ephemeral(true),
    )
    .await
    .context(SendMessageSnafu)?;
    Ok(())
}

/// Builds the embed and button rows for the version shown on the current page.
async fn browse_components(
    ctx: Context<'_>,
    characters: &NonEmpty<Character>,
    chain: &[Character],
    version: usize,
    page: usize,
) -> (
    CreateEmbed<'static>,
    Cow<'static, [CreateComponent<'static>]>,
) {
    let embed = browse_embed(ctx, characters, chain, version, page).await;
    let buttons = browse_buttons(ctx.id(), characters.len() < 2, version, chain.len());
    (embed, buttons)
}

/// Returns the search result shown on the given page, falling back to the first.
fn page_character(characters: &NonEmpty<Character>, page: usize) -> &Character {
    characters.get(page).unwrap_or_else(|| characters.first())
}

/// Loads the version chain (oldest to newest) of a character, falling back to the
/// character itself when the chain cannot be resolved.
async fn load_version_chain(ctx: Context<'_>, character: &Character) -> AppResult<Vec<Character>> {
    let chain = ctx.data().db.character_versions(character.id()).await?;
    Ok(if chain.is_empty() {
        vec![character.clone()]
    } else {
        chain
    })
}

/// Builds the view embed for the version shown on the current page, with a footer
/// carrying the page counter, the head's conversation count, the search-result
/// similarity, and the version counter.
async fn browse_embed(
    ctx: Context<'_>,
    characters: &NonEmpty<Character>,
    chain: &[Character],
    version: usize,
    page: usize,
) -> CreateEmbed<'static> {
    let result = page_character(characters, page);
    let head = chain.last().unwrap_or(result);
    let shown = chain.get(version).unwrap_or(result).clone();
    let footer = format!(
        "{}/{} | {} konversationer{} | version {}/{}",
        page.saturating_add(1),
        characters.len(),
        head.conversations_had(),
        result.similarity(),
        version.saturating_add(1),
        chain.len(),
    );
    character_embed(&shown, footer, &ctx.data().db).await
}

/// Rolls the shown character back to the older version currently on screen, then
/// notifies the user and clears the message 5 seconds later. A no-op when the
/// chain endpoints cannot be resolved.
async fn roll_back_to_shown(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    chain: &[Character],
    version: usize,
) -> AppResult {
    let (Some(head), Some(target)) = (chain.last(), chain.get(version)) else {
        return Ok(());
    };
    ctx.data()
        .db
        .rollback_character(head.id(), target.id(), ctx.author().id)
        .await?;

    respond_then_clear(ctx, interaction, rolled_back()).await
}

/// Returns the action rows for the view paginator: a page-navigation row and a
/// version-navigation row. The version buttons are disabled at the chain's ends,
/// and rollback is disabled while the newest version is shown.
#[must_use]
fn browse_buttons(
    id: u64,
    page_disabled: bool,
    version: usize,
    chain_len: usize,
) -> Cow<'static, [CreateComponent<'static>]> {
    let at_oldest = version == 0;
    let at_head = version.saturating_add(1) >= chain_len;
    let page_row = CreateActionRow::Buttons(
        vec![
            emoji_button(InteractionKind::Previous.custom_id(id), PREVIOUS).disabled(page_disabled),
            emoji_button(InteractionKind::Next.custom_id(id), NEXT).disabled(page_disabled),
        ]
        .into(),
    );
    let version_row = CreateActionRow::Buttons(
        vec![
            emoji_button(InteractionKind::OlderVersion.custom_id(id), OLDER_VERSION)
                .disabled(at_oldest),
            emoji_button(InteractionKind::NewerVersion.custom_id(id), NEWER_VERSION)
                .disabled(at_head),
            emoji_button(InteractionKind::Rollback.custom_id(id), ROLLBACK).disabled(at_head),
        ]
        .into(),
    );
    vec![
        CreateComponent::ActionRow(page_row),
        CreateComponent::ActionRow(version_row),
    ]
    .into()
}
