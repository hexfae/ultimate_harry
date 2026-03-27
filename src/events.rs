//! The event handler and the Discord events it responds to.

mod interaction;
mod message;
mod ready;

use crate::app_state::AppState;
use alloc::sync::Arc;
use miette::{IntoDiagnostic as _, Report, Result};
use poise::FrameworkError;
use serenity::{
    all::{Context, EventHandler as EventHandlerTrait, FullEvent, Interaction},
    async_trait,
};
use tracing::error;

/// The bot's event handler.
pub struct EventHandler;

#[async_trait]
impl EventHandlerTrait for EventHandler {
    async fn dispatch(&self, ctx: &Context, event: &FullEvent) {
        let data: Arc<AppState> = ctx.data();
        drop(match event {
            FullEvent::Ready { data_about_bot, .. } => ready::ready(ctx, data_about_bot).await,
            FullEvent::Message { new_message, .. } => {
                message::message(ctx, new_message, &data.db).await
            }
            FullEvent::InteractionCreate { interaction, .. } => {
                if let Interaction::Component(component) = interaction {
                    interaction::component(ctx, component, &data.db).await
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        });
    }
}

/// The bot's error handler.
#[expect(
    clippy::print_stderr,
    reason = "color printing is broken when logging with tracing"
)]
#[expect(
    clippy::use_debug,
    reason = "debug printing is necessary for fancy miette diagnostics"
)]
pub async fn on_error(framework_error: FrameworkError<'_, AppState, Report>) -> Result<()> {
    if let FrameworkError::Command { error, ctx, .. } = framework_error {
        error!("in command: {error}");
        eprintln!("{error:?}");
        ctx.say(error.to_string()).await.into_diagnostic()?;
    }
    Ok(())
}
