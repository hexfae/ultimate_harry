//! The button that stops an in-flight reply mid-stream.

use crate::{AppResult, cancellation::Cancellations, error::SendResponseSnafu};
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, MessageId,
};
use snafu::ResultExt as _;

/// Stop the in-flight reply on this message, keeping whatever has streamed so far.
///
/// Cancels the stream's token (a no-op if it already finished) and acknowledges
/// the press without changing the message; the streaming task re-renders the
/// frozen reply with its buttons live on its next tick and at finalize.
pub async fn stop(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    cancellations: &Cancellations,
) -> AppResult {
    cancellations.cancel(id);
    interaction
        .create_response(&ctx.http, CreateInteractionResponse::Acknowledge)
        .await
        .context(SendResponseSnafu)?;
    Ok(())
}
