//! The shared streaming loop behind every character reply.
//!
//! Posting a reply means streaming the LLM token-by-token while editing the
//! Discord message on a roughly one-second tick, breaking at the character
//! limit or after 30 seconds of silence, and re-requesting empty completions a
//! few times. That flow is identical whether the reply lives on a freshly sent
//! message or on an interaction response, so it lives here once: [`stream_into`]
//! drives the loop and a [`ReplySink`] handles the per-tick rendering.

mod sinks;

pub use sinks::{ContinueSink, InteractionSink, MessageSink};

use crate::{
    AppResult,
    cancellation::Cancellations,
    constants::{CHARACTER_LIMIT, CONTINUE_REVEAL_DELAY},
    database::Database,
    error::StreamingSnafu,
    llm::{LlmManager, ReplyStream},
    models::{
        character::Character,
        history::History,
        message::{AttachmentMode, Message as ChatMessage},
    },
    util::report_error,
    media::resolve_attachments,
};
use core::time::Duration;
use poise::serenity_prelude::MessageId;
use rig::{agent::MultiTurnStreamItem, streaming::StreamedAssistantContent};
use serenity::futures::StreamExt as _;
use snafu::ResultExt as _;
use std::time::Instant;
use tokio::time::{MissedTickBehavior, interval, sleep, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, warn};

/// How many times to re-request the LLM when it returns an empty completion.
const MAX_ATTEMPTS: u32 = 3;

/// How long to wait for the first token before giving up on the request.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Reply shown when no token arrives within [`RESPONSE_TIMEOUT`].
const TIMEOUT_MESSAGE: &str = "30 sekunder har gått utan ett svar. Jag ger upp.";

/// Reply shown when every attempt returned an empty completion.
const GAVE_UP_MESSAGE: &str = "Gubben gav inget svar efter flera försök. Jag ger upp.";

/// Reply shown when the request fails to start or the stream errors out.
const ERROR_MESSAGE: &str = "Det krånglade när jag försökte svara. Jag ger upp.";

/// Truncates `text` to at most `limit` characters, never splitting a multi-byte
/// character. Discord counts characters, not bytes, so the limit is in characters.
fn truncate_to_chars(text: &mut String, limit: usize) {
    if let Some((cut, _)) = text.char_indices().nth(limit) {
        text.truncate(cut);
    }
}

/// Joins a continuation onto the reply it extends, capped at `limit` characters.
///
/// A single space is inserted only when the two sides would otherwise butt a word
/// against a word (neither already carries a whitespace boundary), so a reply
/// continued mid-sentence reads naturally rather than gluing two words together.
fn combine_continuation(seed: &str, addition: &str, limit: usize) -> String {
    let needs_space = !seed.is_empty()
        && !addition.is_empty()
        && !seed.ends_with(char::is_whitespace)
        && !addition.starts_with(char::is_whitespace);
    let mut combined = String::with_capacity(seed.len().saturating_add(addition.len()));
    combined.push_str(seed);
    if needs_space {
        combined.push(' ');
    }
    combined.push_str(addition);
    truncate_to_chars(&mut combined, limit);
    combined
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

    /// Whether a failed reply should keep whatever has already streamed rather
    /// than be stored as the failure sentinel and marked an error.
    ///
    /// False for a fresh reply (a failure becomes the red error choice). True for
    /// a continuation, which must never clobber the genuine reply it extends.
    fn keeps_reply_on_failure(&self) -> bool {
        false
    }

    /// Persist the finished history together with the character's generation stats.
    fn persist(
        &mut self,
        counts: (u32, u32),
        complete: bool,
    ) -> impl Future<Output = AppResult> + Send;

    /// Re-render the current reply and edit it into the Discord target.
    fn render_and_edit(&mut self) -> impl Future<Output = AppResult> + Send;

    /// Renders the settled reply, revealing its Continue button a beat later when
    /// the reply can be continued.
    ///
    /// Continue takes over the slot the live Stop button held while streaming, so
    /// a continuable reply is first rendered with Continue suppressed, then
    /// re-rendered once it is revealed (the delay keeps a late Stop press from
    /// landing on a freshly live Continue). A reply that cannot be continued just
    /// renders once.
    fn settle_and_reveal(&mut self) -> impl Future<Output = AppResult> + Send
    where
        Self: Send,
    {
        async move {
            if self.history().continue_available() {
                self.history().set_show_continue(false);
                self.render_and_edit().await?;
                sleep(CONTINUE_REVEAL_DELAY).await;
                self.history().set_show_continue(true);
            }
            self.render_and_edit().await
        }
    }

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
            // a failed continuation keeps the genuine reply it extends untouched;
            // every other failure is stored as the sentinel and marked an error
            if complete || !self.keeps_reply_on_failure() {
                self.store_choice((self.character().clone(), reply.text, elapsed));
                self.before_persist();
                if !complete {
                    self.history().set_current_choice_error();
                }
            }
            self.persist(counts, complete).await?;
            self.settle_and_reveal().await
        }
    }
}

