//! Shared character paginators for the view, edit, delete, and restore commands.
//!
//! The edit, delete, and restore commands run a search, render the matching
//! characters as a swipeable embed with Previous/Next page buttons plus a
//! Confirm button, and act on the page the user confirms. The view command uses
//! the separate [`browse`] paginator, which additionally walks each character's
//! version chain and can roll a character back to an older version.

use crate::{
    AppResult, Context,
    commands::character::render::character_embed,
    commands::notify_no_character,
    components::{confirm_interaction_response, emoji_button},
    constants::{
        CANCEL, NEWER_VERSION, NEXT, OLDER_VERSION, PREVIOUS, ROLLBACK, TRANSIENT_LINGER,
    },
    error::{DeleteResponseSnafu, SendMessageSnafu, SendResponseSnafu},
    events::interaction::{Interaction, InteractionKind},
    models::character::Character,
    phrases::{cancelled, rolled_back},
    traits::RespondToWith as _,
    util::wrapping_previous,
};
use alloc::borrow::Cow;
use nonempty::NonEmpty;
use poise::{
    CreateReply, ReplyHandle,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, CreateActionRow, CreateComponent,
        CreateEmbed, CreateInteractionResponse, CreateInteractionResponseMessage,
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

/// The 5 button tags visible on the read-only [`browse`] embed: page navigation,
/// version navigation, and rollback.
const BROWSE_BUTTONS: [&str; 5] = [
    InteractionKind::Previous.as_tag(),
    InteractionKind::Next.as_tag(),
    InteractionKind::OlderVersion.as_tag(),
    InteractionKind::NewerVersion.as_tag(),
    InteractionKind::Rollback.as_tag(),
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
    paginate_characters(ctx, characters, action_emoji, on_confirm).await
}

/// Like [`paginate`], but searches only the soft-deleted characters, used by the
/// restore command to bring a deleted character back from deletion.
#[expect(
    clippy::module_name_repetitions,
    reason = "this is the deleted-character variant of paginate, so the shared prefix is meaningful"
)]
pub async fn paginate_deleted<'a, F, Fut>(
    ctx: Context<'a>,
    name: String,
    action_emoji: &'static str,
    on_confirm: F,
) -> AppResult
where
    F: FnOnce(Context<'a>, ComponentInteraction, Character) -> Fut,
    Fut: Future<Output = AppResult>,
{
    let characters: Vec<Character> = ctx.data().db.deleted_characters_by_similarity(name).await?;
    paginate_characters(ctx, characters, action_emoji, on_confirm).await
}

/// Builds the paginated embeds for an already-fetched character list and drives
/// the swipe/cancel/confirm UI.
async fn paginate_characters<'a, F, Fut>(
    ctx: Context<'a>,
    characters: Vec<Character>,
    action_emoji: &'static str,
    on_confirm: F,
) -> AppResult
where
    F: FnOnce(Context<'a>, ComponentInteraction, Character) -> Fut,
    Fut: Future<Output = AppResult>,
{
    let Some(pages) = build_pages(ctx, characters).await? else {
        return Ok(());
    };
    Box::pin(display_pagination(ctx, pages, action_emoji, on_confirm)).await
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
        notify_no_character(ctx).await?;
        Ok(None)
    }
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
    let mut pending_confirm = Some(on_confirm);

    send_initial_embed(
        ctx,
        characters_and_footers.first().to_owned(),
        action_emoji,
        pages < 2,
    )
    .await?;

    while let Some(interaction) = create_collector(ctx, &BUTTONS).await {
        match Interaction::try_from(&interaction)?.kind {
            InteractionKind::Previous => {
                current_page = wrapping_previous(current_page, pages);
            }
            InteractionKind::Next => {
                current_page = current_page.saturating_add(1).strict_rem(pages);
            }
            InteractionKind::Cancel => {
                return notify_cancelled(ctx, interaction).await;
            }
            InteractionKind::Confirm => {
                if let Some(confirm) = pending_confirm.take() {
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
        let embed = character_embed(&character, footer_text, &ctx.data().db).await;

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
                version = version
                    .saturating_add(1)
                    .min(chain.len().saturating_sub(1));
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
) -> (CreateEmbed<'static>, Cow<'static, [CreateComponent<'static>]>) {
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
async fn load_version_chain(
    ctx: Context<'_>,
    character: &Character,
) -> AppResult<Vec<Character>> {
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

/// Creates a collector that listens for any of `tags` keyed on the context's ID.
/// The paginator message is ephemeral, so only the command author can see and
/// press these buttons; no author filter is needed.
#[must_use]
async fn create_collector(ctx: Context<'_>, tags: &[&str]) -> Option<ComponentInteraction> {
    let custom_ids = FixedArray::from_vec_trunc(
        tags.iter()
            .map(|suffix| FixedString::from_string_trunc(format!("{}{suffix}", ctx.id())))
            .collect(),
    );
    ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(custom_ids)
        .await
}

/// Sends the first message, containing the embed of a character and the
/// Confirm/Cancel/Previous/Next buttons.
async fn send_initial_embed<'a>(
    ctx: Context<'a>,
    (character, footer_text): (Character, String),
    action_emoji: &'static str,
    nav_disabled: bool,
) -> AppResult<ReplyHandle<'a>> {
    let id = ctx.id();
    let embed = character_embed(&character, footer_text, &ctx.data().db).await;
    let buttons = create_buttons(id, action_emoji, nav_disabled);
    ctx.send(
        CreateReply::default()
            .embed(embed)
            .components(buttons)
            .ephemeral(true),
    )
    .await
    .context(SendMessageSnafu)
}

/// Returns the action row for the confirm paginator: Confirm/Cancel followed by
/// Previous/Next. `nav_disabled` greys out the Previous/Next buttons (used on
/// single-page results).
#[must_use]
fn create_buttons(
    id: u64,
    action_emoji: &'static str,
    nav_disabled: bool,
) -> Cow<'static, [CreateComponent<'static>]> {
    let buttons = vec![
        emoji_button(InteractionKind::Confirm.custom_id(id), action_emoji),
        emoji_button(InteractionKind::Cancel.custom_id(id), CANCEL),
        emoji_button(InteractionKind::Previous.custom_id(id), PREVIOUS).disabled(nav_disabled),
        emoji_button(InteractionKind::Next.custom_id(id), NEXT).disabled(nav_disabled),
    ];
    vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        buttons.into(),
    ))]
    .into()
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

/// Sends a cancellation message in response to `interaction`, then deletes it 5 seconds later.
pub async fn notify_cancelled(ctx: Context<'_>, interaction: ComponentInteraction) -> AppResult {
    respond_then_clear(ctx, interaction, cancelled()).await
}

/// Responds to `interaction` with `text`, then deletes the response after a
/// short linger. The shared "transient interaction reply" used by the
/// confirm/cancel and rollback flows.
pub async fn respond_then_clear<T: AsRef<str>>(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    text: T,
) -> AppResult {
    ctx.respond_to_with(&interaction, text)
        .await
        .context(SendResponseSnafu)?;
    sleep(TRANSIENT_LINGER).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
}

/// Shows a confirm/cancel prompt on `interaction`, waits for the press, and
/// returns the response when confirmed. On an explicit cancel it shows the
/// cancellation notice itself; on cancel or no response it returns `None`.
pub async fn confirm_prompt<P: Into<String>>(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    prompt: P,
) -> AppResult<Option<ComponentInteraction>> {
    let id = ctx.id();
    let confirm_id = InteractionKind::Confirm.custom_id(id);
    interaction
        .create_response(ctx.http(), confirm_interaction_response(id, prompt.into()))
        .await
        .context(SendResponseSnafu)?;
    let Some(response) = create_collector(
        ctx,
        &[
            InteractionKind::Confirm.as_tag(),
            InteractionKind::Cancel.as_tag(),
        ],
    )
    .await
    else {
        return Ok(None);
    };
    if response.data.custom_id == confirm_id {
        Ok(Some(response))
    } else {
        notify_cancelled(ctx, response).await?;
        Ok(None)
    }
}
