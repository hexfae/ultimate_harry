//! The history model: a conversation between a user and a character.
//!
//! A new [`History`] is saved per bot reply, keyed by that reply's Discord message ID, so a button
//! interaction or a user reply can find the conversation from the message it acted on.
//!
//! ## Self-contained storage
//!
//! A history embeds the full messages it owns, so each stored record is a complete, readable record
//! of one branch of a conversation:
//!
//! - [`StoredHistory`] (the persisted form) holds the full `previous` messages (the chat context)
//!   and the full `choices` (the swipeable replies for this turn), inline.
//! - The system-prompt scaffolding (the Swedish roleplay framing built by [`scaffolding`]) is not
//!   stored; it is rebuilt from the character at request time. Characters are versioned and a
//!   history references a specific version, so the rebuilt scaffolding always matches.
//!
//! Sibling histories of one conversation overlap (each branch carries its own root-to-node path),
//! which is accepted: the bot serves a handful of users and a self-contained, openable record is
//! worth the duplication.
//!
//! ## In-memory vs stored
//!
//! [`History`] (this in-memory form, used by the chat loop, interaction handlers, and rendering)
//! differs from [`StoredHistory`] only in carrying the transient `has_finished` flag (used while
//! streaming) and a [`NonEmpty`] choices list. [`History::into_stored`] (on save) and
//! [`History::hydrate`] (on load) bridge the two forms; the LLM context is assembled by
//! [`History::build_context`], which wraps the `previous` messages with the rebuilt scaffolding in
//! front and the character's system prompt appended after.

use bon::Builder;
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use serenity::all::{MessageId, UserId};

use crate::{
    constants::CHARACTER_LIMIT,
    models::{
        character::Character,
        message::{DescribedAttachment, Message},
        modals::EditMessageModal,
    },
    util::wrapping_previous,
};

mod context;
mod render;

/// A log of messages between the user and a character.
///
/// This is the in-memory form used by the chat loop, interaction handlers, and rendering.
/// It is persisted as a [`StoredHistory`] (which embeds the full messages inline); the
/// system-prompt scaffolding is never stored, but rebuilt from the character via [`scaffolding`].
#[derive(Debug, Clone, Builder)]
pub struct History {
    /// The Discord Message ID of this history.
    #[builder(with = |id: MessageId| id.to_string())]
    id: String,
    /// The ulid ID of the currently responding character.
    #[builder(with = |id: &str| id.to_owned())]
    character: String,
    /// The current responses the user can pick between by "swiping" (pressing next/previous).
    choices: NonEmpty<Message>,
    /// The index of the current response the user has chosen.
    #[builder(default)]
    current: usize,
    /// If the streaming has finished for this history. Transient, never stored.
    ///
    /// This is used to create embeds while streaming a response.
    #[builder(default = true)]
    has_finished: bool,
    /// Whether the Continue button may be shown on a finished reply. Transient,
    /// never stored.
    ///
    /// Held false for a beat right after a reply finishes so the button takes
    /// over the live Stop slot only after a short delay (see
    /// [`CONTINUE_REVEAL_DELAY`](crate::constants::CONTINUE_REVEAL_DELAY)).
    #[builder(default = true)]
    show_continue: bool,
    /// The previous messages (the chat context), in order. Contains no scaffolding.
    #[builder(default)]
    previous: Vec<Message>,
}

/// The persisted form of a [`History`]: the full messages inline, no scaffolding.
#[derive(Debug, Serialize, Deserialize)]
#[expect(
    clippy::module_name_repetitions,
    reason = "StoredHistory is the persisted counterpart of History and belongs in this module"
)]
pub struct StoredHistory {
    /// The Discord Message ID of this history, used as the file name.
    pub id: String,
    /// The ulid ID of the currently responding character.
    pub character: String,
    /// The previous messages (the chat context), in order.
    pub previous: Vec<Message>,
    /// The current swipeable responses.
    pub choices: Vec<Message>,
    /// The index of the current chosen response.
    pub current: usize,
}

/// A log of messages between the user and a character.
impl History {
    /// Edits the content of the current choice message.
    pub fn edit_content<A, C, E>(&mut self, author: A, content: C, editor: Option<E>)
    where
        A: Into<String>,
        C: Into<String>,
        E: Into<UserId>,
    {
        if let Some(choice) = self.choices.get_mut(self.current) {
            choice.edit(author, content, editor);
        }
    }

