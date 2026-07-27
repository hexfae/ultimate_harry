//! The button that continues a finished reply, extending it in place.

use crate::{
    AppResult,
    cancellation::Cancellations,
    database::Database,
    error::SendResponseSnafu,
    events::streaming::{ContinueSink, InteractionSink, stream_and_finalize},
    models::{character::Character, history::History},
};
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::ResultExt as _;

/// The instruction handed to the model when continuing a reply.
const CONTINUE_PROMPT: &str = "Fortsätt ditt föregående svar precis där det slutade. Skriv enbart fortsättningen, och upprepa inte det du redan har skrivit.";

/// Continue this finished reply, streaming more text appended onto it in place.
///
/// Re-renders the message with the existing reply and a live Stop, then streams a
/// continuation through a [`ContinueSink`] (which seeds the model with the reply
/// so far and appends the new tokens to it), reusing the shared cancel/finalize
/// machinery so Stop halts the continuation just like a first reply.
pub async fn continue_reply(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
    mut history: History,
    character: Character,
    cancellations: &Cancellations,
) -> AppResult {
    let seed = history.chosen_content().to_owned();
    let options = db.character_menu_options().await?;
    let voices = db.voice_options().await;
    history.set_finished(false);

    let response = history
        .to_interaction(&character, id, db, &options, &voices)
        .await;
    interaction
        .create_response(&ctx.http, response)
        .await
        .context(SendResponseSnafu)?;

    let sink = ContinueSink {
        inner: InteractionSink {
            ctx,
            history: &mut history,
            character: &character,
            interaction,
            id,
            db,
            options: &options,
            voices: &voices,
        },
        seed,
    };
    stream_and_finalize(Some(CONTINUE_PROMPT.to_owned()), cancellations, sink).await
}
