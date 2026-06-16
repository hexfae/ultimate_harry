//! The button that shows the next reply of a message or generates a new one.

use crate::{
    AppResult,
    constants::CHARACTER_LIMIT,
    database::Database,
    error::{EditResponseSnafu, SendResponseSnafu, StreamingSnafu},
    events::message::history_and_character_of,
    llm::LlmManager,
};
use core::time::Duration;
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use rig::{agent::MultiTurnStreamItem, streaming::StreamedAssistantContent};
use serenity::futures::StreamExt as _;
use snafu::ResultExt as _;
use std::time::Instant;
use tokio::time::{MissedTickBehavior, interval};

/// Show the next reply to this message or a generate a new one.
#[expect(
    clippy::cognitive_complexity,
    reason = "the streaming loop with empty-completion retry is a single cohesive flow"
)]
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

        let requester = LlmManager::new(
            character
                .model_settings()
                .unwrap_or(db.model_settings().await),
        );

        let mut total = String::new();
        let now = Instant::now();
        let context = db
            .build_context(&history, &character, interaction.user.id)
            .await?;

        // OpenRouter (or the upstream model) occasionally ends a stream having
        // produced zero tokens, which would otherwise leave the user with an
        // empty reply. Re-request a few times before giving up so a single empty
        // completion doesn't swallow the response.
        let max_attempts: u32 = 3;
        let mut attempt: u32 = 0;
        'attempts: loop {
            attempt = attempt.saturating_add(1);
            let mut stream = requester.request_stream(&context, None).await?;
            let mut interval = interval(Duration::from_secs(1));
            interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

            loop {
                tokio::select! {
                    result = stream.next() => {
                        match result.transpose().context(StreamingSnafu)? {
                            Some(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(delta))) => {
                                if total.len() >= CHARACTER_LIMIT {
                                    break 'attempts;
                                }
                                total += delta.text();
                            }
                            Some(_) => {},
                            None => break,
                        }
                    }
                    _ = interval.tick() => {
                        if total.is_empty() {
                            if now.elapsed() >= Duration::from_secs(30) {
                                total += "30 sekunder har gått utan ett svar. Jag ger upp.";
                                break 'attempts;
                            }
                            let placeholder_edit = history.to_placeholder_interaction_edit(&character, now.elapsed(), db).await;
                            interaction.edit_response(&ctx.http, placeholder_edit).await.context(EditResponseSnafu)?;
                        } else {
                            history.update_current_choice((character.clone(), total.clone(), now.elapsed()));
                            let edit = history.to_edit_interaction(&character, id, db).await;
                            interaction.edit_response(&ctx.http, edit).await.context(EditResponseSnafu)?;
                        }
                    }
                }
            }

            // the stream ended: keep a non-empty reply, otherwise retry until we
            // run out of attempts.
            if !total.is_empty() {
                break 'attempts;
            }
            if attempt >= max_attempts {
                total += "AI:n gav inget svar efter flera försök. Jag ger upp.";
                break 'attempts;
            }
        }

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