    /// Sets whether the history has finished generating.
    pub const fn set_finished(&mut self, has_finished: bool) {
        self.has_finished = has_finished;
    }

    /// Sets whether a finished reply may show its Continue button.
    pub const fn set_show_continue(&mut self, show_continue: bool) {
        self.show_continue = show_continue;
    }

    /// Whether the chosen reply can be continued (extended in place).
    ///
    /// True only for a finished, genuine reply with room left to grow: a reply
    /// still streaming, a failed-generation notice, an empty reply, or one that
    /// already reached the character limit cannot be continued.
    #[must_use]
    pub fn continue_available(&self) -> bool {
        if !self.has_finished {
            return false;
        }
        let chosen = self.chosen_message();
        if chosen.is_error() {
            return false;
        }
        let content = chosen.chosen_revision().head().content();
        !content.is_empty() && content.chars().count() < CHARACTER_LIMIT
    }

    /// Shows the previous choice by cycling the current index backward.
    pub fn previous(&mut self) {
        self.current = wrapping_previous(self.current, self.choices.len());
    }

    /// Shows the next choice by cycling the current index forward.
    ///
    /// Callers must ensure this is not invoked on the last choice; advancing off
    /// the end is handled separately as a swipe-to-generate.
    pub const fn next(&mut self) {
        self.current = self.current.saturating_add(1);
    }

    /// Returns the current choice index.
    #[must_use]
    pub const fn current_choice(&self) -> usize {
        self.current
    }

    /// Returns the number of choices.
    #[must_use]
    pub fn choices_count(&self) -> usize {
        self.choices.len()
    }

    /// Returns true if there are multiple choices of this history.
    fn has_multiple_choices(&self) -> bool {
        self.choices.len() > 1
    }

    /// Returns true if the currently chosen message has one or more edits.
    fn chosen_has_edit(&self) -> bool {
        self.chosen_message().revisions_count() > 0
    }

    /// Returns whether the user is on the last choice.
    #[must_use]
    pub fn is_on_last_choice(&self) -> bool {
        self.current_choice().saturating_add(1) >= self.choices_count()
    }

    /// Undoes the current message edit by showing the previous revision.
    pub fn undo(&mut self) {
        if let Some(message) = self.choices.get_mut(self.current) {
            message.undo();
        }
    }

    /// Redoes the current message edit by showing the next revision.
    pub fn redo(&mut self) {
        if let Some(message) = self.choices.get_mut(self.current) {
            message.redo();
        }
    }

    /// Returns the currently chosen message.
    #[must_use]
    pub fn chosen_message(&self) -> &Message {
        self.choices
            .get(self.current)
            .unwrap_or_else(|| self.choices.first())
    }

    /// Returns the text of the chosen reply's current revision (its first line).
    #[must_use]
    pub fn chosen_content(&self) -> &str {
        self.chosen_message().chosen_revision().head().content()
    }

    /// Builds the message-edit modal pre-filled with the chosen reply's current content,
    /// so the edit form opens populated with the existing text instead of blank.
    #[must_use]
    pub fn edit_modal_default(&self) -> EditMessageModal {
        EditMessageModal {
            content: self.chosen_content().to_owned(),
        }
    }

    /// Returns the Discord message ID of this history.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Sets the Discord message ID of this history.
    pub fn set_id<M: Into<MessageId>>(&mut self, id: M) {
        self.id = id.into().to_string();
    }

    /// Returns the character ID of the currently responding character.
    #[must_use]
    pub fn character(&self) -> &str {
        &self.character
    }

    /// Sets the character ID of the currently responding character.
    pub fn set_character(&mut self, character: String) {
        self.character = character;
    }

    /// Appends a message to the chat context. It is saved as part of the history on the next save.
    pub fn push<M: Into<Message>>(&mut self, value: M) {
        self.previous.push(value.into());
    }

    /// Removes all but the first choice and resets the current index to 0. The
    /// surviving first choice is the previous turn's reply, so every caller must
    /// replace it via `set_choices`/`update_current_choice` before persisting.
    pub fn reset_choices(&mut self) {
        self.choices.tail.clear();
        self.current = 0;
    }

