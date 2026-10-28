//! The LLM prompt-scaffolding for a conversation: the Swedish roleplay framing
//! rebuilt from the character at request time, plus the full-context assembly.
//!
//! Split out from the model in `history.rs`; as a child module this can still
//! reach `History`'s private fields.

use core::num::NonZeroUsize;

use crate::{
    constants::MAX_TOKENS,
    models::{character::Character, message::Message},
};
use tracing::info;

use super::History;

/// A deliberately low estimate of how many characters make up one token, for
/// sizing a conversation against a model's context window.
///
/// Swedish prose averages somewhat more than this, so the estimate overcounts
/// tokens and trims a little early rather than sending a prompt the model
/// refuses.
const CHARS_PER_TOKEN: u64 = 3;

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

/// The system message that precedes the conversation actually beginning.
const BEGIN_MESSAGE: &str = "Rollspelet börjar nu. Efter denna punkt får du inte längre avbryta rollspelet, gå ur karaktär, eller skriva åt användaren.";

/// Builds the system-prompt scaffolding messages for a character.
///
/// These frame the roleplay (system message, personality, prompt, scenario, example messages) and
/// are rebuilt from the character at request time rather than stored. The character's own system
/// prompt is deliberately not part of this front matter; [`History::build_context`] appends it
/// after the conversation instead, so it lands as the most recent instruction. The generated
/// message IDs are throwaway, since scaffolding is never persisted.
fn scaffolding(character: &Character) -> Vec<Message> {
    let mut messages = vec![Message::new_system(SYSTEM_MESSAGE)];

    if let Some(personality) = character.personality() {
        messages.push(Message::new_system(BEGIN_PERSONALITY));
        messages.push(Message::new_assistant(personality));
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
                messages.push(Message::new_user(content));
            }
            messages.push(Message::new_assistant(assistant_message));
            messages.push(Message::new_system(EXAMPLE_MESSAGE_SEPARATOR));
        }
    }

    messages.push(Message::new_system(BEGIN_MESSAGE));
    messages
}

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the prompt-scaffolding is split into this child module to separate it from the History model"
)]
impl History {
    /// Builds the full LLM context: the character's rebuilt scaffolding, then the previous messages,
    /// then the character's system prompt (if any) as the last context message.
    ///
    /// Placing the system prompt after the conversation keeps it as the most recent instruction the
    /// model sees, rather than burying it at the front before every turn.
    #[must_use]
    pub fn build_context(&self, character: &Character) -> Vec<Message> {
        self.build_context_within(character, None)
    }

    /// Like [`build_context`](Self::build_context), but when the model's context
    /// `window` (in tokens) is known, drops the oldest conversation messages
    /// until the estimated prompt leaves room for a reply.
    ///
    /// A conversation is never trimmed below its latest message, and the
    /// scaffolding and system prompt are always kept. Without this, a long
    /// roleplay eventually outgrows the window and every later reply fails.
    #[must_use]
    pub fn build_context_within(&self, character: &Character, window: Option<u64>) -> Vec<Message> {
        let mut context = scaffolding(character);
        let system_prompt = character.system_prompt().map(Message::new_system);
        let fixed = context
            .iter()
            .chain(system_prompt.as_ref())
            .map(estimated_size)
            .fold(0, usize::saturating_add);
        let room = window.map_or(usize::MAX, |tokens| {
            let chars = tokens
                .saturating_sub(MAX_TOKENS)
                .saturating_mul(CHARS_PER_TOKEN);
            usize::try_from(chars)
                .unwrap_or(usize::MAX)
                .saturating_sub(fixed)
        });
        let kept = newest_within(&self.previous, room);
        if let Some(dropped) = NonZeroUsize::new(self.previous.len().saturating_sub(kept.len())) {
            info!("dropped the {dropped} oldest messages to fit the model's context window");
        }
        context.extend(kept.iter().cloned());
        context.extend(system_prompt);
        context
    }
}

/// How many characters `message` adds to a prompt, as its transcript renders it.
fn estimated_size(message: &Message) -> usize {
    message.transcript().chars().count()
}

/// The newest run of `messages` whose estimated size fits in `room` characters,
/// keeping at least the latest message however large it is.
fn newest_within(messages: &[Message], room: usize) -> &[Message] {
    let mut total = messages
        .iter()
        .map(estimated_size)
        .fold(0, usize::saturating_add);
    let mut kept = messages;
    while total > room
        && let [oldest, rest @ ..] = kept
        && !rest.is_empty()
    {
        total = total.saturating_sub(estimated_size(oldest));
        kept = rest;
    }
    kept
}

/// Tests for the prompt-scaffolding and full-context assembly.
#[cfg(test)]
mod tests {
    use super::{
        BEGIN_EXAMPLE_MESSAGES, BEGIN_MESSAGE, CHARS_PER_TOKEN, SYSTEM_MESSAGE, estimated_size,
        newest_within, scaffolding,
    };
    use crate::constants::MAX_TOKENS;
    use crate::models::{character::Character, history::History, message::Message};
    use core::iter::repeat_with;
    use nonempty::NonEmpty;
    use serenity::all::{MessageId, UserId};

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

