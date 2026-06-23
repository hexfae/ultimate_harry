//! The history model: a conversation between a user and a character.
//!
//! A new [`History`] is saved per bot reply, keyed by that reply's Discord message ID, so a button
//! interaction or a user reply can find the conversation from the message it acted on.
//!
//! ## Normalized storage
//!
//! History is stored in two pieces so the same message is never written twice and the derivable
//! scaffolding is never written at all:
//!
//! - Each [`Message`] lives once in its own `native_db` table, keyed by its ID.
//! - [`StoredHistory`] (the persisted form) holds only ordered ID lists: `previous` (the chat
//!   context) and `choices` (the swipeable replies for this turn).
//! - The system-prompt scaffolding (the Swedish roleplay framing built by [`scaffolding`]) is not
//!   stored; it is rebuilt from the character at request time. Characters are versioned and a
//!   history references a specific version, so the rebuilt scaffolding always matches.
//!
//! This replaced an older design where every history stored the whole conversation (plus
//! scaffolding) inline, which duplicated messages across the many histories of one conversation and
//! grew quadratically.
//!
//! ## In-memory vs stored
//!
//! [`History`] (this in-memory form, used by the chat loop, interaction handlers, and rendering)
//! differs from [`StoredHistory`]:
//!
//! - `choices` are hydrated into full [`Message`]s, because the swipe/edit/undo/redo/regenerate
//!   handlers operate on them directly. Rendering also only needs `choices` + the character.
//! - `previous` stays as IDs; the full messages are resolved (and the scaffolding prepended) only
//!   when building the LLM context, via [`Database::build_context`](crate::database::Database::build_context).
//! - `pending` buffers messages pushed this turn so they can be written to the message table on the
//!   next save.
//!
//! [`History::into_stored`] (on save) and [`History::hydrate`] (on load) bridge the two forms;
//! `Database::history` / `Database::upsert_history` perform the message-table resolution.

use bon::Builder;
use native_db::{ToKey as _, native_db};
use native_model::{Model as _, native_model};
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use serenity::all::{MessageId, UserId};

use crate::{
    models::{
        character::Character,
        message::{DescribedAttachment, Message},
    },
    util::wrapping_previous,
};

mod render;

/// The system message that precedes every conversation.
const SYSTEM_MESSAGE: &str = "Du kommer nu att gå in i ett rollspel med en användare. Under inga omständigheter får du bryta rollspelet, gå ur karaktär, eller prata åt användaren.";

/// The system message that precedes the bot's personality.
const BEGIN_PERSONALITY: &str = "Beskriv nu karaktären du ska rollspela som.";

/// The system message that precedes the prompt.
const BEGIN_PROMPT: &str = "Detta är dina instruktioner som du ska följa under hela rollspelet: ";

/// The system message that precedes the scenario.
const BEGIN_SCENARIO: &str = "Detta är scenen du och användaren finner er själva i: ";

/// The system message that precedes the example messages.
const BEGIN_EXAMPLE_MESSAGES: &str = "Det följande är exempel på hur du ska prata med användaren.";

/// The system message that gets placed between every example message.
const EXAMPLE_MESSAGE_SEPARATOR: &str = "Nytt exempelmeddelande.";

/// The system that precedes the conversation actually beginning.
const BEGIN_MESSAGE: &str = "Rollspelet börjar nu. Efter denna punkt får du inte längra avbryta rollspelet, gå ur karaktär, eller skriva åt användaren.";

/// A log of messages between the user and a character.
///
/// This is the in-memory form used by the chat loop, interaction handlers, and rendering.
/// It is persisted as a [`StoredHistory`] (which holds message IDs, not inline messages); the
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
    /// The IDs of the previous messages (the chat context), resolved from the message table on
    /// demand. Contains no scaffolding.
    #[builder(default)]
    previous: Vec<String>,
    /// Messages introduced this turn that must be persisted on save. Not part of the stored shape.
    #[builder(default)]
    pending: Vec<Message>,
}

