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
    cancellation::Cancellations,
    constants::CHARACTER_LIMIT,
    database::Database,
    error::{EditMessageSnafu, EditResponseSnafu, StreamingSnafu},
    llm::LlmManager,
    models::{
        character::{Character, CharacterOption},
        history::History,
        message::{AttachmentMode, Message as ChatMessage},
    },
    util::report_error,
    media::resolve_attachments,
};
use core::time::Duration;
use poise::serenity_prelude::{ComponentInteraction, Context, Message, MessageId};
use rig::{agent::MultiTurnStreamItem, streaming::StreamedAssistantContent};
use serenity::futures::StreamExt as _;
use snafu::ResultExt as _;
use std::time::Instant;
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;
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

/// Truncates `text` to at most `limit` characters, never splitting a multi-byte
/// character. Discord counts characters, not bytes, so the limit is in characters.
fn truncate_to_chars(text: &mut String, limit: usize) {
    if let Some((cut, _)) = text.char_indices().nth(limit) {
        text.truncate(cut);
    }
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
        // best-effort: the reply is already saved, so a stats-write blip must
        // not fail the reply and replace it with an error notice
        if let Err(why) = db
            .record_character_generation(character.id(), words, tokens)
            .await
        {
            warn!("failed to record generation stats, keeping the reply");
            report_error(why);
        }
    }
    Ok(())
}

/// The Discord message a streamed reply is rendered into, tick by tick.
///
/// [`stream_into`] calls [`placeholder`](ReplySink::placeholder) while the reply
/// is still empty and [`progress`](ReplySink::progress) once tokens arrive. The
/// futures are `Send` so the loop stays usable from the event handler.
pub trait ReplySink {
    /// The Discord message ID of the reply being streamed, used to key its stop
    /// token in the cancellation registry.
    fn message_id(&self) -> MessageId;

    /// Prepare the LLM request (requester, conversation context, attachment mode)
    /// for this reply, from the sink's own character and history.
    fn prepare(
        &mut self,
    ) -> impl Future<Output = AppResult<(LlmManager, Vec<ChatMessage>, AttachmentMode)>> + Send;

    /// Render the waiting placeholder after `elapsed` of silence.
    fn placeholder(&mut self, elapsed: Duration) -> impl Future<Output = AppResult> + Send;

    /// The conversation being rendered, for the shared finalize bookkeeping.
    fn history(&mut self) -> &mut History;

    /// The character producing the reply, for the shared progress/finalize steps.
    fn character(&self) -> &Character;

    /// Store `choice` as this reply's current choice: a new swipe branch on a
    /// freshly sent message, an in-place update for an interaction swipe.
    fn store_choice(&mut self, choice: (Character, String, Duration));

    /// Hook run in [`finalize`](ReplySink::finalize) after the final choice is
    /// stored but before persisting. A sent-message sink keys the history by the
    /// posted message ID here; an interaction sink does nothing.
    fn before_persist(&mut self) {}

    /// Persist the finished history together with the character's generation stats.
    fn persist(
        &mut self,
        counts: (u32, u32),
        complete: bool,
    ) -> impl Future<Output = AppResult> + Send;

    /// Re-render the current reply and edit it into the Discord target.
    fn render_and_edit(&mut self) -> impl Future<Output = AppResult> + Send;

    /// Store `total` as the current reply and render it after `elapsed`.
    fn progress(
        &mut self,
        total: String,
        elapsed: Duration,
    ) -> impl Future<Output = AppResult> + Send
    where
        Self: Send,
    {
        async move {
            self.store_choice((self.character().clone(), total, elapsed));
            self.render_and_edit().await
        }
    }

    /// Store `reply` as the finished reply, persist it (along with the
    /// character's generation stats), and render it once more.
    fn finalize(
        mut self,
        reply: Reply,
        elapsed: Duration,
    ) -> impl Future<Output = AppResult> + Send
    where
        Self: Sized + Send,
    {
        async move {
            let counts = reply.counts();
            let complete = reply.complete;
            self.store_choice((self.character().clone(), reply.text, elapsed));
            self.before_persist();
            if !complete {
                self.history().set_current_choice_error();
            }
            self.persist(counts, complete).await?;
            self.render_and_edit().await
        }
    }
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
    fn message_id(&self) -> MessageId {
        self.message.id
    }

