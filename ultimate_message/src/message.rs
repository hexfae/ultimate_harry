use std::time::Duration;

use async_openai::types::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessage, ChatCompletionRequestUserMessage,
    CreateChatCompletionResponse,
};
use bon::Builder;
use jiff::Zoned;
use miette::Diagnostic;
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use serenity::all::{Attachment, Message as DiscordMessage, MessageId, UserId};
use snafu::{OptionExt, Snafu};
use surrealdb::RecordId;
use ulid::Ulid;
use ultimate_character::Character;

#[derive(Debug, Snafu, Diagnostic)]
pub enum Error {
    #[snafu(display("The OpenAI response has no choices"))]
    NoChoices,
    #[snafu(display("The OpenAI response has no content"))]
    NoContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct Message {
    /// If this message originates from Discord, the message's Discord ID, otherwise a ulid ID.
    #[builder(default = RecordId::from(("message", Ulid::new().to_string())), with = |id: MessageId| RecordId::from(("message", id.to_string())))]
    id: RecordId,
    /// The "parts" of the mssage. A message will ONLY have multiple parts if created from a user's Discord
    /// message. If so, the "parts" of it are every line.
    ///
    /// This allows the user to "send" multiple messages in one, like:
    ///
    /// ```
    /// hello
    /// system: message
    /// ai: what
    /// steve: no
    /// ```
    #[builder(into)]
    parts: Parts,
    /// The time this message was created.
    #[builder(default = Zoned::now())]
    timestamp: Zoned,
    /// If Some, the time it took to generate this message.
    time_taken: Option<Duration>,
    #[builder(default)]
    edits: Vec<MessageEdit>,
    /// The URLs of all attached images, if any.
    #[builder(default, with = |attachments: &[Attachment]| attachments.iter().map(|a| a.url.clone()).collect::<Vec<String>>()  )]
    images: Vec<String>,
    /// The message "revision," 0 is the original (unedited) message, 1 is the
    /// first edit, 2 is the second edit, etc.
    #[builder(default)]
    chosen_revision: usize,
    /// The original source of this message (Discord, AI, example message, etc.)..
    #[builder(into)]
    original: Original,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parts(NonEmpty<Part>);

#[derive(Debug, Default, Clone, Serialize, Deserialize, Builder)]
pub struct Part {
    author: String,
    content: String,
    role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct MessageEdit {
    #[builder(default = Zoned::now())]
    timestamp: Zoned,
    #[builder(into)]
    parts: Parts,
    #[builder(into)]
    editor: UserId,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub enum Role {
    #[default]
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum Original {
    Discord {
        user_id: UserId,
        // message: Box<DiscordMessage>,
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

impl Message {
    #[must_use]
    pub fn id(&self) -> RecordId {
        self.id.clone()
    }

    /// Unsplit on newlines.
    pub fn new_assistant(content: impl Into<String>, character: &Character) -> Self {
        Self::builder()
            .parts((character.name().to_owned(), content.into(), Role::Assistant))
            .original(character)
            .build()
    }

    /// Unsplit on newlines.
    pub fn new_user(
        author: impl Into<String>,
        content: impl Into<String>,
        user_id: UserId,
    ) -> Self {
        Self::builder()
            .parts((author.into(), content.into(), Role::User))
            .original(user_id)
            .build()
    }

    /// Unsplit on newlines.
    pub fn new_system(content: impl Into<String>) -> Self {
        Self::builder()
            .parts(("System".to_owned(), content.into(), Role::System))
            .original(Original::System)
            .build()
    }

    pub fn edit(
        &mut self,
        author: impl Into<String>,
        content: impl Into<String>,
        editor: impl Into<UserId>,
    ) {
        self.edits.push(
            MessageEdit::builder()
                .parts((author.into(), content.into(), Role::Assistant))
                .editor(editor)
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

    #[must_use]
    pub fn chosen_revision(&self) -> Parts {
        match self.chosen_revision {
            0 => self.parts.clone(),
            other => self.edits[other - 1].parts.clone(),
        }
    }

    pub fn get_version_part(&self, version: impl Into<usize> + Copy) -> Parts {
        match version.into() {
            0 => self.parts.clone(),
            other => self.edits[other - 1].parts.clone(),
        }
    }

    #[must_use]
    pub const fn revisions_len(&self) -> usize {
        self.edits.len()
    }

    pub fn editor_of_version(&self, version: impl Into<usize> + Copy) -> Option<UserId> {
        match version.into() {
            0 => None,
            _ => Some(self.edits[version.into() - 1].editor),
        }
    }

    #[must_use]
    /// The editor of the currently chosen revision, if any.
    pub fn current_editor(&self) -> Option<UserId> {
        self.editor_of_version(self.chosen_revision)
    }

    pub const fn set_revision(&mut self, revision: usize) {
        self.chosen_revision = revision;
    }
}

impl Parts {
    pub fn head(&self) -> Part {
        self.0.head.clone()
    }
}

impl Part {
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

impl TryFrom<(Character, CreateChatCompletionResponse)> for Message {
    type Error = Error;

    fn try_from(input: (Character, CreateChatCompletionResponse)) -> Result<Self, Self::Error> {
        let (character, response) = input;
        Ok(Self::builder()
            .parts((
                character.name().to_owned(),
                response.try_to_string()?,
                Role::Assistant,
            ))
            .original((character, response))
            .build())
    }
}

impl From<(DiscordMessage, String)> for Message {
    fn from((message, author): (DiscordMessage, String)) -> Self {
        let parts = message
            .content
            .lines()
            .map(|line| {
                let (author, content) = if let Some((author, _)) = line.split_once(": ") {
                    (author.to_owned(), line.to_owned())
                } else {
                    (author.clone(), format!("{author}: {line}"))
                };
                let role = if message.author.bot || author.to_lowercase() == "ai" {
                    Role::Assistant
                } else if author.to_lowercase() == "system" {
                    Role::System
                } else {
                    Role::User
                };
                Part::builder()
                    .author(author)
                    .content(content)
                    .role(role)
                    .build()
            })
            .collect::<Vec<Part>>();
        let out = Self::builder()
            .id(message.id)
            .parts(parts)
            .images(&message.attachments)
            .original(message)
            .build();
        dbg!(&out);
        out
    }
}

impl From<Message> for NonEmpty<ChatCompletionRequestMessage> {
    fn from(input: Message) -> Self {
        match input.original {
            Original::Discord { .. } | Original::ExampleMessage { .. } => {
                let mut parts = Self::new(input.parts.head().into());
                parts.append(&mut input.parts.0.tail.into_iter().map(Into::into).collect());
                parts
            }
            Original::OpenAi { name, .. } | Original::Character { name, .. } => Self::new(
                ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                    content: Some(input.parts.0.head.content.into()),
                    name: Some(name),
                    ..Default::default()
                }),
            ),
            Original::System => Self::new(ChatCompletionRequestMessage::System(
                ChatCompletionRequestSystemMessage {
                    content: input.parts.0.head.content.into(),
                    name: Some("System".to_owned()),
                },
            )),
        }
    }
}

impl From<DiscordMessage> for Original {
    fn from(input: DiscordMessage) -> Self {
        Self::Discord {
            user_id: input.author.id,
            // message: Box::new(input),
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
            character: character.id().to_owned(),
            response,
        }
    }
}

impl From<&Character> for Original {
    fn from(character: &Character) -> Self {
        Self::Character {
            id: character.id().to_owned(),
            name: character.name().to_owned(),
        }
    }
}

impl From<UserId> for Original {
    fn from(user_id: UserId) -> Self {
        Self::ExampleMessage { user_id }
    }
}

trait TryToString {
    fn try_to_string(&self) -> Result<String, Error>;
}

impl TryToString for CreateChatCompletionResponse {
    fn try_to_string(&self) -> Result<String, Error> {
        self.choices
            .first()
            .context(NoChoicesSnafu)?
            .message
            .content
            .clone()
            .context(NoContentSnafu)
    }
}

impl From<Part> for ChatCompletionRequestMessage {
    fn from(part: Part) -> Self {
        match part.role {
            Role::User => Self::User(ChatCompletionRequestUserMessage {
                content: part.content.into(),
                name: Some(part.author),
            }),
            Role::Assistant => Self::Assistant(ChatCompletionRequestAssistantMessage {
                content: Some(part.content.into()),
                name: Some(part.author),
                ..Default::default()
            }),
            Role::System => Self::System(ChatCompletionRequestSystemMessage {
                content: part.content.into(),
                name: Some(part.author),
            }),
        }
    }
}

impl From<Vec<Part>> for Parts {
    fn from(parts: Vec<Part>) -> Self {
        Self(parts.try_into().unwrap_or_default())
    }
}

impl From<(String, String, Role)> for Parts {
    fn from((author, content, role): (String, String, Role)) -> Self {
        Self(NonEmpty::new(
            Part::builder()
                .author(author)
                .content(content)
                .role(role)
                .build(),
        ))
    }
}