/// Prepares the LLM request for a reply: builds the requester from the
/// character's resolved model settings, assembles the conversation context, and
/// resolves how attachments are sent (describing and caching images when the
/// model lacks vision). Borrows the history mutably only for the duration of
/// attachment resolution, so the caller can hand it to a [`ReplySink`] after.
async fn prepare_request(
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

/// How an attempt to start the reply stream ended.
enum StreamStart {
    /// The stream is live and ready to be consumed.
    Started(ReplyStream),
    /// The user stopped the reply before it started.
    Cancelled,
    /// The request could not be started at all.
    Failed,
    /// No stream was handed back within the silence budget.
    TimedOut,
}

/// Starts the reply stream, racing the request against both the stop token and
/// what is left of the [`RESPONSE_TIMEOUT`] silence budget measured from `start`.
///
/// Without the race a stalled connection (accepted, but never answering) would
/// never reach the streaming loop, leaving the reply stuck on its placeholder
/// with a Stop button that has nothing to cancel.
async fn start_stream(
    requester: &LlmManager,
    context: &[ChatMessage],
    prompt: Option<String>,
    mode: AttachmentMode,
    start: Instant,
    token: &CancellationToken,
) -> StreamStart {
    let remaining = RESPONSE_TIMEOUT.saturating_sub(start.elapsed());
    let started = timeout(remaining, requester.request_stream(context, prompt, mode));
    tokio::select! {
        biased;
        () = token.cancelled() => StreamStart::Cancelled,
        result = started => match result {
            Ok(Ok(stream)) => StreamStart::Started(stream),
            Ok(Err(source)) => {
                error!("failed to start the reply stream, giving up");
                report_error(source);
                StreamStart::Failed
            }
            Err(_) => {
                error!("the reply stream did not start within the response timeout, giving up");
                StreamStart::TimedOut
            }
        },
    }
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
async fn stream_into<S: ReplySink + Send>(
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
        let mut stream = match start_stream(requester, context, prompt.clone(), mode, start, token)
            .await
        {
            StreamStart::Started(stream) => stream,
            StreamStart::Cancelled => {
                debug!("user stopped the stream before it started");
                break 'attempts;
            }
            StreamStart::Failed => {
                total += ERROR_MESSAGE;
                complete = false;
                break 'attempts;
            }
            StreamStart::TimedOut => {
                total += TIMEOUT_MESSAGE;
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

/// Tests for the truncation helper and the shared finalize bookkeeping.
///
/// The in-stream loop itself (timeout/retry/cancel/character-limit) is not unit
/// tested here: it consumes a live `rig` stream whose item types are not
/// constructible without a network, so faking it would buy no real coverage.
/// What is testable without a network, and what these tests pin, is the
/// finalize path that turns a finished [`Reply`] into a stored choice: the
/// `complete == false` to red-error-container mechanism, the generation-stat
/// counts, and the pre-stream-failure sentinel.
#[cfg(test)]
mod tests {
    use super::{
        ERROR_MESSAGE, GAVE_UP_MESSAGE, Reply, ReplySink, combine_continuation, finalize_failed,
        truncate_to_chars,
    };
    use crate::AppResult;
    use crate::llm::{LlmManager, ModelSettings};
    use crate::models::character::Character;
    use crate::models::history::History;
    use crate::models::message::{AttachmentMode, Message};
    use core::time::Duration;
    use nonempty::NonEmpty;
    use serenity::all::{MessageId, UserId};

    /// The side effects the finalize path is expected to drive, captured so the
    /// test can assert them after the sink is consumed.
    #[derive(Default)]
    struct Records {
        /// The text passed to the last `store_choice`.
        stored_text: Option<String>,
        /// The `(counts, complete)` passed to `persist`.
        persisted: Option<((u32, u32), bool)>,
        /// Whether the `before_persist` hook ran.
        before_persist: bool,
        /// How many times the reply was rendered.
        rendered: u32,
    }

    /// A [`ReplySink`] that records the finalize bookkeeping into borrowed state
    /// instead of touching Discord, mirroring `MessageSink`'s `store_choice`.
    struct TestSink<'a> {
        /// The conversation being finalized.
        history: &'a mut History,
        /// The character producing the reply.
        character: Character,
        /// Where the driven side effects are recorded.
        records: &'a mut Records,
        /// Whether a failed reply is kept rather than marked an error (a continuation).
        keep_on_failure: bool,
    }

    impl ReplySink for TestSink<'_> {
        fn message_id(&self) -> MessageId {
            MessageId::new(1)
        }

        async fn prepare(&mut self) -> AppResult<(LlmManager, Vec<Message>, AttachmentMode)> {
            Ok((
                LlmManager::new(ModelSettings::default()),
                Vec::new(),
                AttachmentMode::default(),
            ))
        }

        async fn placeholder(&mut self, _elapsed: Duration) -> AppResult {
            Ok(())
        }

        fn history(&mut self) -> &mut History {
            self.history
        }

        fn character(&self) -> &Character {
            &self.character
        }

        fn store_choice(&mut self, choice: (Character, String, Duration)) {
            self.records.stored_text = Some(choice.1.clone());
            self.history.set_choices(choice);
        }

        fn before_persist(&mut self) {
            self.records.before_persist = true;
        }

        fn keeps_reply_on_failure(&self) -> bool {
            self.keep_on_failure
        }

        async fn persist(&mut self, counts: (u32, u32), complete: bool) -> AppResult {
            self.records.persisted = Some((counts, complete));
            Ok(())
        }

        async fn render_and_edit(&mut self) -> AppResult {
            self.records.rendered = self.records.rendered.saturating_add(1);
            Ok(())
        }
    }

    /// Builds a minimal history with a single placeholder choice to finalize over.
    fn fresh_history() -> History {
        History::builder()
            .id(MessageId::new(1))
            .character("character-id")
            .choices(NonEmpty::new(Message::new_system("placeholder")))
            .current(0_usize)
            .previous(Vec::new())
            .build()
    }

    /// A minimal character for the finalize path.
    fn test_character() -> Character {
        Character::builder()
            .id("character-id".to_owned())
            .name("Harry")
            .greeting("hej")
            .creator(UserId::new(1))
            .build()
    }

    /// `counts` reports whitespace-split words and the output tokens, saturating
    /// rather than wrapping on an enormous token report.
    #[test]
    fn counts_reports_words_and_tokens() {
        let reply = Reply {
            text: "  hej  på  dig ".to_owned(),
            output_tokens: 7,
            complete: true,
        };
        assert_eq!(
            reply.counts(),
            (3, 7),
            "three whitespace-separated words and the reported token count"
        );
        let huge = Reply {
            text: String::new(),
            output_tokens: u64::MAX,
            complete: true,
        };
        assert_eq!(
            huge.counts(),
            (0, u32::MAX),
            "no words, and the token count saturates instead of wrapping"
        );
    }

    /// Finalizing a failed reply marks the stored choice as an error (so it
    /// renders as the red error container) and persists it as non-complete.
    #[tokio::test]
    async fn finalize_marks_a_failed_reply_as_an_error() {
        let mut history = fresh_history();
        let mut records = Records::default();
        let sink = TestSink {
            history: &mut history,
            character: test_character(),
            records: &mut records,
            keep_on_failure: false,
        };

        let reply = Reply {
            text: "oops".to_owned(),
            output_tokens: 0,
            complete: false,
        };
        let finalized = sink.finalize(reply, Duration::from_secs(1)).await;

        assert!(finalized.is_ok(), "finalize succeeds");
        assert!(
            history.chosen_message().is_error(),
            "a non-complete reply marks the current choice as an error"
        );
        assert_eq!(
            records.stored_text.as_deref(),
            Some("oops"),
            "the reply text is stored as the choice"
        );
        assert_eq!(
            records.persisted,
            Some(((1, 0), false)),
            "the failed reply persists as non-complete and is not counted"
        );
        assert!(records.before_persist, "the before-persist hook ran");
        assert_eq!(records.rendered, 1, "the finished reply is rendered once");
    }

    /// Finalizing a genuine reply leaves the choice unmarked and persists it as
    /// complete, so its generation stats are recorded.
    ///
    /// Paused-time: a genuine reply is continuable, so finalize sleeps for the
    /// Continue reveal delay, which virtual time skips.
    #[tokio::test(start_paused = true)]
    async fn finalize_keeps_a_genuine_reply() {
        let mut history = fresh_history();
        let mut records = Records::default();
        let sink = TestSink {
            history: &mut history,
            character: test_character(),
            records: &mut records,
            keep_on_failure: false,
        };

        let reply = Reply {
            text: "hej hej".to_owned(),
            output_tokens: 4,
            complete: true,
        };
        let finalized = sink.finalize(reply, Duration::from_secs(1)).await;

        assert!(finalized.is_ok(), "finalize succeeds");
        assert!(
            !history.chosen_message().is_error(),
            "a complete reply is not marked as an error"
        );
        assert_eq!(
            records.persisted,
            Some(((2, 4), true)),
            "a genuine reply persists as complete with its word and token counts"
        );
        assert_eq!(
            records.rendered, 2,
            "a continuable reply renders twice: once with Continue suppressed, then once revealed"
        );
    }

    /// A continuable reply reveals Continue with a second render after the delay,
    /// while a non-continuable one renders only once.
    #[tokio::test(start_paused = true)]
    async fn finalize_reveals_continue_with_a_delayed_second_render() {
        let mut history = fresh_history();
        let mut records = Records::default();
        let sink = TestSink {
            history: &mut history,
            character: test_character(),
            records: &mut records,
            keep_on_failure: false,
        };

        let reply = Reply {
            text: "ett riktigt svar".to_owned(),
            output_tokens: 3,
            complete: true,
        };
        let finalized = sink.finalize(reply, Duration::from_secs(1)).await;

        assert!(finalized.is_ok(), "finalize succeeds");
        assert_eq!(
            records.rendered, 2,
            "the reply is rendered with Continue suppressed, then again once it is revealed"
        );
        assert!(
            history.continue_available(),
            "the finished genuine reply ends up continuable"
        );
    }

    /// `combine_continuation` appends the continuation onto the seed, inserting a
    /// single space only when two words would otherwise be glued together.
    #[test]
    fn combine_continuation_joins_with_a_space_only_when_needed() {
        assert_eq!(
            combine_continuation("hello and", "the rain", 100),
            "hello and the rain",
            "a word-to-word join gets one separating space"
        );
        assert_eq!(
            combine_continuation("hello ", "the rain", 100),
            "hello the rain",
            "a seed already ending in whitespace is not double-spaced"
        );
        assert_eq!(
            combine_continuation("hello", " the rain", 100),
            "hello the rain",
            "a continuation already starting with whitespace is not double-spaced"
        );
        assert_eq!(
            combine_continuation("", "fresh start", 100),
            "fresh start",
            "an empty seed yields the continuation alone, with no leading space"
        );
        assert_eq!(
            combine_continuation("kept", "", 100),
            "kept",
            "an empty continuation leaves the seed untouched"
        );
    }

    /// `combine_continuation` truncates the joined text to the character limit so a
    /// continuation cannot push a reply past it.
    #[test]
    fn combine_continuation_caps_at_the_limit() {
        let seed = "x".repeat(8);
        let combined = combine_continuation(&seed, "yyyy", 10);
        assert_eq!(
            combined.chars().count(),
            10,
            "the joined text is capped at the limit"
        );
        assert_eq!(combined, "xxxxxxxx y", "the cap keeps the seed and cuts the tail");
    }

    /// A keep-on-failure sink (a continuation) leaves the reply it was extending
    /// untouched on failure: it does not store the sentinel text and does not mark
    /// the choice an error, so a failed Continue cannot clobber a genuine reply.
    ///
    /// Paused-time: the kept reply stays continuable, so finalize sleeps for the
    /// Continue reveal delay, which virtual time skips.
    #[tokio::test(start_paused = true)]
    async fn finalize_keeps_the_reply_when_a_continuation_fails() {
        let mut history = fresh_history();
        let mut records = Records::default();
        let sink = TestSink {
            history: &mut history,
            character: test_character(),
            records: &mut records,
            keep_on_failure: true,
        };

        let reply = Reply {
            text: GAVE_UP_MESSAGE.to_owned(),
            output_tokens: 0,
            complete: false,
        };
        let finalized = sink.finalize(reply, Duration::from_secs(1)).await;

        assert!(finalized.is_ok(), "finalize succeeds");
        assert!(
            !history.chosen_message().is_error(),
            "a failed continuation does not mark the reply it extends as an error"
        );
        assert_eq!(
            records.stored_text, None,
            "the failure sentinel is not stored over the existing reply"
        );
        assert!(
            !records.before_persist,
            "the before-persist hook is skipped when the reply is kept"
        );
        assert_eq!(
            records.persisted.map(|(_, complete)| complete),
            Some(false),
            "the kept reply still persists, classified as non-complete"
        );
    }

    /// A pre-stream failure finalizes with the generic error sentinel, marking the
    /// choice as an error exactly like an in-stream failure.
    #[tokio::test]
    async fn finalize_failed_renders_the_error_sentinel() {
        let mut history = fresh_history();
        let mut records = Records::default();
        let sink = TestSink {
            history: &mut history,
            character: test_character(),
            records: &mut records,
            keep_on_failure: false,
        };

        let finalized = finalize_failed(sink).await;

        assert!(finalized.is_ok(), "the failed finalize succeeds");
        assert!(
            history.chosen_message().is_error(),
            "the sentinel reply marks the choice as an error"
        );
        assert_eq!(
            records.stored_text.as_deref(),
            Some(ERROR_MESSAGE),
            "the generic error message is stored as the choice"
        );
        assert_eq!(
            records.persisted.map(|(_, complete)| complete),
            Some(false),
            "the sentinel reply persists as non-complete"
        );
    }

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