    async fn prepare(&mut self) -> AppResult<(LlmManager, Vec<ChatMessage>, AttachmentMode)> {
        prepare_request(self.db, self.character, self.history).await
    }

    async fn placeholder(&mut self, elapsed: Duration) -> AppResult {
        let edit = self
            .history
            .to_placeholder_message_edit(
                self.character,
                self.message.id,
                elapsed,
                self.db,
                self.options,
            )
            .await;
        self.message
            .edit(self.ctx, edit)
            .await
            .context(EditMessageSnafu)?;
        Ok(())
    }

    fn history(&mut self) -> &mut History {
        self.history
    }

    fn character(&self) -> &Character {
        self.character
    }

    fn store_choice(&mut self, choice: (Character, String, Duration)) {
        self.history.set_choices(choice);
    }

    fn before_persist(&mut self) {
        self.history.set_id(&*self.message);
    }

    async fn persist(&mut self, counts: (u32, u32), complete: bool) -> AppResult {
        persist_reply(self.db, self.history, self.character, counts, complete).await
    }

    async fn render_and_edit(&mut self) -> AppResult {
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
    fn message_id(&self) -> MessageId {
        self.id
    }

    async fn prepare(&mut self) -> AppResult<(LlmManager, Vec<ChatMessage>, AttachmentMode)> {
        prepare_request(self.db, self.character, self.history).await
    }

    async fn placeholder(&mut self, elapsed: Duration) -> AppResult {
        let edit = self
            .history
            .to_placeholder_interaction_edit(
                self.character,
                self.id,
                elapsed,
                self.db,
                self.options,
            )
            .await;
        self.interaction
            .edit_response(&self.ctx.http, edit)
            .await
            .context(EditResponseSnafu)?;
        Ok(())
    }

    fn history(&mut self) -> &mut History {
        self.history
    }

    fn character(&self) -> &Character {
        self.character
    }

    fn store_choice(&mut self, choice: (Character, String, Duration)) {
        self.history.update_current_choice(choice);
    }

    async fn persist(&mut self, counts: (u32, u32), complete: bool) -> AppResult {
        persist_reply(self.db, self.history, self.character, counts, complete).await
    }

    async fn render_and_edit(&mut self) -> AppResult {
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

/// Prepares the LLM request for a reply: builds the requester from the
/// character's resolved model settings, assembles the conversation context, and
/// resolves how attachments are sent (describing and caching images when the
/// model lacks vision). Borrows the history mutably only for the duration of
/// attachment resolution, so the caller can hand it to a [`ReplySink`] after.
pub async fn prepare_request(
    db: &Database,
    character: &Character,
    history: &mut History,
) -> AppResult<(LlmManager, Vec<ChatMessage>, AttachmentMode)> {
    let requester = LlmManager::new(db.resolved_model_settings(character).await);
    let mut context = history.build_context(character);
    let mode = resolve_attachments(&requester, history, &mut context).await;
    Ok((requester, context, mode))
}

/// Prepares the request, streams the reply into `sink`, then finalizes it
/// (storing the chosen reply, persisting the history, and recording the
/// character's generation stats).
///
/// The shared tail of every reply handler: build a sink and hand it here. A
/// failure to prepare the request is treated like any other reply-ending failure
/// (a timeout or a stream error): it renders as the red error choice, with the
/// buttons kept live so the user can swipe/next to retry, rather than aborting.
///
/// The reply's stop token is registered in `cancellations` for the duration of
/// the stream (and removed when the returned guard drops), so the Stop button
/// can end the stream from another task.
pub async fn stream_and_finalize<S: ReplySink + Send>(
    prompt: Option<String>,
    cancellations: &Cancellations,
    mut sink: S,
) -> AppResult {
    let guard = cancellations.begin(sink.message_id());
    let token = guard.token();
    let (requester, context, mode) = match sink.prepare().await {
        Ok(prepared) => prepared,
        Err(why) => {
            error!("failed to prepare the reply request, giving up");
            report_error(why);
            return finalize_failed(sink).await;
        }
    };
    let now = Instant::now();
    let reply = stream_into(&requester, &context, prompt, mode, now, &token, &mut sink).await?;
    sink.finalize(reply, now.elapsed()).await
}

/// Finalizes `sink` with the generic error sentinel, so a pre-stream failure
/// renders as the red error choice exactly like an in-stream one.
async fn finalize_failed<S: ReplySink + Send>(sink: S) -> AppResult {
    let reply = Reply {
        text: ERROR_MESSAGE.to_owned(),
        output_tokens: 0,
        complete: false,
    };
    sink.finalize(reply, Duration::ZERO).await
}

/// Stream an LLM reply, ticking `sink` about once a second so the Discord
/// message is edited in place, and return the accumulated [`Reply`].
///
/// While the reply is empty the sink renders a placeholder; once tokens arrive
/// it renders the growing reply. Empty completions are re-requested up to
/// [`MAX_ATTEMPTS`] times; `start` measures the silence window across attempts.
///
/// Cancelling `token` (via the Stop button) ends the stream immediately, keeping
/// whatever has streamed so far as a genuine reply.
#[expect(
    clippy::cognitive_complexity,
    reason = "the select loop, retry/timeout/limit handling and their logging are one cohesive flow that the module deliberately keeps together"
)]
pub async fn stream_into<S: ReplySink + Send>(
    requester: &LlmManager,
    context: &[ChatMessage],
    prompt: Option<String>,
    mode: AttachmentMode,
    start: Instant,
    token: &CancellationToken,
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
                error!("failed to start the reply stream, giving up");
                report_error(source);
                total += ERROR_MESSAGE;
                complete = false;
                break 'attempts;
            }
        };
        let mut interval = interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                // biased so a stop press always wins the race against an
                // arriving token batch or a render tick, rather than being
                // out-voted by tokio's random branch selection
                biased;
                () = token.cancelled() => {
                    debug!("user stopped the stream, keeping the partial reply");
                    break 'attempts;
                }
                result = stream.next() => {
                    let item = match result.transpose().context(StreamingSnafu) {
                        Ok(item) => item,
                        Err(why) => {
                            error!("the reply stream errored, giving up");
                            report_error(why);
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
                            if total.chars().count() >= CHARACTER_LIMIT {
                                warn!("reply reached the character limit, truncating");
                                truncate_to_chars(&mut total, CHARACTER_LIMIT);
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
                            error!("no first token within the response timeout, giving up");
                            total += TIMEOUT_MESSAGE;
                            complete = false;
                            break 'attempts;
                        }
                        // a render tick failing is best-effort: the next tick (or
                        // finalize) re-renders, so warn concisely and keep streaming.
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
            error!("llm returned empty completions after {attempt} attempts, giving up");
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
    use super::truncate_to_chars;

    /// The limit counts characters, not bytes, and never splits a multi-byte one.
    #[test]
    fn truncation_counts_characters_not_bytes() {
        let mut text = "héllo".to_owned();
        truncate_to_chars(&mut text, 2);
        assert_eq!(
            text, "hé",
            "two characters are kept even though é is two bytes"
        );
    }

    /// A limit at or above the character count keeps everything.
    #[test]
    fn truncation_at_the_length_keeps_everything() {
        let mut text = "hello".to_owned();
        truncate_to_chars(&mut text, 5);
        assert_eq!(text, "hello", "nothing is cut when the limit is not exceeded");
    }

    /// A zero limit truncates to the empty string.
    #[test]
    fn truncation_to_zero_empties_the_string() {
        let mut text = "abc".to_owned();
        truncate_to_chars(&mut text, 0);
        assert!(text.is_empty(), "a zero limit truncates to nothing");
    }
}
