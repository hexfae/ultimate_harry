//! The event handler and the Discord events it responds to.

pub mod interaction;
pub mod lookup;
pub mod message;
pub mod ready;
pub mod streaming;

use crate::{
    app_state::AppState, error::AppError, error_display::error_reply, util::report_error,
};
use alloc::sync::Arc;
use miette::{IntoDiagnostic as _, Result};
use poise::{FrameworkError, builtins};
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
        match event {
            FullEvent::Ready { data_about_bot, .. } => {
                if let Err(why) = ready::ready(ctx, data_about_bot).await {
                    error!("in ready handler");
                    report_error(why);
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
                    if let Err(why) =
                        message::message(&reply_ctx, &user_message, &state.db, &state.cancellations)
                            .await
                    {
                        error!(
                            message_id = %user_message.id,
                            channel_id = %user_message.channel_id,
                            user_id = %user_message.author.id,
                            "in message handler"
                        );
                        report_error(why);
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
                    if let Err(why) = interaction::component(
                        &reply_ctx,
                        &pressed,
                        &state.db,
                        &state.cancellations,
                    )
                    .await
                    {
                        error!(
                            custom_id = %pressed.data.custom_id,
                            message_id = %pressed.message.id,
                            channel_id = %pressed.message.channel_id,
                            user_id = %pressed.user.id,
                            "in interaction handler"
                        );
                        report_error(why);
                    }
                });
            }
            _ => {}
        }
    }
}

/// The bot's error handler.
///
/// Logs the full diagnostic for developers but shows the user only a clean
/// Swedish error notice (the same red error container as the chat surface),
/// never the raw diagnostic dump, and keeps the user-reachable framework errors
/// in Swedish rather than poise's English built-in messages.
pub async fn on_error(framework_error: FrameworkError<'_, AppState, AppError>) -> Result<()> {
    match framework_error {
        FrameworkError::Command { error, ctx, .. } => {
            let notice = error.user_message();
            error!(
                command = %ctx.command().qualified_name,
                user_id = %ctx.author().id,
                "in command"
            );
            report_error(error);
            send_error(ctx, notice).await?;
        }
        other => report_framework_error(other).await?,
    }
    Ok(())
}

/// Shows the user the red error container ephemerally for a failed command.
async fn send_error(ctx: crate::Context<'_>, message: String) -> Result<()> {
    ctx.send(error_reply(message)).await.into_diagnostic()?;
    Ok(())
}

/// Handles the non-command framework errors, surfacing the user-reachable ones
/// (a missing subcommand, an unparseable argument, a command panic) as Swedish
/// notices and leaving the rest to poise's built-in handler.
#[expect(
    clippy::cognitive_complexity,
    reason = "a flat dispatch over framework-error variants whose tracing macros the lint overcounts"
)]
async fn report_framework_error(
    framework_error: FrameworkError<'_, AppState, AppError>,
) -> Result<()> {
    match framework_error {
        FrameworkError::SubcommandRequired { ctx } => {
            send_error(ctx, "Du måste välja ett underkommando.".to_owned()).await?;
        }
        FrameworkError::ArgumentParse {
            error, input, ctx, ..
        } => {
            error!(
                command = %ctx.command().qualified_name,
                "could not parse an argument: {error} (input: {input:?})"
            );
            send_error(
                ctx,
                "Ogiltigt argument, kontrollera värdet och försök igen.".to_owned(),
            )
            .await?;
        }
        FrameworkError::CommandPanic { payload, ctx, .. } => {
            error!(
                command = %ctx.command().qualified_name,
                "command panicked: {payload:?}"
            );
            send_error(ctx, "Ett internt fel inträffade.".to_owned()).await?;
        }
        other => {
            if let Err(why) = builtins::on_error(other).await {
                error!("while handling a framework error: {why}");
            }
        }
    }
    Ok(())
}
