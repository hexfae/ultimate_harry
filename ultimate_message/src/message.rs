use async_openai::types::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionResponse,
};
use jiff::Zoned;
use serde::{Deserialize, Serialize};
use serenity::all::{Message as DiscordMessage, UserId};
use snafu::{OptionExt, Snafu};
use ulid::Ulid;
use ultimate_character::{CHARACTERS, Character};
use ultimate_config::CONFIG;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("The OpenAI response has no choices"))]
    NoChoices,
    #[snafu(display("The OpenAI response has no content"))]
    NoContent,
}

// TODO: remove debug from everything
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    content: String,
    role: Role,
    timestamp: Zoned,
    edits: Vec<MessageEdit>,
    /// The message "revision," 0 is the original (unedited) message, 1 is the
    /// first edit, 2 is the second edit, etc.
    chosen_revision: usize,
    original: Original,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageEdit {
    timestamp: Zoned,
    content: String,
    author: UserId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Role {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Original {
    Discord {
        user_id: UserId,
        // TODO: maybe store message ids and make a MESSAGES file/static instead?
        message: Box<DiscordMessage>,
    },
    OpenAi {
        // TODO: maybe store character ids (ulids) instead?
        character: Ulid,
        response: CreateChatCompletionResponse,
    },
    Character {
        character_id: Ulid,
    },
    ExampleMessage {
        user_id: UserId,
    },
    System,
}

impl Message {
    pub fn new_assistant(content: impl Into<String>, id: impl Into<Ulid>) -> Self {
        let content = content.into();
        let id = id.into();
        Self {
            content,
            role: Role::Assistant,
            timestamp: Zoned::now(),
            edits: Vec::new(),
            chosen_revision: 0,
            original: Original::Character { character_id: id },
        }
    }

    pub fn new_user(content: impl Into<String>, user_id: impl Into<UserId>) -> Self {
        let content = content.into();
        let user_id = user_id.into();
        Self {
            content,
            role: Role::User,
            timestamp: Zoned::now(),
            edits: Vec::new(),
            chosen_revision: 0,
            original: Original::ExampleMessage { user_id },
        }
    }

    pub fn new_system(content: impl Into<String>) -> Self {
        let content = content.into();
        Self {
            content,
            role: Role::System,
            timestamp: Zoned::now(),
            edits: Vec::new(),
            chosen_revision: 0,
            original: Original::System,
        }
    }

    pub fn edit(&mut self, content: impl Into<String>, author: impl Into<UserId>) {
        self.edits.push(MessageEdit {
            timestamp: Zoned::now(),
            content: content.into(),
            author: author.into(),
        });
        self.chosen_revision = self.edits.len();
    }

    pub fn undo(&mut self) -> MessageEdit {
        if self.chosen_revision == 0 {
            self.chosen_revision = self.edits.len();
        } else {
            self.chosen_revision -= 1;
        }
        self.edits[self.chosen_revision].clone()
    }

    pub fn get_version_content(&self, version: impl Into<usize> + Copy) -> String {
        match version.into() {
            0 => self.content.clone(),
            _ => self.edits[version.into() - 1].content.clone(),
        }
    }

    #[must_use]
    pub const fn revisions_len(&self) -> usize {
        self.edits.len()
    }

    pub fn editor_of_version(&self, version: impl Into<usize> + Copy) -> Option<UserId> {
        match version.into() {
            0 => None,
            _ => Some(self.edits[version.into() - 1].author),
        }
    }

    pub const fn set_revision(&mut self, revision: usize) {
        self.chosen_revision = revision;
    }
}

impl TryFrom<(Character, CreateChatCompletionResponse)> for Message {
    type Error = Error;

    fn try_from(input: (Character, CreateChatCompletionResponse)) -> Result<Self, Self::Error> {
        let (character, response) = input;
        Ok(Self {
            content: response.clone().try_into_string()?,
            role: Role::Assistant,
            timestamp: Zoned::now(),
            edits: Vec::new(),
            chosen_revision: 0,
            original: Original::from((character, response)),
        })
    }
}

impl From<DiscordMessage> for Message {
    fn from(input: DiscordMessage) -> Self {
        let role = if input.content.to_lowercase().starts_with("system:") {
            Role::System
        } else {
            Role::User
        };
        Self {
            content: input.content.clone(),
            role,
            timestamp: Zoned::now(),
            // TODO: should this be some? i don't think so, because discord
            // doesn't store the original content of an edited message (at
            // least not such that i can see it)
            edits: Vec::new(),
            chosen_revision: 0,
            original: Original::from(input),
        }
    }
}

impl From<Message> for ChatCompletionRequestMessage {
    fn from(input: Message) -> Self {
        match input.original {
            Original::Discord { user_id, .. } | Original::ExampleMessage { user_id, .. } => {
                Self::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text(input.content),
                    name: Some(CONFIG.read().substitute_name(user_id)),
                })
            }
            Original::OpenAi { character, .. } => {
                let name = CHARACTERS
                    .read()
                    .get_by_id(character)
                    .map(|character| character.name().to_owned());

                Self::Assistant(ChatCompletionRequestAssistantMessage {
                    content: Some(input.content.into()),
                    name,
                    ..Default::default()
                })
            }
            Original::Character { character_id: id } => {
                let name = CHARACTERS
                    .read()
                    .get_by_id(id)
                    .map(|character| character.name().to_owned());
                Self::Assistant(ChatCompletionRequestAssistantMessage {
                    content: Some(input.content.into()),
                    name,
                    ..Default::default()
                })
            }
            Original::System => Self::System(ChatCompletionRequestSystemMessage {
                content: input.content.into(),
                name: Some("System".to_owned()),
            }),
        }
    }
}

impl From<DiscordMessage> for Original {
    fn from(input: DiscordMessage) -> Self {
        Self::Discord {
            user_id: input.author.id,
            message: Box::new(input),
        }
    }
}

impl From<(Character, CreateChatCompletionResponse)> for Original {
    fn from(input: (Character, CreateChatCompletionResponse)) -> Self {
        let (character, response) = input;
        Self::OpenAi {
            character: character.id(),
            response,
        }
    }
}

trait TryToString {
    fn try_into_string(self) -> Result<String, Error>;
}

impl TryToString for CreateChatCompletionResponse {
    fn try_into_string(self) -> Result<String, Error> {
        self.choices
            .first()
            .context(NoChoicesSnafu)?
            .message
            .content
            .clone()
            .context(NoContentSnafu)
    }
}
