//! The button that shows the next reply of a message or generates a new one.

use crate::{
    constants::CHARACTER_LIMIT, database::Database, events::message::history_and_character_of,
    llm::LlmManager,
};
use core::time::Duration;
use miette::{Diagnostic, Result};
use poise::serenity_prelude::{
    ComponentInteraction, Context, CreateInteractionResponse, MessageId,
};
use rig::{
    agent::{MultiTurnStreamItem, StreamingError},
    streaming::StreamedAssistantContent,
};
use serenity::futures::StreamExt as _;
use snafu::{ResultExt as _, Snafu};
use std::time::Instant;

/// Show the next reply to this message or a generate a new one.
pub async fn next(
    ctx: &Context,
    interaction: &ComponentInteraction,
    id: MessageId,
    db: &Database,
) -> Result<()> {
    let Some((mut history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    if history.is_on_last_choice() {
        history.has_finished(false);

        let placeholder = CreateInteractionResponse::UpdateMessage(
            history.to_placeholder_interaction(&character),
        );

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
        let mut time_since_last_edit = now;
        let mut stream = requester.request_stream(&history, None).await?;

        while let Some(result) = stream.next().await {
            if let MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(delta)) =
                result.context(StreamingSnafu)?
            {
                if total.len() >= CHARACTER_LIMIT {
                    break;
                }
                total += delta.text();
                if time_since_last_edit.elapsed() >= Duration::from_secs(1) {
                    time_since_last_edit = Instant::now();
                    history.set_choices((character.clone(), total.clone(), now.elapsed()));
                    let edit = history.to_edit_interaction(&character, id, db).await;

                    interaction
                        .edit_response(&ctx.http, edit)
                        .await
                        .context(EditResponseSnafu)?;
                }
            }
        }

        // current choice is set in this function
        history.push_choice((character.clone(), total, now.elapsed()));
        history.has_finished(true);

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

/// All errors that can happen when showing the next reply.
#[derive(Debug, Snafu, Diagnostic)]
enum NextReplyError {
    /// Sending a response failed.
    #[snafu(display("Kunde inte skicka interaktionssvar: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att interaktionen fortfarande är giltig"),
        code(events::interaction::next::send_response)
    )]
    SendResponse {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Editing a response failed.
    #[snafu(display("Kunde inte redigera interaktionssvar: {source}"))]
    #[diagnostic(
        help("Detta kan bero på att interaktionen har gått ut"),
        code(events::interaction::next::edit_response)
    )]
    EditResponse {
        ///The source of the error.
        source: serenity::Error,
    },
    /// Streaming the AI model response failed.
    #[snafu(display("Strömning misslyckades: {source}"))]
    #[diagnostic(
        help(
            "Försök igen eller kontrollera att modellen är tillgänglig och att nätverket fungerar"
        ),
        code(events::interaction::next::streaming)
    )]
    Streaming {
        ///The source of the error.
        source: StreamingError,
    },
}
