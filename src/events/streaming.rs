//! The shared streaming loop behind every character reply.
//!
//! Posting a reply means streaming the LLM token-by-token while editing the
//! Discord message on a roughly one-second tick, breaking at the character
//! limit or after 30 seconds of silence, and re-requesting empty completions a
//! few times. That flow is identical whether the reply lives on a freshly sent
//! message or on an interaction response, so it lives here once: [`stream_into`]
//! drives the loop and a [`ReplySink`] handles the per-tick rendering.

use crate::{
    AppResult,
    constants::CHARACTER_LIMIT,
    database::Database,
    error::{AppError, EditMessageSnafu, EditResponseSnafu, StreamingSnafu},
    llm::LlmManager,
    models::{
        character::{Character, CharacterOption},
        history::History,
        message::{AttachmentMode, Message as ChatMessage},
    },
};
use core::time::Duration;
use poise::serenity_prelude::{ComponentInteraction, Context, Message, MessageId};
use rig::{agent::MultiTurnStreamItem, streaming::StreamedAssistantContent};
use serenity::futures::StreamExt as _;
use snafu::ResultExt as _;
use std::time::Instant;
use tokio::time::{MissedTickBehavior, interval};
use tracing::{debug, error, warn};

/// How many times to re-request the LLM when it returns an empty completion.
const MAX_ATTEMPTS: u32 = 3;

/// How long to wait for the first token before giving up on the request.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Reply shown when no token arrives within [`RESPONSE_TIMEOUT`].
const TIMEOUT_MESSAGE: &str = "30 sekunder har gått utan ett svar. Jag ger upp.";

/// Reply shown when every attempt returned an empty completion.
const GAVE_UP_MESSAGE: &str = "AI:n gav inget svar efter flera försök. Jag ger upp.";

/// Reply shown when the request fails to start or the stream errors out.
const ERROR_MESSAGE: &str = "Något gick fel när jag försökte svara. Försök igen senare.";

/// Truncates `text` to the largest character boundary at or below `limit`, so
/// the cut never splits a multi-byte character.
fn truncate_to_char_boundary(text: &mut String, limit: usize) {
    let mut cut = limit;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut = cut.saturating_sub(1);
    }
    text.truncate(cut);
}

/// A finished streamed reply: the rendered text and the model's reported
/// output-token count, used to record per-character generation stats.
pub struct Reply {
    /// The full reply text accumulated from the stream.
    pub text: String,
    /// Output tokens the model reported, or `0` if the stream gave no final usage report.
    pub output_tokens: u64,
    /// Whether this is a genuine model reply rather than a timeout/error/gave-up
    /// sentinel; sentinel replies are not counted into generation stats.
    pub complete: bool,
}

impl Reply {
    /// Returns the reply's word and output-token counts, saturating at
    /// [`u32::MAX`], for recording per-character generation stats.
    fn counts(&self) -> (u32, u32) {
        let words = u32::try_from(self.text.split_whitespace().count()).unwrap_or(u32::MAX);
        let tokens = u32::try_from(self.output_tokens).unwrap_or(u32::MAX);
        (words, tokens)
    }
}

/// Marks the history finished, persists it, and records the character's
/// generation stats for a genuine (non-sentinel) reply. Shared by both sinks'
/// finalize step; the caller stores the chosen reply (and, for a new message,
/// its ID) first.
async fn persist_reply(
    db: &Database,
    history: &mut History,
    character: &Character,
    counts: (u32, u32),
    complete: bool,
) -> AppResult {
    history.set_finished(true);
    db.upsert_history(history.clone()).await?;
    if complete {
        let (words, tokens) = counts;
        db.record_character_generation(character.id(), words, tokens)
            .await?;
    }
    Ok(())
}

/// The Discord message a streamed reply is rendered into, tick by tick.
///
/// [`stream_into`] calls [`placeholder`](ReplySink::placeholder) while the reply
/// is still empty and [`progress`](ReplySink::progress) once tokens arrive. The
/// futures are `Send` so the loop stays usable from the event handler.
pub trait ReplySink {
    /// Render the waiting placeholder after `elapsed` of silence.
    fn placeholder(&mut self, elapsed: Duration) -> impl Future<Output = AppResult> + Send;

    /// Store `total` as the current reply and render it after `elapsed`.
    fn progress(&mut self, total: String, elapsed: Duration)
    -> impl Future<Output = AppResult> + Send;

