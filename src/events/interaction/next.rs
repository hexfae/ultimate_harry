//! The button that shows the next reply of a message or generates a new one.

use super::swipe::swipe;
use crate::{
    AppResult,
    database::Database,
    error::SendResponseSnafu,
    events::streaming::{InteractionSink, stream_and_finalize},
    models::{character::Character, history::History},
};
use core::time::Duration;
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::ResultExt as _;

/// Show the next reply to this message or generate a new one.
pub async fn next(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
    mut history: History,
    character: Character,
) -> AppResult {
    if history.is_on_last_choice() {
        let options = db.character_menu_options().await?;
        history.push_choice((character.clone(), String::new(), Duration::ZERO));
        history.set_finished(false);

        let placeholder = history.to_placeholder_interaction(&character, &options);
        interaction
            .create_response(&ctx.http, placeholder)
            .await
            .context(SendResponseSnafu)?;

        let sink = InteractionSink {
            ctx,
            history: &mut history,
            character: &character,
            interaction,
            id,
            db,
            options: &options,
        };
        stream_and_finalize(None, sink).await?;
    } else {
        swipe(ctx, interaction, id, db, history, character, History::next).await?;
    }

    Ok(())
}