    /// Sets the choices and resets the current index to 0.
    pub fn set_choices<M: Into<Message>>(&mut self, choices: M) {
        self.choices = NonEmpty::new(choices.into());
        self.current = 0;
    }

    /// Pushes a new choice and sets the current index to that choice.
    pub fn push_choice<M: Into<Message>>(&mut self, choice: M) {
        self.choices.push(choice.into());
        self.current = self.choices_count().saturating_sub(1);
    }

    /// Updates the current choice in place.
    pub fn update_current_choice<M: Into<Message>>(&mut self, choice: M) {
        if let Some(slot) = self.choices.get_mut(self.current) {
            *slot = choice.into();
        }
    }

    /// Marks the current choice as a failed-generation notice, so it renders as a
    /// distinct error rather than as the character speaking.
    pub fn set_current_choice_error(&mut self) {
        if let Some(choice) = self.choices.get_mut(self.current) {
            choice.set_error(true);
        }
    }

    /// Begins a new reply turn: the chosen reply and the user message join the
    /// context, the old swipeable choices are cleared, and the reply is marked
    /// as not yet finished.
    ///
    /// An empty chosen reply (the "skip the greeting" choice) is omitted from the context so the
    /// user message becomes the first turn, rather than injecting a blank turn.
    pub fn begin_new_turn<M: Into<Message>>(&mut self, user_message: M) {
        if !self
            .chosen_message()
            .chosen_revision()
            .head()
            .content()
            .is_empty()
        {
            self.push(self.chosen_message().to_owned());
        }
        self.push(user_message);
        self.reset_choices();
        self.set_finished(false);
    }

    /// Hands the conversation to another character: the chosen reply joins the
    /// context, the old choices are cleared, the responding character switches,
    /// the hand-off system message joins the context, and the reply is marked as
    /// not yet finished.
    pub fn begin_handoff<M: Into<Message>>(&mut self, character: String, system_message: M) {
        self.push(self.chosen_message().to_owned());
        self.reset_choices();
        self.set_character(character);
        self.push(system_message);
        self.set_finished(false);
    }

    /// Returns the previous messages (the chat context), in order.
    #[must_use]
    pub fn previous_messages(&self) -> &[Message] {
        &self.previous
    }

    /// Merges image descriptions into the context messages so they are saved with the turn.
    ///
    /// Replaces the old store-side caching: each previous message takes descriptions for its own
    /// attachments, so a first-turn image is persisted on the very first reply and older images keep
    /// their descriptions across turns. Each message only takes descriptions for its own
    /// attachments, so passing the whole set is safe.
    pub fn apply_descriptions(&mut self, descriptions: &[DescribedAttachment]) {
        for message in &mut self.previous {
            message.add_descriptions(descriptions);
        }
    }

    /// Converts the in-memory history into its persisted form, embedding the full messages inline.
    #[must_use]
    pub fn into_stored(self) -> StoredHistory {
        StoredHistory {
            id: self.id,
            character: self.character,
            previous: self.previous,
            choices: self.choices.into_iter().collect(),
            current: self.current,
        }
    }

    /// Rebuilds an in-memory history from its persisted form, returning `None` if it stored no
    /// choices (a corrupt record), since a history must have at least one swipeable reply.
    #[must_use]
    pub fn hydrate(stored: StoredHistory) -> Option<Self> {
        let choices = NonEmpty::from_vec(stored.choices)?;
        // clamp the chosen index to what is actually available rather than dangling past the end
        let current = stored.current.min(choices.len().saturating_sub(1));
        Some(Self {
            id: stored.id,
            character: stored.character,
            choices,
            current,
            has_finished: true,
            show_continue: true,
            previous: stored.previous,
        })
    }
}

/// Creates a new [`History`] from a character and message ID.
///
/// The conversation starts empty (the scaffolding is derived at request time). Two swipe choices
/// are seeded: the greeting (shown by default) and an empty "skip the greeting" choice, so the user
/// can swipe to start the conversation with their own message first, leaving the character unprimed.
impl From<(&Character, MessageId)> for History {
    fn from((character, id): (&Character, MessageId)) -> Self {
        let greeting = Message::new_assistant(character.greeting(), character);
        let skip_greeting = Message::new_assistant("", character);
        let choices = NonEmpty {
            head: greeting,
            tail: vec![skip_greeting],
        };

        Self::builder()
            .choices(choices)
            .character(character.id())
            .id(id)
            .build()
    }
}