    /// `build_context` prepends the character scaffolding to the previous messages, in order.
    #[test]
    fn build_context_prepends_scaffolding_to_previous() {
        let character = character();
        let first = Message::new_user("hello");
        let second = Message::new_assistant("hi there");
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

    /// Builds `count` user messages of `size` characters each (as their transcript renders them).
    fn turns(count: usize, size: usize) -> Vec<Message> {
        let body = "x".repeat(size.saturating_sub("user: ".len()));
        repeat_with(|| Message::new_user(body.clone()))
            .take(count)
            .collect()
    }

    /// The newest messages that fit are kept, oldest dropped first, and a run that already fits
    /// is kept whole.
    #[test]
    fn newest_within_drops_the_oldest_first() {
        let messages = turns(5, 10);
        assert_eq!(
            messages.first().map(estimated_size),
            Some(10),
            "each fixture message renders to ten characters"
        );
        assert_eq!(
            newest_within(&messages, 50).len(),
            5,
            "an exact fit keeps everything"
        );
        assert_eq!(
            newest_within(&messages, 49).len(),
            4,
            "one character over drops one message"
        );
        let kept = newest_within(&messages, 30);
        assert_eq!(kept.len(), 3, "the three newest fit");
        assert_eq!(
            kept.first().map(Message::id),
            messages.get(2).map(Message::id),
            "it is the oldest that go"
        );
    }

    /// However little room there is, the latest message is kept so there is something to reply to.
    #[test]
    fn newest_within_always_keeps_the_latest_message() {
        let messages = turns(3, 10);
        let kept = newest_within(&messages, 0);
        assert_eq!(
            kept.iter().map(Message::id).collect::<Vec<&str>>(),
            messages
                .last()
                .map(Message::id)
                .into_iter()
                .collect::<Vec<&str>>(),
            "only the latest message survives"
        );
        assert!(
            newest_within(&[], 0).is_empty(),
            "an empty conversation stays empty"
        );
    }

    /// A known window trims the conversation to what fits beside the scaffolding and the reply
    /// budget, while an unknown window trims nothing and the system prompt always stays last.
    #[test]
    fn build_context_within_trims_to_the_window() {
        let character = Character::builder()
            .id("id".to_owned())
            .name("Harry")
            .greeting("hi")
            .creator(UserId::new(1))
            .system_prompt("the jailbreak".to_owned())
            .build();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(Message::new_system("greeting")))
            .previous(turns(10, 300))
            .build();
        let fixed = scaffolding(&character).iter().map(estimated_size).fold(
            estimated_size(&Message::new_system("the jailbreak")),
            usize::saturating_add,
        );
        let fixed_tokens = u64::try_from(fixed).unwrap_or(u64::MAX);
        let room_for_four: u64 = 1200;
        let window = MAX_TOKENS
            .saturating_add(fixed_tokens.div_ceil(CHARS_PER_TOKEN))
            .saturating_add(room_for_four.div_ceil(CHARS_PER_TOKEN));

        let trimmed = history.build_context_within(&character, Some(window));
        let untrimmed = history.build_context_within(&character, None);

        let scaffolding_len = scaffolding(&character).len();
        assert_eq!(
            trimmed.len(),
            scaffolding_len.saturating_add(4).saturating_add(1),
            "the scaffolding, the four newest turns, and the system prompt"
        );
        assert_eq!(
            untrimmed.len(),
            scaffolding_len.saturating_add(10).saturating_add(1),
            "without a window nothing is trimmed"
        );
        assert_eq!(
            trimmed.last().map(first_part_content),
            Some("the jailbreak"),
            "the system prompt stays last"
        );
    }

    /// `build_context` appends the character's system prompt after the conversation, so it is the
    /// last (most recent) context message rather than buried in the front scaffolding.
    #[test]
    fn build_context_appends_system_prompt_after_the_conversation() {
        let character = Character::builder()
            .id("character-id".to_owned())
            .name("Harry")
            .greeting("hello")
            .creator(UserId::new(1))
            .system_prompt("the jailbreak".to_owned())
            .build();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(Message::new_system("greeting")))
            .previous(vec![Message::new_user("hello")])
            .build();

        let context = history.build_context(&character);
        assert!(
            !scaffolding(&character)
                .iter()
                .any(|message| first_part_content(message) == "the jailbreak"),
            "the system prompt is no longer part of the front scaffolding"
        );
        assert_eq!(
            context.last().map(first_part_content),
            Some("the jailbreak"),
            "the system prompt is the last context message, after the conversation"
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
            10,
            "the framing, personality pair, prompt, scenario, example pair and begin-message \
             marker total ten messages (the system prompt is appended in build_context, not here)"
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
}
