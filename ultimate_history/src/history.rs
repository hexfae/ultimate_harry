use std::{
    fs::{read, write},
    path::Path,
    sync::LazyLock,
    time::Duration,
};

use async_openai::types::ChatCompletionRequestMessage;
use dashmap::DashMap;
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use serenity::all::{MessageId, UserId};
use snafu::{ResultExt, Snafu};
use tracing::warn;
use ulid::Ulid;
use ultimate_character::Character;
use ultimate_message::{Message, MessageEdit};

const SYSTEM_MESSAGE: &str = "Du kommer nu att gå in i ett rollspel med en användare. Under inga omständigheter får du bryta rollspelet, gå ur karaktär, eller prata åt användaren.";

const BEGIN_PERSONALITY: &str = "Beskriv nu karaktären du ska rollspela som.";

const BEGIN_PROMPT: &str = "Detta är dina instruktioner som du ska följa under hela rollspelet: ";

const BEGIN_SCENARIO: &str = "Detta är scenen du och användaren finner er själva i: ";

const BEGIN_EXAMPLE_MESSAGES: &str = "Det följande är exempel på hur du ska prata med användaren.";

const EXAMPLE_MESSAGE_SEPARATOR: &str = "Nytt exempelmeddelande.";

const BEGIN_MESSAGE: &str = "Rollspelet börjar nu. Efter denna punkt får du inte längra avbryta rollspelet, gå ur karaktär, eller skriva åt användaren.";

const HISTORIES_PATH: &str = "histories.ron";

pub static HISTORIES: LazyLock<Histories> =
    LazyLock::new(|| Histories::load().expect("valid histories"));

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Error while reading histories: {source}"))]
    Read { source: std::io::Error },
    #[snafu(display("Error while deserializing histories: {source}"))]
    Deserialize { source: ron::de::SpannedError },
    #[snafu(display("Error while serializing histories: {source}"))]
    Serialize { source: ron::error::Error },
    #[snafu(display("Error while writing histories: {source}"))]
    Write { source: std::io::Error },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct History {
    previous: Vec<(MessageType, Message)>,
    seconds_taken_and_choices: Vec<(Duration, (MessageType, Message))>,
    current_page: usize,
    character: Ulid,
    id: MessageId,
}

pub struct Histories(DashMap<MessageId, History>);

impl Histories {
    fn load() -> Result<Self, Error> {
        if Path::new(HISTORIES_PATH).exists() {
            read(HISTORIES_PATH)
                .context(ReadSnafu)
                .and_then(|bytes| {
                    ron::de::from_bytes::<Vec<History>>(&bytes).context(DeserializeSnafu)
                })
                .map(|histories| {
                    histories
                        .into_iter()
                        .map(|history| (history.id, history))
                        .collect::<DashMap<MessageId, History>>()
                })
                .map(Self)
        } else {
            Ok(Self::default())
        }
    }

    fn save(&self) {
        if let Err(why) = ron::ser::to_string_pretty(
            &self
                .0
                .iter()
                .map(|c| c.value().to_owned())
                .collect::<Vec<History>>(),
            PrettyConfig::new(),
        )
        .context(SerializeSnafu)
        .and_then(|string| write(HISTORIES_PATH, string).context(WriteSnafu))
        {
            warn!("Error while saving histories: {why}");
        }
    }

    pub fn insert(&self, history: History) {
        self.0.insert(history.id, history);
        self.save();
    }

    pub fn get(&self, id: MessageId) -> Option<History> {
        self.0.get(&id).map(|c| c.value().to_owned())
    }
}

impl Default for Histories {
    fn default() -> Self {
        let characters = Self(DashMap::new());
        if !Path::new(HISTORIES_PATH).exists() {
            characters.save();
        }
        characters
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub enum MessageType {
    Greeting,
    Description,
    Personality,
    Prompt,
    SystemPrompt,
    Scenario,
    StartExampleMessages,
    ExampleMessage,
    SeparateExampleMessages,
    EndExampleMessages,
    System,
    Chat,
}

impl History {
    pub fn edit_content(&mut self, content: impl Into<String>, author: impl Into<UserId>) {
        if let Some((_, msg)) = self.previous.last_mut() {
            msg.edit(content, author);
        }
    }

    pub fn set_revision(&mut self, revision: usize) {
        self.last().set_revision(revision);
    }

    pub fn undo(&mut self) -> Option<MessageEdit> {
        if let Some((_, msg)) = self.previous.last_mut() {
            Some(msg.undo())
        } else {
            None
        }
    }

    pub fn set_id(&mut self, id: impl Into<MessageId>) {
        self.id = id.into();
    }

    /// # Panics
    ///
    /// Since a history will never be empty, this will never panic.
    #[must_use]
    pub fn last(&self) -> Message {
        self.previous
            .last()
            .expect("history is never empty")
            .1
            .clone()
    }

    #[must_use]
    pub const fn character(&self) -> Ulid {
        self.character
    }

    pub fn push(&mut self, message: impl Into<Message>) {
        self.previous.push((MessageType::Chat, message.into()));
    }
}

impl From<(Character, MessageId, UserId)> for History {
    fn from((character, id, user_id): (Character, MessageId, UserId)) -> Self {
        let mut history = Vec::new();
        history.push((MessageType::System, Message::new_system(SYSTEM_MESSAGE)));

        if let Some(personality) = character.personality() {
            history.push((MessageType::System, Message::new_system(BEGIN_PERSONALITY)));
            history.push((
                MessageType::Personality,
                Message::new_assistant(personality, character.id()),
            ));
        }

        if let Some(prompt) = character.prompt() {
            let mut begin_prompt = BEGIN_PROMPT.to_owned();
            begin_prompt.push_str(prompt);
            history.push((MessageType::Prompt, Message::new_system(begin_prompt)));
        }

        if let Some(scenario) = character.scenario() {
            let mut begin_scenario = BEGIN_SCENARIO.to_owned();
            begin_scenario.push_str(scenario);

            history.push((MessageType::Scenario, Message::new_system(begin_scenario)));
        }

        if !character.example_messages().is_empty() {
            history.push((
                MessageType::StartExampleMessages,
                Message::new_system(BEGIN_EXAMPLE_MESSAGES),
            ));

            for (user_message, assistant_message) in character.example_messages() {
                if let Some(user_message) = user_message {
                    history.push((
                        MessageType::ExampleMessage,
                        Message::new_user(user_message, user_id),
                    ));
                }

                history.push((
                    MessageType::ExampleMessage,
                    Message::new_assistant(assistant_message, character.id()),
                ));

                history.push((
                    MessageType::SeparateExampleMessages,
                    Message::new_system(EXAMPLE_MESSAGE_SEPARATOR),
                ));
            }
        }

        if let Some(system_prompt) = character.system_prompt() {
            history.push((
                MessageType::SystemPrompt,
                Message::new_assistant(system_prompt, character.id()),
            ));
        }

        history.push((MessageType::System, Message::new_system(BEGIN_MESSAGE)));

        history.push((
            MessageType::Greeting,
            Message::new_assistant(character.greeting(), character.id()),
        ));

        Self {
            previous: history,
            seconds_taken_and_choices: Vec::new(),
            current_page: 0,
            character: character.id(),
            id,
        }
    }
}

impl From<History> for Vec<ChatCompletionRequestMessage> {
    fn from(history: History) -> Self {
        history
            .previous
            .into_iter()
            .map(|(_, msg)| msg.into())
            .collect()
    }
}