/// Tests for in-memory history navigation.
#[cfg(test)]
mod tests {
    use super::{History, StoredHistory};
    use crate::constants::CHARACTER_LIMIT;
    use crate::models::{
        character::Character,
        message::{DescribedAttachment, Message, Role},
    };
    use nonempty::NonEmpty;
    use serenity::all::{MessageId, UserId};

    /// `apply_descriptions` reaches the context messages, so a first-turn image description is
    /// carried into the persisted turn rather than lost.
    #[test]
    fn apply_descriptions_reaches_context_messages() {
        let user = Message::builder()
            .id(MessageId::new(5))
            .parts(("Alice".to_owned(), "Alice: hi".to_owned(), Role::User))
            .attachments(vec!["https://cdn/x.png".to_owned()])
            .build();
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("char")
            .choices(NonEmpty::new(Message::new_system("greeting")))
            .previous(vec![user])
            .build();

        history.apply_descriptions(&[DescribedAttachment {
            url: "https://cdn/x.png".to_owned(),
            description: "en bild".to_owned(),
        }]);

        assert_eq!(
            history
                .previous_messages()
                .first()
                .and_then(|message| message.description_for("https://cdn/x.png")),
            Some("en bild"),
            "the description reaches the context message so it persists with the turn"
        );
    }

    /// `apply_descriptions` only reaches a message that owns the attachment URL, so
    /// a description never bleeds onto a context message that lacks it.
    #[test]
    fn apply_descriptions_only_reaches_owning_messages() {
        let owner = Message::builder()
            .id(MessageId::new(5))
            .parts(("Alice".to_owned(), "Alice: hi".to_owned(), Role::User))
            .attachments(vec!["https://cdn/x.png".to_owned()])
            .build();
        let other = Message::builder()
            .id(MessageId::new(6))
            .parts(("Bob".to_owned(), "Bob: yo".to_owned(), Role::User))
            .attachments(vec!["https://cdn/y.png".to_owned()])
            .build();
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("char")
            .choices(NonEmpty::new(Message::new_system("greeting")))
            .previous(vec![owner, other])
            .build();

        history.apply_descriptions(&[DescribedAttachment {
            url: "https://cdn/x.png".to_owned(),
            description: "en bild".to_owned(),
        }]);

        let previous = history.previous_messages();
        assert_eq!(
            previous
                .first()
                .and_then(|message| message.description_for("https://cdn/x.png")),
            Some("en bild"),
            "the owning message takes its own attachment's description"
        );
        assert_eq!(
            previous
                .get(1)
                .and_then(|message| message.description_for("https://cdn/x.png")),
            None,
            "a message that does not own the url gets no description for it"
        );
    }

    /// Builds a minimal character with no name-similarity score.
    fn character() -> Character {
        Character::builder()
            .id("id".to_owned())
            .name("Harry")
            .greeting("hi")
            .creator(UserId::new(1))
            .build()
    }

    /// Collects the IDs of a history's previous (context) messages, in order.
    fn previous_ids(history: &History) -> Vec<String> {
        history
            .previous_messages()
            .iter()
            .map(|message| message.id().to_owned())
            .collect()
    }

    /// Builds a history with `count` swipeable choices.
    fn history_with_choices(count: usize) -> History {
        let mut choices = NonEmpty::new(Message::new_system("choice"));
        for _ in 1..count {
            choices.push(Message::new_system("choice"));
        }
        History::builder()
            .id(MessageId::new(1))
            .character("character-id")
            .choices(choices)
            .build()
    }

    /// Builds a history whose single choice is an assistant reply with the given content.
    fn history_with_reply(content: &str) -> History {
        let reply = Message::new_assistant(content, &character());
        History::builder()
            .id(MessageId::new(1))
            .character("character-id")
            .choices(NonEmpty::new(reply))
            .build()
    }

