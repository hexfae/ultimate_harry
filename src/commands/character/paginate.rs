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
    components::emoji_button,
    constants::{CANCEL, NEXT, PREVIOUS, TRANSIENT_LINGER},
    error::{DeleteResponseSnafu, SendMessageSnafu, SendResponseSnafu},
    events::interaction::{Interaction, InteractionKind},
    models::character::Character,
    phrases::{cancelled, no, yes},
    util::wrapping_previous,
};
use alloc::borrow::Cow;
use nonempty::NonEmpty;
use poise::{
    CreateReply, ReplyHandle,
    serenity_prelude::{
        ButtonStyle, ComponentInteraction, ComponentInteractionCollector, CreateActionRow,
        CreateButton, CreateComponent, CreateInteractionResponse,
        CreateInteractionResponseMessage,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt as _;
use tokio::time::sleep;

mod browse;

pub use browse::browse;

/// The 4 button tags visible on a Confirm/Cancel character paginator embed.
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

/// Creates a collector that listens for any of `tags` keyed on the context's ID.
/// The paginator message is ephemeral, so only the command author can see and
/// press these buttons; no author filter is needed.
#[must_use]
pub(super) async fn create_collector(
    ctx: Context<'_>,
    tags: &[&str],
) -> Option<ComponentInteraction> {
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
    interaction
        .create_response(
            ctx.http(),
            CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new()
                    .content(text.as_ref())
                    .embeds(vec![])
                    .components(vec![]),
            ),
        )
        .await
        .context(SendResponseSnafu)?;
    sleep(TRANSIENT_LINGER).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
}

/// Builds a confirmation update-message response with confirm/cancel buttons keyed on `id`.
fn confirm_interaction_response<I, C>(id: I, content: C) -> CreateInteractionResponse<'static>
where
    I: Into<u64>,
    C: Into<String>,
{
    let buttons = confirm_buttons(id);
    CreateInteractionResponse::UpdateMessage(
        CreateInteractionResponseMessage::new()
            .content(content.into())
            .components(buttons),
    )
}

/// Builds confirmation buttons with "confirm" and "cancel" actions keyed on `into_id`.
fn confirm_buttons(into_id: impl Into<u64>) -> Vec<CreateComponent<'static>> {
    let id: u64 = into_id.into();
    let confirm_id = InteractionKind::Confirm.custom_id(id);
    let cancel_id = InteractionKind::Cancel.custom_id(id);
    vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        vec![
            CreateButton::new(confirm_id)
                .style(ButtonStyle::Secondary)
                .label(yes()),
            CreateButton::new(cancel_id)
                .style(ButtonStyle::Secondary)
                .label(no()),
        ]
        .into(),
    ))]
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
