use async_openai::types::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    ChatCompletionRequestUserMessageContent, CreateChatCompletionResponse,
};
use bon::Builder;
use jiff::Zoned;
use serde::{Deserialize, Serialize};
use serenity::all::{Message as DiscordMessage, UserId};
use snafu::{OptionExt, Snafu};
use surrealdb::RecordId;
use ulid::Ulid;
use ultimate_character::Character;
use ultimate_config::CONFIG;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("The OpenAI response has no choices"))]
    NoChoices,
    #[snafu(display("The OpenAI response has no content"))]
    NoContent,
}

// TODO: remove debug from everything
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct Message {
    #[builder(default = RecordId::from(("message", Ulid::new().to_string())))]
    id: RecordId,
    #[builder(into)]
    content: String,
    #[builder(into)]
    role: Role,
    #[builder(default = Zoned::now())]
    timestamp: Zoned,
    #[builder(default)]
    edits: Vec<MessageEdit>,
    /// The message "revision," 0 is the original (unedited) message, 1 is the
    /// first edit, 2 is the second edit, etc.
    #[builder(default)]
    chosen_revision: usize,
    #[builder(into)]
    original: Original,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct MessageEdit {
    #[builder(default = Zoned::now())]
    timestamp: Zoned,
    #[builder(into)]
    content: String,
    #[builder(into)]
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
        character: RecordId,
        name: String,
        response: CreateChatCompletionResponse,
    },
    Character {
        id: RecordId,
        name: String,
    },
    ExampleMessage {
        user_id: UserId,
    },
    System,
}

impl From<&Character> for Original {
    fn from(character: &Character) -> Self {
        Self::Character {
            id: character.id(),
            name: character.name().to_owned(),
        }
    }
}

impl From<UserId> for Original {
    fn from(user_id: UserId) -> Self {
        Self::ExampleMessage { user_id }
    }
}

impl Message {
    #[must_use]
    pub fn id(&self) -> RecordId {
        self.id.clone()
    }

    pub fn new_assistant(content: impl Into<String>, character: &Character) -> Self {
        Self::builder()
            .content(content)
            .role(Role::Assistant)
            .original(character)
            .build()
    }

    pub fn new_user(content: impl Into<String>, user_id: UserId) -> Self {
        Self::builder()
            .content(content)
            .role(Role::User)
            .original(user_id)
            .build()
    }

    pub fn new_system(content: impl Into<String>) -> Self {
        Self::builder()
            .content(content)
            .role(Role::System)
            .original(Original::System)
            .build()
    }

    pub fn edit(&mut self, content: impl Into<String>, author: impl Into<UserId>) {
        self.edits.push(
            MessageEdit::builder()
                .content(content)
                .author(author)
                .build(),
        );
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
            id: RecordId::from(("message", Ulid::new().to_string())),
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
        Self::builder()
            .role(&input)
            .content(&input.content)
            .original(input)
            .build()
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
            Original::OpenAi { name, .. } | Original::Character { name, .. } => {
                Self::Assistant(ChatCompletionRequestAssistantMessage {
                    content: Some(input.content.into()),
                    name: Some(name),
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

impl From<&DiscordMessage> for Role {
    fn from(input: &DiscordMessage) -> Self {
        if input.content.to_lowercase().starts_with("system:") {
            Self::System
        } else {
            Self::User
        }
    }
}

impl From<(Character, CreateChatCompletionResponse)> for Original {
    fn from(input: (Character, CreateChatCompletionResponse)) -> Self {
        let (character, response) = input;
        Self::OpenAi {
            name: character.name().into(),
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