    /// The edit modal is pre-filled with the chosen reply's full content, including every line,
    /// so a multi-line reply opens whole rather than truncated to its first line.
    #[test]
    fn edit_modal_default_prefills_the_full_chosen_reply() {
        let history = history_with_reply("first line\nsecond line");
        assert_eq!(
            history.edit_modal_default().content,
            "first line\nsecond line",
            "the edit modal is pre-filled with the chosen reply's full, multi-line content"
        );
    }

    /// The pre-filled content tracks the currently shown revision across edits and undo.
    #[test]
    fn edit_modal_default_follows_edits_and_undo() {
        let mut history = history_with_reply("original");
        history.edit_content("Harry", "edited", None::<UserId>);
        assert_eq!(
            history.edit_modal_default().content,
            "edited",
            "after an edit the modal pre-fills with the newest revision"
        );
        history.undo();
        assert_eq!(
            history.edit_modal_default().content,
            "original",
            "after an undo the modal pre-fills with the now-current revision"
        );
    }

    /// Re-submitting the pre-filled content unchanged leaves the displayed reply identical.
    #[test]
    fn editing_with_the_prefilled_default_round_trips() {
        let mut history = history_with_reply("keep me");
        let default = history.edit_modal_default().content;
        history.edit_content("Harry", default, None::<UserId>);
        assert_eq!(
            history.chosen_content(),
            "keep me",
            "re-submitting the pre-filled content unchanged round-trips to the same reply"
        );
    }

    /// A finished, genuine, non-empty reply with room to grow is continuable.
    #[test]
    fn continue_available_for_a_finished_genuine_reply() {
        let history = history_with_reply("a real reply");
        assert!(
            history.continue_available(),
            "a finished non-empty reply below the limit can be continued"
        );
    }

    /// A reply still streaming cannot be continued yet.
    #[test]
    fn continue_unavailable_while_unfinished() {
        let mut history = history_with_reply("partial");
        history.set_finished(false);
        assert!(
            !history.continue_available(),
            "an unfinished reply is not continuable"
        );
    }

    /// A failed-generation reply is not a genuine reply, so it cannot be continued.
    #[test]
    fn continue_unavailable_for_an_error_reply() {
        let mut history = history_with_reply("trasigt");
        history.set_current_choice_error();
        assert!(
            !history.continue_available(),
            "an error notice is not continuable"
        );
    }

    /// An empty reply has nothing to extend, so it is not continuable.
    #[test]
    fn continue_unavailable_for_an_empty_reply() {
        let history = history_with_reply("");
        assert!(
            !history.continue_available(),
            "an empty reply is not continuable"
        );
    }

    /// A reply already at the character limit has no room to grow, so it is not
    /// continuable; one character short of the limit still is.
    #[test]
    fn continue_unavailable_at_the_character_limit() {
        let at_limit = history_with_reply(&"x".repeat(CHARACTER_LIMIT));
        assert!(
            !at_limit.continue_available(),
            "a reply at the character limit cannot be continued"
        );
        let below_limit = history_with_reply(&"x".repeat(CHARACTER_LIMIT.saturating_sub(1)));
        assert!(
            below_limit.continue_available(),
            "a reply one character short of the limit can still be continued"
        );
    }

    /// `previous` steps the current index backward and wraps past the first choice.
    #[test]
    fn previous_cycles_backward_and_wraps() {
        let mut history = history_with_choices(3);
        assert_eq!(
            history.current_choice(),
            0,
            "a fresh history starts on the first choice"
        );
        history.previous();
        assert_eq!(
            history.current_choice(),
            2,
            "previous on the first choice wraps to the last"
        );
        history.previous();
        assert_eq!(
            history.current_choice(),
            1,
            "previous steps backward by one"
        );
        history.previous();
        assert_eq!(
            history.current_choice(),
            0,
            "previous returns to the first choice"
        );
    }

    /// `next` advances by one and, past the last choice, leaves a dangling index
    /// that `chosen_message` safely falls back to the first choice for.
    #[test]
    fn next_advances_and_dangles_past_the_last_choice() {
        let mut choices = NonEmpty::new(Message::new_system("first"));
        choices.push(Message::new_system("second"));
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("character-id")
            .choices(choices)
            .build();
        history.next();
        assert_eq!(
            history.current_choice(),
            1,
            "next advances to the second choice"
        );
        assert!(
            history.is_on_last_choice(),
            "the second of two choices is the last"
        );
        assert_eq!(
            history.chosen_content(),
            "second",
            "the chosen content tracks the advanced index"
        );
        history.next();
        assert_eq!(
            history.current_choice(),
            2,
            "advancing past the last choice leaves the index dangling for swipe-to-generate"
        );
        assert_eq!(
            history.chosen_content(),
            "first",
            "a dangling index falls back to the first choice rather than panicking"
        );
    }