    /// Store `reply` as the finished reply, persist it (along with the
    /// character's generation stats), and render it once more.
    fn finalize(self, reply: Reply, elapsed: Duration) -> impl Future<Output = AppResult> + Send;
}

/// Renders a streamed reply onto a sent [`Message`] (new replies and hand-offs).
pub struct MessageSink<'a> {
    /// The serenity context used to edit the message.
    pub ctx: &'a Context,
    /// The conversation the reply belongs to.
    pub history: &'a mut History,
    /// The character producing the reply.
    pub character: &'a Character,
    /// The bot message being edited in place.
    pub message: &'a mut Message,
    /// The database used while rendering.
    pub db: &'a Database,
    /// The hand-off select-menu options, computed once for the whole stream.
    pub options: &'a [CharacterOption],
}

impl ReplySink for MessageSink<'_> {
    async fn placeholder(&mut self, elapsed: Duration) -> AppResult {
        let edit =
            self.history
                .to_placeholder_message_edit(self.character, elapsed, self.options);
        self.message
            .edit(self.ctx, edit)
            .await
            .context(EditMessageSnafu)?;
        Ok(())
    }

    async fn progress(&mut self, total: String, elapsed: Duration) -> AppResult {
        self.history
            .set_choices((self.character.clone(), total, elapsed));
        let edit = self
            .history
            .to_edit_response(self.character, &*self.message, self.db, self.options)
            .await;
        self.message
            .edit(self.ctx, edit)
            .await
            .context(EditMessageSnafu)?;
        Ok(())
    }

    async fn finalize(self, reply: Reply, elapsed: Duration) -> AppResult {
        let counts = reply.counts();
        let complete = reply.complete;
        self.history
            .set_choices((self.character.clone(), reply.text, elapsed));
        self.history.set_id(&*self.message);
        persist_reply(self.db, self.history, self.character, counts, complete).await?;
        let edit = self
            .history
            .to_edit_response(self.character, &*self.message, self.db, self.options)
            .await;
        self.message
            .edit(self.ctx, edit)
            .await
            .context(EditMessageSnafu)?;
        Ok(())
    }
}

/// Renders a streamed reply onto a [`ComponentInteraction`] response (swipes).
pub struct InteractionSink<'a> {
    /// The serenity context used to edit the response.
    pub ctx: &'a Context,
    /// The conversation the reply belongs to.
    pub history: &'a mut History,
    /// The character producing the reply.
    pub character: &'a Character,
    /// The interaction whose response is being edited.
    pub interaction: &'a ComponentInteraction,
    /// The message ID of the reply being rendered.
    pub id: MessageId,
    /// The database used while rendering.
    pub db: &'a Database,
    /// The hand-off select-menu options, computed once for the whole stream.
    pub options: &'a [CharacterOption],
}

impl ReplySink for InteractionSink<'_> {
    async fn placeholder(&mut self, elapsed: Duration) -> AppResult {
        let edit =
            self.history
                .to_placeholder_interaction_edit(self.character, elapsed, self.options);
        self.interaction
            .edit_response(&self.ctx.http, edit)
            .await
            .context(EditResponseSnafu)?;
        Ok(())
    }

    async fn progress(&mut self, total: String, elapsed: Duration) -> AppResult {
        self.history
            .update_current_choice((self.character.clone(), total, elapsed));
        let edit = self
            .history
            .to_edit_interaction(self.character, self.id, self.db, self.options)
            .await;
        self.interaction
            .edit_response(&self.ctx.http, edit)
            .await
            .context(EditResponseSnafu)?;
        Ok(())
    }

    async fn finalize(self, reply: Reply, elapsed: Duration) -> AppResult {
        let counts = reply.counts();
        let complete = reply.complete;
        self.history
            .update_current_choice((self.character.clone(), reply.text, elapsed));
        persist_reply(self.db, self.history, self.character, counts, complete).await?;
        let edit = self
            .history
            .to_edit_interaction(self.character, self.id, self.db, self.options)
            .await;
        self.interaction
            .edit_response(&self.ctx.http, edit)
            .await
            .context(EditResponseSnafu)?;
        Ok(())
    }
}

