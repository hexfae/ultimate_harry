//! The event handler and the Discord events it responds to.

pub mod interaction;
pub mod message;
pub mod ready;
pub mod streaming;

use crate::{app_state::AppState, error::AppError};
use alloc::sync::Arc;
use miette::{IntoDiagnostic as _, Report, Result};
use poise::FrameworkError;
use serenity::{
    all::{Context, EventHandler as EventHandlerTrait, FullEvent, Interaction},
    async_trait,
};
use strip_ansi_escapes::strip_str;
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
pub async fn on_error(framework_error: FrameworkError<'_, AppState, AppError>) -> Result<()> {
    if let FrameworkError::Command { error, ctx, .. } = framework_error {
        let report = format!("{:?}", Report::from(error));
        error!("in command");
        eprintln!("{report}");
        ctx.say(format!("```\n{}```", strip_str(report)))
            .await
            .into_diagnostic()?;
    }
    Ok(())
}