    /// `update_current_choice` replaces only the current slot, leaving its index
    /// and the sibling choices untouched.
    #[test]
    fn update_current_choice_replaces_only_the_current_slot() {
        let mut choices = NonEmpty::new(Message::new_system("zero"));
        choices.push(Message::new_system("one"));
        choices.push(Message::new_system("two"));
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("character-id")
            .choices(choices)
            .current(1_usize)
            .build();
        history.update_current_choice(Message::new_system("replaced"));
        assert_eq!(
            history.current_choice(),
            1,
            "the current index is unchanged"
        );
        assert_eq!(
            history.chosen_content(),
            "replaced",
            "the current slot holds the new choice"
        );
        history.previous();
        assert_eq!(
            history.chosen_content(),
            "zero",
            "the earlier sibling is untouched"
        );
        history.previous();
        assert_eq!(
            history.chosen_content(),
            "two",
            "the later sibling is untouched"
        );
    }

    /// `push_choice` appends a distinct choice, selects it, and leaves the earlier
    /// choices unchanged.
    #[test]
    fn push_choice_appends_and_selects_the_new_choice() {
        let mut choices = NonEmpty::new(Message::new_system("zero"));
        choices.push(Message::new_system("one"));
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("character-id")
            .choices(choices)
            .build();
        history.push_choice(Message::new_system("two"));
        assert_eq!(history.choices_count(), 3, "the new choice is appended");
        assert_eq!(
            history.current_choice(),
            2,
            "the new choice becomes the current one"
        );
        assert_eq!(
            history.chosen_content(),
            "two",
            "the current slot holds the pushed choice"
        );
        history.previous();
        assert_eq!(
            history.chosen_content(),
            "one",
            "the earlier choices keep their content"
        );
    }

    /// An edit with an editor records who made it, and undoing back to the
    /// original revision clears the attribution.
    #[test]
    fn edit_content_records_the_editor_and_clears_on_undo() {
        let mut history = history_with_reply("original");
        history.edit_content("Harry", "edited", Some(UserId::new(7)));
        assert_eq!(
            history.chosen_message().current_editor(),
            Some(UserId::new(7)),
            "an edit records the editor"
        );
        history.undo();
        assert_eq!(
            history.chosen_message().current_editor(),
            None,
            "undoing to the original revision clears the editor"
        );
    }

    /// `is_on_last_choice` is true only when the current index is the final choice.
    #[test]
    fn last_choice_detected_at_the_end() {
        let mut history = history_with_choices(3);
        assert!(
            !history.is_on_last_choice(),
            "the first of three choices is not the last"
        );
        history.push_choice(Message::new_system("pushed"));
        assert!(
            history.is_on_last_choice(),
            "a pushed choice becomes the current and last choice"
        );
        history.previous();
        assert!(
            !history.is_on_last_choice(),
            "stepping back from the last choice is no longer last"
        );
    }

    /// A single-choice history is always on its last (and only) choice.
    #[test]
    fn single_choice_is_always_last() {
        let history = history_with_choices(1);
        assert!(
            history.is_on_last_choice(),
            "the only choice is also the last choice"
        );
    }

