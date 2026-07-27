//! The different interaction events that can happen on a chat message.

mod character;
mod codec;
mod continue_reply;
mod edit;
mod next;
mod pin;
mod speak;
mod stop;
mod swipe;
mod tts;
mod voice;

use crate::{
    AppResult,
    cancellation::Cancellations,
    database::Database,
    error::{AppError, SendResponseSnafu},
    error_display::{error_followup, error_response},
    events::lookup::history_and_character_of,
    models::history::History,
    util::report_error,
};
use poise::serenity_prelude::{ComponentInteraction, Context};
use snafu::ResultExt as _;
use tracing::warn;

#[expect(
    clippy::module_name_repetitions,
    reason = "these are the app-wide public names for the interaction codec; the module prefix disambiguates them at use sites"
)]
pub use codec::{Interaction, InteractionKind, UnknownInteraction};

use character::character as character_fn;
use continue_reply::continue_reply;
use edit::edit;
use next::next;
use pin::pin;
use stop::stop;
use swipe::swipe;
use tts::tts;
use voice::voice;

/// Handle the interaction based on which button was pressed, surfacing any
/// failure to the user as an ephemeral error notice before returning it to be
/// logged, so a button press never silently does nothing.
pub async fn component(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    cancellations: &Cancellations,
) -> AppResult {
    let result = dispatch(ctx, interaction, db, cancellations).await;
    if let Err(ref why) = result {
        report_failure(ctx, interaction, why).await;
    }
    result
}

/// Surfaces a failed interaction to the user as an ephemeral error notice.
///
/// An unacknowledged interaction (the common case: a failure before any
/// response) needs its initial response within Discord's 3-second window, so that
/// is tried first; an already-acknowledged interaction (a failure after a
/// placeholder was posted) rejects a second response, so it falls back to a
/// followup. Logs if neither can be sent.
async fn report_failure(ctx: &Context, interaction: &ComponentInteraction, why: &AppError) {
    let message = why.user_message();
    if interaction
        .create_response(&ctx.http, error_response(message.clone()))
        .await
        .is_ok()
    {
        return;
    }
    if let Err(report_why) = interaction
        .create_followup(&ctx.http, error_followup(message))
        .await
        .context(SendResponseSnafu)
    {
        warn!(
            custom_id = %interaction.data.custom_id,
            "failed to show the error notice"
        );
        report_error(report_why);
    }
}

/// Parses the pressed component and dispatches to the matching button handler.
async fn dispatch(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    cancellations: &Cancellations,
) -> AppResult {
    let Some(ultimate_interaction) = route(&interaction.data.custom_id) else {
        return Ok(());
    };

    let (id, kind) = (ultimate_interaction.id, ultimate_interaction.kind);

    if matches!(kind, InteractionKind::Confirm | InteractionKind::Cancel) {
        return Ok(());
    }

    // stopping needs no history load: it just cancels the in-flight stream, which
    // re-renders the frozen reply itself.
    if kind == InteractionKind::Stop {
        return stop(ctx, interaction, id, cancellations).await;
    }

    let Some((history, character)) = history_and_character_of(id, db).await? else {
        return Err(UnknownInteraction::stale(&interaction.data.custom_id).into());
    };

    match kind {
        InteractionKind::Previous => {
            swipe(
                ctx,
                interaction,
                id,
                db,
                history,
                character,
                History::previous,
            )
            .await?;
        }
        InteractionKind::Next => {
            next(ctx, interaction, id, db, history, character, cancellations).await?;
        }
        InteractionKind::Continue => {
            continue_reply(ctx, interaction, id, db, history, character, cancellations).await?;
        }
        InteractionKind::Edit => edit(ctx, interaction, id, db, history, character).await?,
        InteractionKind::Undo => {
            swipe(ctx, interaction, id, db, history, character, History::undo).await?;
        }
        InteractionKind::Redo => {
            swipe(ctx, interaction, id, db, history, character, History::redo).await?;
        }
        InteractionKind::Pin => pin(ctx, interaction, db, history, character).await?,
        InteractionKind::Tts => tts(ctx, interaction, db, history, character).await?,
        InteractionKind::Voice => voice(ctx, interaction, db, history, character).await?,
        InteractionKind::Character => {
            character_fn(ctx, interaction, db, history, cancellations).await?;
        }
        // stop is handled above, before the history load; the rest only ever fire
        // on the ephemeral /gubbe visa paginator, collected by that command's own
        // collector, never on a chat message's history
        InteractionKind::Stop
        | InteractionKind::Confirm
        | InteractionKind::Cancel
        | InteractionKind::OlderVersion
        | InteractionKind::NewerVersion
        | InteractionKind::Rollback => {}
    }
    Ok(())
}

/// Routes a pressed component by its `custom_id` before any history load.
///
/// `Some(_)` is one of Ultimate Harry's persistent message buttons
/// (`<message_id><tag>`), to be dispatched to the matching handler. `None` is a
/// button owned by a command's own ephemeral `ComponentInteractionCollector`
/// (keyed on a bare interaction id, no tag): the global handler must leave it
/// untouched so the collector can claim it, rather than acknowledging it out from
/// under the command and breaking the two-modal character create/edit flow.
fn route(custom_id: &str) -> Option<Interaction> {
    Interaction::parse(custom_id).ok()
}

/// Tests for routing a pressed component to dispatch or to a command's collector.
#[cfg(test)]
mod tests {
    use super::route;

    /// A command's own collector button (the two-modal "tempting button", keyed
    /// on a bare interaction id with no tag) must route to `None`, so the
    /// global handler leaves it for the collector instead of acknowledging it as
    /// an unknown interaction (which broke the character create/edit flow).
    #[test]
    fn collector_owned_custom_id_is_left_for_its_collector() {
        let tempting_button = "1519649606525386782";
        assert!(
            route(tempting_button).is_none(),
            "a bare interaction id belongs to a command's collector and must be ignored, not errored"
        );
    }

    /// A persistent message button (`<message_id><tag>`) still routes to its
    /// interaction, so the global handler dispatches it.
    #[test]
    fn message_button_custom_id_is_dispatched() {
        assert!(
            route("123456prev").is_some(),
            "a well-formed message-button custom_id is ours to dispatch"
        );
    }
}
