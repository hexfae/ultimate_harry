//! The three concrete [`ReplySink`] adapters that wire the shared streaming
//! engine to a particular Discord surface: a freshly sent message, an
//! interaction response, and an in-place continuation of a finished reply.

use crate::{
    AppResult,
    constants::CHARACTER_LIMIT,
    database::Database,
    error::{EditMessageSnafu, EditResponseSnafu},
    llm::LlmManager,
    models::{
        character::{Character, CharacterOption},
        history::History,
        message::{AttachmentMode, Message as ChatMessage},
    },
};
use core::time::Duration;
use poise::serenity_prelude::{ComponentInteraction, Context, Message, MessageId};
use snafu::ResultExt as _;

use super::{ReplySink, combine_continuation, persist_reply, prepare_request};

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

/// Renders a streamed *continuation* of an existing reply onto a
/// [`ComponentInteraction`] response (the Continue button).
///
/// Like [`InteractionSink`], but it seeds the LLM context with the reply being
/// continued (as a trailing assistant turn) and prepends that reply's text to
/// every streamed update, so the new tokens extend the reply in place rather than
/// replacing it.
pub struct ContinueSink<'a> {
    /// The interaction sink doing the rendering and persistence.
    pub inner: InteractionSink<'a>,
    /// The existing reply text the continuation is appended to.
    pub seed: String,
}

impl ReplySink for ContinueSink<'_> {
    fn message_id(&self) -> MessageId {
        self.inner.message_id()
    }

    async fn prepare(&mut self) -> AppResult<(LlmManager, Vec<ChatMessage>, AttachmentMode)> {
        let (requester, mut context, mode) = self.inner.prepare().await?;
        // give the model the reply so far as the last assistant turn, so it
        // continues that text rather than starting a fresh reply
        if !self.seed.is_empty() {
            context.push(ChatMessage::new_assistant(
                self.seed.clone(),
                self.inner.character,
            ));
        }
        Ok((requester, context, mode))
    }

    async fn placeholder(&mut self, _elapsed: Duration) -> AppResult {
        // keep the existing reply (and a live Stop) on screen while waiting for
        // the first continuation token, rather than blanking it to the "…" glyph
        self.render_and_edit().await
    }

    fn history(&mut self) -> &mut History {
        self.inner.history()
    }

    fn character(&self) -> &Character {
        self.inner.character()
    }

    fn store_choice(&mut self, choice: (Character, String, Duration)) {
        let (character, addition, elapsed) = choice;
        let combined = combine_continuation(&self.seed, &addition, CHARACTER_LIMIT);
        self.inner.store_choice((character, combined, elapsed));
    }

    async fn persist(&mut self, counts: (u32, u32), complete: bool) -> AppResult {
        self.inner.persist(counts, complete).await
    }

    async fn render_and_edit(&mut self) -> AppResult {
        self.inner.render_and_edit().await
    }

    /// A failed continuation keeps the reply it was extending, rather than
    /// replacing it with an error: that reply was a genuine one.
    fn keeps_reply_on_failure(&self) -> bool {
        true
    }
}