    /// `into_stored` embeds the full messages inline, and `hydrate` rebuilds the in-memory history
    /// from them unchanged.
    #[test]
    fn into_stored_then_hydrate_preserves_history() {
        let choice_one = Message::new_system("choice one");
        let choice_two = Message::new_system("choice two");
        let choice_two_id = choice_two.id().to_owned();
        // a third choice makes the chosen index (1) distinct from the last index (2),
        // so this round-trip would catch a hydrate that clamped to the last choice
        let choice_three = Message::new_system("choice three");
        let previous_one = Message::new_user("Alice", "hello");
        let previous_ids = vec![previous_one.id().to_owned()];

        let mut choices = NonEmpty::new(choice_one);
        choices.push(choice_two);
        choices.push(choice_three);

        let history = History::builder()
            .id(MessageId::new(42))
            .character("character-id")
            .choices(choices)
            .current(1_usize)
            .previous(vec![previous_one])
            .build();

        let stored = history.into_stored();
        assert_eq!(stored.id, "42", "the message ID is preserved as the key");
        assert_eq!(
            stored.character, "character-id",
            "the character ID is preserved"
        );
        assert_eq!(
            stored.choices.len(),
            3,
            "all three choices are stored inline"
        );
        assert_eq!(
            stored
                .previous
                .iter()
                .map(|message| message.id().to_owned())
                .collect::<Vec<String>>(),
            previous_ids,
            "the previous messages are stored inline in order"
        );
        assert_eq!(stored.current, 1, "the chosen index is preserved");

        let maybe_hydrated = History::hydrate(stored);
        assert!(maybe_hydrated.is_some(), "a history with choices hydrates");
        let Some(hydrated) = maybe_hydrated else {
            return;
        };
        assert_eq!(hydrated.id(), "42", "hydrate restores the message ID");
        assert_eq!(
            hydrated.character(),
            "character-id",
            "hydrate restores the character ID"
        );
        assert_eq!(
            hydrated.current_choice(),
            1,
            "hydrate restores the chosen index"
        );
        assert_eq!(
            hydrated
                .previous_messages()
                .iter()
                .map(|message| message.id().to_owned())
                .collect::<Vec<String>>(),
            previous_ids,
            "hydrate restores the context messages"
        );
        assert_eq!(
            hydrated.chosen_message().id(),
            choice_two_id.as_str(),
            "the chosen index points at the second choice"
        );
        assert_eq!(
            hydrated.chosen_message().chosen_revision().head().content(),
            "choice two",
            "the chosen choice's content survives the round-trip, not just its ID"
        );
        assert_eq!(
            hydrated
                .previous_messages()
                .first()
                .map(|message| message.chosen_revision().head().content()),
            Some("hello"),
            "the context message's content survives the round-trip"
        );
    }

    /// Marking the current choice as an error sets it on the chosen message and survives the
    /// stored round-trip, so a failed reply still renders as an error after the history reloads.
    #[test]
    fn current_choice_error_marks_chosen_and_persists() {
        let mut choices = NonEmpty::new(Message::new_system("a real reply"));
        choices.push(Message::new_system("the failed reply"));
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("character-id")
            .choices(choices)
            .current(1_usize)
            .build();

        history.set_current_choice_error();
        assert!(
            history.chosen_message().is_error(),
            "marking sets the error flag on the current choice"
        );

        let stored = history.into_stored();
        let maybe_hydrated = History::hydrate(stored);
        assert!(maybe_hydrated.is_some(), "a history with choices hydrates");
        let Some(hydrated) = maybe_hydrated else {
            return;
        };
        assert!(
            hydrated.chosen_message().is_error(),
            "the error flag survives the stored round-trip"
        );
        assert!(
            !hydrated.choices.first().is_error(),
            "only the failed choice is marked, not the genuine one"
        );
    }

    /// A stored history with no choices is a corrupt record, so `hydrate` returns `None` rather than
    /// fabricating an empty choices list.
    #[test]
    fn hydrate_rejects_a_history_without_choices() {
        let stored = StoredHistory {
            id: "42".to_owned(),
            character: "character-id".to_owned(),
            previous: Vec::new(),
            choices: Vec::new(),
            current: 0,
        };
        assert!(
            History::hydrate(stored).is_none(),
            "a history with no choices does not hydrate"
        );
    }

    /// `hydrate` clamps a stored `current` index that points past the available choices to the last
    /// choice rather than leaving it dangling past the end.
    #[test]
    fn hydrate_clamps_current_past_the_available_choices() {
        let stored = StoredHistory {
            id: "42".to_owned(),
            character: "character-id".to_owned(),
            previous: Vec::new(),
            choices: vec![
                Message::new_system("first choice"),
                Message::new_system("second choice"),
            ],
            current: 5,
        };

        let maybe_hydrated = History::hydrate(stored);
        assert!(maybe_hydrated.is_some(), "a history with choices hydrates");
        let Some(hydrated) = maybe_hydrated else {
            return;
        };
        assert_eq!(
            hydrated.current_choice(),
            1,
            "current is clamped to the last choice, not the stored index"
        );
    }

