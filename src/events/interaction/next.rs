//! The button that shows the next reply of a message or generates a new one.

use crate::{
    AppResult,
    database::Database,
    error::SendResponseSnafu,
    events::{
        message::history_and_character_of,
        streaming::{InteractionSink, stream_into},
    },
    llm::LlmManager,
};
use core::time::Duration;
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::ResultExt as _;
use std::time::Instant;

/// Show the next reply to this message or a generate a new one.
pub async fn next(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> AppResult {
    let Some((mut history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    if history.is_on_last_choice() {
        history.push_choice((character.clone(), String::new(), Duration::ZERO));
        history.set_finished(false);

        let placeholder = history.to_placeholder_interaction(&character, db).await;
        interaction
            .create_response(&ctx.http, placeholder)
            .await
            .context(SendResponseSnafu)?;

        let requester = LlmManager::new(db.resolved_model_settings(&character).await);

        let now = Instant::now();
        let context = db
            .build_context(&history, &character, interaction.user.id)
            .await?;

        let mut sink = InteractionSink {
            ctx,
            history: &mut history,
            character: &character,
            interaction,
            id,
            db,
        };
        let total = stream_into(&requester, &context, None, now, &mut sink).await?;

        // update the pre-allocated choice with the final content.
        history.update_current_choice((character.clone(), total, now.elapsed()));
        history.set_finished(true);

        let response = history.to_edit_interaction(&character, id, db).await;

        interaction
            .edit_response(&ctx.http, response)
            .await
            .context(SendResponseSnafu)?;
    } else {
        history.next();

        let response = history.to_interaction(&character, id, db).await;

        interaction
            .create_response(&ctx.http, response)
            .await
            .context(SendResponseSnafu)?;
    }

    db.upsert_history(history).await?;

    Ok(())
}
