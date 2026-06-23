//! The event handler and the Discord events it responds to.

pub mod interaction;
pub mod lookup;
pub mod message;
pub mod ready;
pub mod streaming;

use crate::{
    app_state::AppState, error::AppError, traits::SayEphemeral as _, util::render_diagnostic,
};
use alloc::sync::Arc;
use miette::{IntoDiagnostic as _, Report, Result};
use poise::{FrameworkError, builtins};
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
        match event {
            FullEvent::Ready { data_about_bot, .. } => {
                if let Err(why) = ready::ready(ctx, data_about_bot).await {
                    error!("in ready handler:\n{}", render_diagnostic(why));
                }
            }
            // chat replies and interactions stream an LLM response and only
            // persist the resulting `History` once finished, so run them under
            // the tracker and let the shutdown drain wait for them.
            FullEvent::Message { new_message, .. } => {
                let reply_ctx = ctx.clone();
                let user_message = new_message.clone();
                data.tasks.clone().spawn(async move {
                    let state: Arc<AppState> = reply_ctx.data();
                    if let Err(why) = message::message(&reply_ctx, &user_message, &state.db).await {
                        error!(
                            message_id = %user_message.id,
                            channel_id = %user_message.channel_id,
                            user_id = %user_message.author.id,
                            "in message handler:\n{}",
                            render_diagnostic(why)
                        );
                    }
                });
            }
            FullEvent::InteractionCreate {
                interaction: Interaction::Component(component),
                ..
            } => {
                let reply_ctx = ctx.clone();
                let pressed = component.clone();
                data.tasks.clone().spawn(async move {
                    let state: Arc<AppState> = reply_ctx.data();
                    if let Err(why) = interaction::component(&reply_ctx, &pressed, &state.db).await {
                        error!(
                            custom_id = %pressed.data.custom_id,
                            message_id = %pressed.message.id,
                            channel_id = %pressed.message.channel_id,
                            user_id = %pressed.user.id,
                            "in interaction handler:\n{}",
                            render_diagnostic(why)
                        );
                    }
                });
            }
            _ => {}
        }
    }
}

/// The bot's error handler.
#[expect(
    clippy::print_stderr,
    reason = "color printing is broken when logging with tracing"
)]
pub async fn on_error(framework_error: FrameworkError<'_, AppState, AppError>) -> Result<()> {
    match framework_error {
        FrameworkError::Command { error, ctx, .. } => {
            let report = format!("{:?}", Report::from(error));
            error!(
                command = %ctx.command().qualified_name,
                user_id = %ctx.author().id,
                "in command"
            );
            eprintln!("{report}");
            ctx.say_ephemeral(format!("```\n{}```", strip_str(report)))
                .await
                .into_diagnostic()?;
        }
        other => {
            if let Err(why) = builtins::on_error(other).await {
                error!("while handling a framework error: {why}");
            }
        }
    }
    Ok(())
}