/// The persisted form of a [`History`]: only message IDs, no inline messages and no scaffolding.
#[derive(Debug, Serialize, Deserialize)]
#[native_model(id = 2, version = 1, with = crate::codec::Json)]
#[native_db]
#[expect(
    clippy::module_name_repetitions,
    reason = "StoredHistory is the persisted counterpart of History and belongs in this module"
)]
pub struct StoredHistory {
    /// The Discord Message ID of this history, used as the primary key.
    #[primary_key]
    pub id: String,
    /// The ulid ID of the currently responding character.
    pub character: String,
    /// The IDs of the previous messages (the chat context), in order.
    pub previous: Vec<String>,
    /// The IDs of the current swipeable responses.
    pub choices: Vec<String>,
    /// The index of the current chosen response.
    pub current: usize,
}

/// Builds the system-prompt scaffolding messages for a character.
///
/// These frame the roleplay (system message, personality, prompt, scenario, example messages,
/// system prompt) and are rebuilt from the character at request time rather than stored. The
/// generated message IDs are throwaway, since scaffolding is never persisted.
pub fn scaffolding(character: &Character) -> Vec<Message> {
    let mut messages = vec![Message::new_system(SYSTEM_MESSAGE)];

    if let Some(personality) = character.personality() {
        messages.push(Message::new_system(BEGIN_PERSONALITY));
        messages.push(Message::new_assistant(personality, character));
    }

    if let Some(prompt) = character.prompt() {
        let mut begin_prompt = BEGIN_PROMPT.to_owned();
        begin_prompt.push_str(prompt);
        messages.push(Message::new_system(begin_prompt));
    }

    if let Some(scenario) = character.scenario() {
        let mut begin_scenario = BEGIN_SCENARIO.to_owned();
        begin_scenario.push_str(scenario);
        messages.push(Message::new_system(begin_scenario));
    }

    if !character.example_messages().is_empty() {
        messages.push(Message::new_system(BEGIN_EXAMPLE_MESSAGES));
        for (user_message, assistant_message) in character.example_messages() {
            if let Some(content) = user_message {
                messages.push(Message::new_user("Användaren", content));
            }
            messages.push(Message::new_assistant(assistant_message, character));
            messages.push(Message::new_system(EXAMPLE_MESSAGE_SEPARATOR));
        }
    }

    if let Some(system_prompt) = character.system_prompt() {
        messages.push(Message::new_system(system_prompt));
    }

    messages.push(Message::new_system(BEGIN_MESSAGE));
    messages
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

    /// Appends a message to the chat context and queues it for persistence on the next save.
    pub fn push<M: Into<Message>>(&mut self, value: M) {
        let message = value.into();
        self.previous.push(message.id().to_owned());
        self.pending.push(message);
    }

    /// Resets the choices, removing all but the first choice, and resets the current index to 0.
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

    /// Returns the IDs of the previous messages (the chat context), in order.
    #[must_use]
    pub fn previous_ids(&self) -> &[String] {
        &self.previous
    }

    /// Returns the messages queued for persistence (those pushed this turn, not yet in the table).
    #[must_use]
    pub fn pending(&self) -> &[Message] {
        &self.pending
    }

    /// Merges image descriptions into the pending messages so they are saved with the turn.
    ///
    /// A pending message (the user's just-sent message) is not yet in the message table, so caching
    /// onto the table is a no-op for it; applying here instead lets [`Self::into_stored`] persist the
    /// description on the very first reply. Each message only takes descriptions for its own
    /// attachments, so passing the whole set is safe.
    pub fn apply_descriptions(&mut self, descriptions: &[DescribedAttachment]) {
        for message in &mut self.pending {
            message.add_descriptions(descriptions);
        }
    }

    /// Converts the in-memory history into its persisted form plus the messages that must be
    /// written to the message table (the queued `pending` messages and the current choices).
    #[must_use]
    pub fn into_stored(self) -> (StoredHistory, Vec<Message>) {
        let choices = self
            .choices
            .iter()
            .map(|message| message.id().to_owned())
            .collect();
        let mut messages = self.pending;
        messages.extend(self.choices);
        let stored = StoredHistory {
            id: self.id,
            character: self.character,
            previous: self.previous,
            choices,
            current: self.current,
        };
        (stored, messages)
    }

    /// Rebuilds an in-memory history from its persisted form and its resolved choice messages.
    #[must_use]
    pub fn hydrate(stored: StoredHistory, choices: NonEmpty<Message>) -> Self {
        // choices can be shorter than the stored list if some message records failed to resolve,
        // so clamp the chosen index to what is actually available rather than dangling past the end
        let current = stored.current.min(choices.len().saturating_sub(1));
        Self {
            id: stored.id,
            character: stored.character,
            choices,
            current,
            has_finished: true,
            previous: stored.previous,
            pending: Vec::new(),
        }
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
    use super::{
        BEGIN_EXAMPLE_MESSAGES, BEGIN_MESSAGE, History, SYSTEM_MESSAGE, StoredHistory, scaffolding,
    };
    use crate::models::{
        character::Character,
        message::{DescribedAttachment, Message, Role},
    };
    use nonempty::NonEmpty;
    use serenity::all::{MessageId, UserId};

    /// `apply_descriptions` reaches the pending messages, so a first-turn image description is
    /// carried into the persisted turn rather than lost.
    #[test]
    fn apply_descriptions_reaches_pending_messages() {
        let user = Message::builder()
            .id(MessageId::new(5))
            .parts(("Alice".to_owned(), "Alice: hi".to_owned(), Role::User))
            .attachments(vec!["https://cdn/x.png".to_owned()])
            .build();
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("char")
            .choices(NonEmpty::new(Message::new_system("greeting")))
            .pending(vec![user])
            .build();

        history.apply_descriptions(&[DescribedAttachment {
            url: "https://cdn/x.png".to_owned(),
            description: "en bild".to_owned(),
        }]);

        assert_eq!(
            history
                .pending()
                .first()
                .and_then(|message| message.description_for("https://cdn/x.png")),
            Some("en bild"),
            "the description reaches the pending message so it persists with the turn"
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

    /// Returns the content of a scaffolding message's first part.
    fn first_part_content(message: &Message) -> &str {
        message.chosen_revision().head().content()
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

    /// `into_stored` emits ID lists plus pending-then-choice messages, and `hydrate` rebuilds
    /// the in-memory history from them unchanged.
    #[test]
    fn into_stored_then_hydrate_preserves_history() {
        let choice_one = Message::new_system("choice one");
        let choice_two = Message::new_system("choice two");
        let pending_one = Message::new_user("Alice", "hello");
        let choice_ids = vec![choice_one.id().to_owned(), choice_two.id().to_owned()];
        let choice_two_id = choice_two.id().to_owned();
        let pending_id = pending_one.id().to_owned();
        let previous_ids = vec!["prev-a".to_owned(), "prev-b".to_owned()];

        let mut choices = NonEmpty::new(choice_one.clone());
        choices.push(choice_two.clone());
        let mut hydrate_choices = NonEmpty::new(choice_one);
        hydrate_choices.push(choice_two);

        let history = History::builder()
            .id(MessageId::new(42))
            .character("character-id")
            .choices(choices)
            .current(1_usize)
            .previous(previous_ids.clone())
            .pending(vec![pending_one])
            .build();

        let (stored, messages) = history.into_stored();
        assert_eq!(stored.id, "42", "the message ID is preserved as the key");
        assert_eq!(
            stored.character, "character-id",
            "the character ID is preserved"
        );
        assert_eq!(
            stored.choices, choice_ids,
            "stored choices are the choice IDs in order"
        );
        assert_eq!(
            stored.previous, previous_ids,
            "stored previous are the context IDs in order"
        );
        assert_eq!(stored.current, 1, "the chosen index is preserved");

        let message_ids = messages
            .iter()
            .map(|message| message.id().to_owned())
            .collect::<Vec<String>>();
        let mut expected_ids = vec![pending_id];
        expected_ids.extend(choice_ids.iter().cloned());
        assert_eq!(
            message_ids, expected_ids,
            "into_stored writes pending messages first, then the choices"
        );

        let hydrated = History::hydrate(stored, hydrate_choices);
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
            hydrated.previous_ids(),
            previous_ids.as_slice(),
            "hydrate restores the context IDs"
        );
        assert!(
            hydrated.pending().is_empty(),
            "a freshly hydrated history has nothing pending"
        );
        assert_eq!(
            hydrated.chosen_message().id(),
            choice_two_id.as_str(),
            "the chosen index points at the second choice"
        );
    }

    /// When some stored choices fail to resolve, the rehydrated choice list is shorter than the
    /// stored `current` index, so hydrate must clamp it to the last available choice rather than
    /// leaving it dangling past the end.
    #[test]
    fn hydrate_clamps_current_past_the_available_choices() {
        let stored = StoredHistory {
            id: "42".to_owned(),
            character: "character-id".to_owned(),
            previous: Vec::new(),
            choices: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            current: 2,
        };
        let mut choices = NonEmpty::new(Message::new_system("only choice"));
        choices.push(Message::new_system("second choice"));

        let hydrated = History::hydrate(stored, choices);
        assert_eq!(
            hydrated.current_choice(),
            1,
            "current is clamped to the last resolvable choice, not the stored index"
        );
    }

    /// A character with no optional fields scaffolds to just the framing system message and the
    /// begin-message marker.
    #[test]
    fn scaffolding_is_minimal_for_a_bare_character() {
        let character = Character::builder()
            .id("character-id".to_owned())
            .name("Harry")
            .greeting("hello")
            .creator(UserId::new(1))
            .build();
        let messages = scaffolding(&character);
        assert_eq!(
            messages.len(),
            2,
            "a bare character scaffolds to two system messages"
        );
        assert_eq!(
            messages.first().map(first_part_content),
            Some(SYSTEM_MESSAGE),
            "the first scaffolding message frames the roleplay"
        );
        assert_eq!(
            messages.last().map(first_part_content),
            Some(BEGIN_MESSAGE),
            "the last scaffolding message marks the start of the conversation"
        );
    }

    /// Every optional field plus an example pair contributes its scaffolding messages in order.
    #[test]
    fn scaffolding_expands_with_optional_fields() {
        let character = Character::builder()
            .id("character-id".to_owned())
            .name("Harry")
            .greeting("hello")
            .creator(UserId::new(1))
            .personality("personality".to_owned())
            .prompt("prompt".to_owned())
            .scenario("scenario".to_owned())
            .system_prompt("system prompt".to_owned())
            .example_messages(vec![(Some("hi".to_owned()), "hello".to_owned())])
            .build();
        let messages = scaffolding(&character);
        assert_eq!(
            messages.len(),
            11,
            "the framing, personality pair, prompt, scenario, example pair, system prompt and \
             begin-message marker total eleven messages"
        );
        assert_eq!(
            messages.first().map(first_part_content),
            Some(SYSTEM_MESSAGE),
            "the framing system message stays first"
        );
        assert_eq!(
            messages.last().map(first_part_content),
            Some(BEGIN_MESSAGE),
            "the begin-message marker stays last"
        );
        assert!(
            messages
                .iter()
                .any(|message| first_part_content(message) == BEGIN_EXAMPLE_MESSAGES),
            "the example messages are introduced by their marker"
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
            history.previous_ids(),
            [chosen_id, user_id].as_slice(),
            "the chosen reply then the user message are appended to the context"
        );
        assert_eq!(
            history.pending().len(),
            2,
            "both pushed messages are queued for persistence"
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
            history.previous_ids(),
            [chosen_id, system_id].as_slice(),
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
            history.previous_ids(),
            [greeting_id, user_id].as_slice(),
            "the greeting then the user message form the context"
        );
        assert_eq!(
            history.pending().len(),
            2,
            "both the greeting and the user message are queued"
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
            history.previous_ids(),
            [user_id].as_slice(),
            "the greeting is omitted; the user message is the first context turn"
        );
        assert_eq!(
            history.pending().len(),
            1,
            "only the user message is queued"
        );
    }
}
