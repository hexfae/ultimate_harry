//! The button that shows the next reply of a message or generates a new one.

use crate::{
    AppResult,
    database::Database,
    error::SendResponseSnafu,
    events::streaming::{InteractionSink, ReplySink as _, stream_into},
    llm::LlmManager,
    models::{character::Character, history::History},
};
use core::time::Duration;
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::ResultExt as _;
use std::time::Instant;

/// Show the next reply to this message or generate a new one.
pub async fn next(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
    mut history: History,
    character: Character,
) -> AppResult {
    let options = db.character_menu_options().await?;

    if history.is_on_last_choice() {
        history.push_choice((character.clone(), String::new(), Duration::ZERO));
        history.set_finished(false);

        let placeholder = history.to_placeholder_interaction(&character, &options);
        interaction
            .create_response(&ctx.http, placeholder)
            .await
            .context(SendResponseSnafu)?;

        let requester = LlmManager::new(db.resolved_model_settings(&character).await);

        let now = Instant::now();
        let context = db.build_context(&history, &character).await?;

        let mut sink = InteractionSink {
            ctx,
            history: &mut history,
            character: &character,
            interaction,
            id,
            db,
            options: &options,
        };
        let reply = stream_into(&requester, &context, None, now, &mut sink).await?;
        sink.finalize(reply, now.elapsed()).await?;
    } else {
        history.next();

        let response = history.to_interaction(&character, id, db, &options).await;

        interaction
            .create_response(&ctx.http, response)
            .await
            .context(SendResponseSnafu)?;

        db.upsert_history(history).await?;
    }

    Ok(())
}
