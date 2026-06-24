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
//! [`History::build_context`], which prepends the rebuilt scaffolding to the `previous` messages.

use bon::Builder;
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

    /// Returns the text of the chosen reply's current revision (its first line).
    #[must_use]
    pub fn chosen_content(&self) -> &str {
        self.chosen_message().chosen_revision().head().content()
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

    /// Builds the full LLM context: the character's rebuilt scaffolding followed by the previous
    /// messages, in order.
    #[must_use]
    pub fn build_context(&self, character: &Character) -> Vec<Message> {
        let mut context = scaffolding(character);
        context.extend(self.previous.iter().cloned());
        context
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
    use super::{
        BEGIN_EXAMPLE_MESSAGES, BEGIN_MESSAGE, History, SYSTEM_MESSAGE, StoredHistory, scaffolding,
    };
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

    /// `into_stored` embeds the full messages inline, and `hydrate` rebuilds the in-memory history
    /// from them unchanged.
    #[test]
    fn into_stored_then_hydrate_preserves_history() {
        let choice_one = Message::new_system("choice one");
        let choice_two = Message::new_system("choice two");
        let choice_two_id = choice_two.id().to_owned();
        let previous_one = Message::new_user("Alice", "hello");
        let previous_ids = vec![previous_one.id().to_owned()];

        let mut choices = NonEmpty::new(choice_one);
        choices.push(choice_two);

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
        assert_eq!(stored.choices.len(), 2, "both choices are stored inline");
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
        let Some(hydrated) = maybe_hydrated else { return };
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
        let Some(hydrated) = maybe_hydrated else { return };
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
        let Some(hydrated) = maybe_hydrated else { return };
        assert_eq!(
            hydrated.current_choice(),
            1,
            "current is clamped to the last choice, not the stored index"
        );
    }

    /// `build_context` prepends the character scaffolding to the previous messages, in order.
    #[test]
    fn build_context_prepends_scaffolding_to_previous() {
        let character = character();
        let first = Message::new_user("Alice", "hello");
        let second = Message::new_assistant("hi there", &character);
        let first_id = first.id().to_owned();
        let second_id = second.id().to_owned();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(Message::new_system("greeting")))
            .previous(vec![first, second])
            .build();

        let context = history.build_context(&character);
        let scaffolding_len = scaffolding(&character).len();
        assert_eq!(
            context.len(),
            scaffolding_len.saturating_add(2),
            "context is the scaffolding plus the two previous messages"
        );
        let tail_ids = context
            .iter()
            .skip(scaffolding_len)
            .map(|message| message.id().to_owned())
            .collect::<Vec<String>>();
        assert_eq!(
            tail_ids,
            vec![first_id, second_id],
            "the previous messages follow the scaffolding in order"
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