/// Stream an LLM reply, ticking `sink` about once a second so the Discord
/// message is edited in place, and return the accumulated [`Reply`].
///
/// While the reply is empty the sink renders a placeholder; once tokens arrive
/// it renders the growing reply. Empty completions are re-requested up to
/// [`MAX_ATTEMPTS`] times; `start` measures the silence window across attempts.
#[expect(
    clippy::cognitive_complexity,
    reason = "the select loop, retry/timeout/limit handling and their logging are one cohesive flow that the module deliberately keeps together"
)]
pub async fn stream_into<S: ReplySink>(
    requester: &LlmManager,
    context: &[ChatMessage],
    prompt: Option<String>,
    mode: AttachmentMode,
    start: Instant,
    sink: &mut S,
) -> AppResult<Reply> {
    let mut total = String::new();
    let mut output_tokens: u64 = 0;
    let mut attempt: u32 = 0;
    let mut complete = true;
    'attempts: loop {
        attempt = attempt.saturating_add(1);
        let mut stream = match requester.request_stream(context, prompt.clone(), mode).await {
            Ok(stream) => stream,
            Err(source) => {
                let why = AppError::from(source);
                error!("failed to start the reply stream, giving up: {why:?}");
                total += ERROR_MESSAGE;
                complete = false;
                break 'attempts;
            }
        };
        let mut interval = interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                result = stream.next() => {
                    let item = match result.transpose().context(StreamingSnafu) {
                        Ok(item) => item,
                        Err(why) => {
                            error!("the reply stream errored, giving up: {why:?}");
                            if total.is_empty() {
                                total += ERROR_MESSAGE;
                                complete = false;
                            }
                            break 'attempts;
                        }
                    };
                    match item {
                        Some(MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(delta))) => {
                            total += delta.text();
                            if total.len() >= CHARACTER_LIMIT {
                                warn!("reply reached the character limit, truncating");
                                truncate_to_char_boundary(&mut total, CHARACTER_LIMIT);
                                break 'attempts;
                            }
                        }
                        Some(MultiTurnStreamItem::FinalResponse(final_response)) => {
                            output_tokens = final_response.usage().output_tokens;
                        }
                        Some(_) => {},
                        None => break,
                    }
                }
                _ = interval.tick() => {
                    if total.is_empty() {
                        if start.elapsed() >= RESPONSE_TIMEOUT {
                            warn!("no first token within the response timeout, giving up");
                            total += TIMEOUT_MESSAGE;
                            complete = false;
                            break 'attempts;
                        }
                        if let Err(why) = sink.placeholder(start.elapsed()).await {
                            warn!("failed to render placeholder, continuing: {why}");
                        }
                    } else if let Err(why) = sink.progress(total.clone(), start.elapsed()).await {
                        warn!("failed to render progress, continuing: {why}");
                    }
                }
            }
        }

        // the stream ended: keep a non-empty reply, otherwise retry until we
        // run out of attempts.
        if !total.is_empty() {
            break 'attempts;
        }
        if attempt >= MAX_ATTEMPTS {
            warn!("llm returned empty completions after {attempt} attempts, giving up");
            total += GAVE_UP_MESSAGE;
            complete = false;
            break 'attempts;
        }
        debug!("llm returned an empty completion, retrying (attempt {attempt})");
    }

    Ok(Reply {
        text: total,
        output_tokens,
        complete,
    })
}

/// Tests for the character-limit truncation helper.
#[cfg(test)]
mod tests {
    use super::truncate_to_char_boundary;

    /// Cutting inside a multi-byte character walks back to the boundary before it.
    #[test]
    fn truncation_keeps_a_whole_multibyte_char() {
        let mut text = "héllo".to_owned();
        truncate_to_char_boundary(&mut text, 2);
        assert_eq!(
            text, "h",
            "a cut inside the é falls back to the boundary before it"
        );
    }

    /// A cut already on a character boundary keeps everything up to the limit.
    #[test]
    fn truncation_on_a_boundary_keeps_everything_up_to_it() {
        let mut text = "hello".to_owned();
        truncate_to_char_boundary(&mut text, 3);
        assert_eq!(text, "hel", "an ascii cut lands exactly on the limit");
    }

    /// A zero limit truncates to the empty string.
    #[test]
    fn truncation_to_zero_empties_the_string() {
        let mut text = "abc".to_owned();
        truncate_to_char_boundary(&mut text, 0);
        assert!(text.is_empty(), "a zero limit truncates to nothing");
    }
}
