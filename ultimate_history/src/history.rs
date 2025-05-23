use async_openai::types::ChatCompletionRequestMessage;
use bon::Builder;
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use serenity::all::{MessageId, UserId};
use surrealdb::RecordId;
use ultimate_character::Character;
use ultimate_message::{Message, MessageEdit};

const SYSTEM_MESSAGE: &str = "Du kommer nu att gå in i ett rollspel med en användare. Under inga omständigheter får du bryta rollspelet, gå ur karaktär, eller prata åt användaren.";

const BEGIN_PERSONALITY: &str = "Beskriv nu karaktären du ska rollspela som.";

const BEGIN_PROMPT: &str = "Detta är dina instruktioner som du ska följa under hela rollspelet: ";

const BEGIN_SCENARIO: &str = "Detta är scenen du och användaren finner er själva i: ";

const BEGIN_EXAMPLE_MESSAGES: &str = "Det följande är exempel på hur du ska prata med användaren.";

const EXAMPLE_MESSAGE_SEPARATOR: &str = "Nytt exempelmeddelande.";

const BEGIN_MESSAGE: &str = "Rollspelet börjar nu. Efter denna punkt får du inte längra avbryta rollspelet, gå ur karaktär, eller skriva åt användaren.";

/// A log of messages between the user and a character.
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct History {
    /// The previous messages, the history of the chat.
    previous: NonEmpty<Message>,
    /// The current responses the user can pick between by "swiping" (pressing next/previous).
    choices: NonEmpty<Message>,
    /// The index of the current response the user has chosen.
    #[builder(default)]
    current: usize,
    /// The ulid ID of the currently responding character.
    #[builder(with = |id: &RecordId| id.to_owned())]
    character: RecordId,
    /// The Discord Message ID of this history.
    #[builder(with = |id: MessageId| RecordId::from(("history", id.to_string())))]
    id: RecordId,
}

impl History {
    pub fn edit_content(
        &mut self,
        author: impl Into<String>,
        content: impl Into<String>,
        editor: impl Into<UserId>,
    ) {
        self.choices[self.current].edit(author, content, editor);
    }

    pub fn set_revision(&mut self, revision: usize) {
        self.choices.last_mut().set_revision(revision);
    }

    #[must_use]
    pub fn chosen_choice(&self) -> Message {
        self.choices[self.current].clone()
    }

    pub fn undo(&mut self) -> MessageEdit {
        self.previous.last_mut().undo()
    }

    #[must_use]
    pub fn id(&self) -> RecordId {
        self.id.clone()
    }

    pub fn set_id(&mut self, id: impl Into<MessageId>) {
        self.id = RecordId::from(("history", id.into().to_string()));
    }

    #[must_use]
    pub fn last(&self) -> Message {
        self.previous.last().clone()
    }

    #[must_use]
    pub fn character(&self) -> RecordId {
        self.character.clone()
    }

    pub fn push(&mut self, message: impl Into<Message>) {
        self.previous.push(message.into());
    }

    pub fn set_choices(&mut self, choices: impl Into<Message>) {
        self.choices = NonEmpty::new(choices.into());
        self.current = 0;
    }

    #[must_use]
    pub fn testing_function() -> Self {
        Self::builder()
            .previous(NonEmpty::new(Message::new_system(SYSTEM_MESSAGE)))
            .choices(NonEmpty::new(Message::new_assistant(
                "",
                &Character::builder()
                    .name("Character")
                    .greeting("Greeting")
                    .id(RecordId::from(("character", "1")))
                    .creator(1)
                    .build(),
            )))
            .character(&RecordId::from(("character", "1")))
            .id(MessageId::from(1))
            .build()
    }
}

impl From<(Character, MessageId, UserId)> for History {
    fn from((character, id, user_id): (Character, MessageId, UserId)) -> Self {
        let mut history = NonEmpty::new(Message::new_system(SYSTEM_MESSAGE));

        if let Some(personality) = character.personality() {
            history.push(Message::new_system(BEGIN_PERSONALITY));
            history.push(Message::new_assistant(personality, &character));
        }

        if let Some(prompt) = character.prompt() {
            let mut begin_prompt = BEGIN_PROMPT.to_owned();
            begin_prompt.push_str(prompt);
            history.push(Message::new_system(begin_prompt));
        }

        if let Some(scenario) = character.scenario() {
            let mut begin_scenario = BEGIN_SCENARIO.to_owned();
            begin_scenario.push_str(scenario);

            history.push(Message::new_system(begin_scenario));
        }

        if !character.example_messages().is_empty() {
            history.push(Message::new_system(BEGIN_EXAMPLE_MESSAGES));

            for (user_message, assistant_message) in character.example_messages() {
                if let Some(user_message) = user_message {
                    history.push(Message::new_user("Användaren", user_message, user_id));
                }

                history.push(Message::new_assistant(assistant_message, &character));

                history.push(Message::new_system(EXAMPLE_MESSAGE_SEPARATOR));
            }
        }

        if let Some(system_prompt) = character.system_prompt() {
            history.push(Message::new_system(system_prompt));
        }

        history.push(Message::new_system(BEGIN_MESSAGE));

        let choices = NonEmpty::new(Message::new_assistant(character.greeting(), &character));

        Self::builder()
            .previous(history)
            .choices(choices)
            .character(character.id())
            .id(id)
            .build()
    }
}

impl From<History> for Vec<ChatCompletionRequestMessage> {
    fn from(history: History) -> Self {
        history
            .previous
            .into_iter()
            .map(|msg| Into::<NonEmpty<ChatCompletionRequestMessage>>::into(msg).into())
            .collect::<Vec<Self>>()
            .concat()
    }
}