    /// Pins the turn-begin sequence run by the message handler: pushing the
    /// chosen reply and the user message into the context, clearing the old
    /// choices, and marking the reply unfinished.
    #[test]
    fn begin_new_turn_extends_context_and_resets_choices() {
        let mut history = history_with_choices(3);
        history.previous();
        assert_eq!(
            history.current_choice(),
            2,
            "the history starts on a non-first choice to prove the reset"
        );
        let chosen_id = history.chosen_message().id().to_owned();
        let user = Message::new_user("Alice", "hello");
        let user_id = user.id().to_owned();

        history.begin_new_turn(user);

        assert_eq!(
            history.choices_count(),
            1,
            "the old swipeable choices are cleared down to one"
        );
        assert_eq!(
            history.current_choice(),
            0,
            "the current index resets to the first choice"
        );
        assert_eq!(
            previous_ids(&history),
            [chosen_id, user_id],
            "the chosen reply then the user message are appended to the context"
        );
        assert!(
            !history.has_finished,
            "the reply is marked as not yet finished"
        );
    }

    /// Pins the hand-off sequence run by the character-select handler: pushing
    /// the chosen reply, clearing choices, switching the responding character,
    /// pushing the system prompt, and marking the reply unfinished.
    #[test]
    fn handoff_switches_character_and_resets_choices() {
        let mut history = history_with_choices(3);
        history.previous();
        let chosen_id = history.chosen_message().id().to_owned();
        let system = Message::new_user("System", "Svara nu som X.");
        let system_id = system.id().to_owned();

        history.begin_handoff("new-character-id".to_owned(), system);

        assert_eq!(
            history.character(),
            "new-character-id",
            "the responding character switches to the new one"
        );
        assert_eq!(
            history.choices_count(),
            1,
            "the old swipeable choices are cleared down to one"
        );
        assert_eq!(
            history.current_choice(),
            0,
            "the current index resets to the first choice"
        );
        assert_eq!(
            previous_ids(&history),
            [chosen_id, system_id],
            "the chosen reply then the hand-off system prompt are appended to the context"
        );
        assert!(
            !history.has_finished,
            "the reply is marked as not yet finished"
        );
    }

    /// A fresh history seeds two swipe choices: the greeting (a clean,
    /// unedited original shown by default) and an empty "skip the greeting" choice.
    #[test]
    fn fresh_history_seeds_a_greeting_and_skip_choice() {
        let character = character();
        let history = History::from((&character, MessageId::new(1)));
        assert_eq!(
            history.choices_count(),
            2,
            "the greeting and the skip-greeting choice are both seeded"
        );
        assert_eq!(
            history.current_choice(),
            0,
            "the greeting is shown by default"
        );
        assert_eq!(
            history.chosen_message().revisions_count(),
            0,
            "the greeting is a clean original, not a fabricated edit"
        );
    }

    /// Keeping the greeting choice includes it in the LLM context.
    #[test]
    fn keeping_the_greeting_includes_it_in_context() {
        let character = character();
        let mut history = History::from((&character, MessageId::new(1)));
        let greeting_id = history.chosen_message().id().to_owned();
        let user = Message::new_user("Alice", "hello");
        let user_id = user.id().to_owned();

        history.begin_new_turn(user);

        assert_eq!(
            previous_ids(&history),
            [greeting_id, user_id],
            "the greeting then the user message form the context"
        );
    }

    /// Swiping to the empty skip choice omits the greeting from the LLM context,
    /// so the user message becomes the first turn.
    #[test]
    fn skipping_the_greeting_omits_it_from_context() {
        let character = character();
        let mut history = History::from((&character, MessageId::new(1)));
        let user = Message::new_user("Alice", "hello");
        let user_id = user.id().to_owned();

        history.previous();
        assert_eq!(
            history.current_choice(),
            1,
            "swiping back from the greeting lands on the skip choice"
        );

        history.begin_new_turn(user);

        assert_eq!(
            previous_ids(&history),
            [user_id],
            "the greeting is omitted; the user message is the first context turn"
        );
    }
}
