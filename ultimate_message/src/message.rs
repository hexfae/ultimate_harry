use std::time::Duration;

use async_openai::types::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
    ChatCompletionRequestMessageContentPartImage, ChatCompletionRequestSystemMessage,
    ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContentPart,
    CreateChatCompletionResponse, ImageUrl,
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
    /// The "parts" of the message. A message will ONLY have multiple parts if created from a user's Discord
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
    created: Zoned,
    /// If Some, the time it took to generate this message.
    elapsed: Option<Duration>,
    #[builder(default)]
    revisions: Vec<Revision>,
    /// The URLs of all attached images, if any.
    #[builder(default, with = |attachments: &[Attachment]| attachments.iter().map(|a| a.url.clone()).collect::<Vec<String>>()  )]
    images: Vec<String>,
    /// The message "revision," 0 is the original (unedited) message, 1 is the
    /// first edit, 2 is the second edit, etc.
    #[builder(default)]
    revision: usize,
    /// The original source of this message (Discord, AI, example message, etc.)..
    #[builder(into)]
    original: Original,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Parts(NonEmpty<Part>);

#[derive(Debug, Default, Clone, Serialize, Deserialize, Builder)]
pub struct Part {
    name: String,
    content: String,
    role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct Revision {
    #[builder(default = Zoned::now())]
    created: Zoned,
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
    pub const fn id(&self) -> &RecordId {
        &self.id
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
        self.revisions.push(
            Revision::builder()
                .parts((author.into(), content.into(), Role::Assistant))
                .editor(editor)
                .build(),
        );
        self.revision = self.revisions.len();
    }

    #[must_use]
    pub fn chosen_revision(&self) -> &Parts {
        match self.revision {
            0 => &self.parts,
            other => &self.revisions[other - 1].parts,
        }
    }

    pub fn get_version_part(&self, version: impl Into<usize> + Copy) -> &Parts {
        match version.into() {
            0 => &self.parts,
            other => &self.revisions[other - 1].parts,
        }
    }

    #[must_use]
    pub const fn revisions_len(&self) -> usize {
        self.revisions.len()
    }

    pub fn editor_of_version(&self, version: impl Into<usize> + Copy) -> Option<UserId> {
        match version.into() {
            0 => None,
            _ => Some(self.revisions[version.into() - 1].editor),
        }
    }

    #[must_use]
    /// The editor of the currently chosen revision, if any.
    pub fn current_editor(&self) -> Option<UserId> {
        self.editor_of_version(self.revision)
    }

    #[must_use]
    pub const fn revision(&self) -> usize {
        self.revision
    }

    pub const fn undo(&mut self) {
        self.revision = (self.revision + self.revisions_len()) % (self.revisions_len() + 1);
    }

    pub const fn redo(&mut self) {
        self.revision = (self.revision + 1) % (self.revisions_len() + 1);
    }

    #[must_use]
    pub const fn time_taken(&self) -> Option<Duration> {
        self.elapsed
    }
}

impl Parts {
    pub const fn head(&self) -> &Part {
        &self.0.head
    }

    pub fn tail(&self) -> Vec<&Part> {
        self.0.tail.iter().collect()
    }
}

impl Part {
    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }
}

impl TryFrom<(Character, CreateChatCompletionResponse, Duration)> for Message {
    type Error = Error;

    fn try_from(
        (character, response, time_taken): (Character, CreateChatCompletionResponse, Duration),
    ) -> Result<Self, Self::Error> {
        Ok(Self::builder()
            .parts((
                character.name().to_owned(),
                response.try_to_string()?,
                Role::Assistant,
            ))
            .original((character, response))
            .elapsed(time_taken)
            .build())
    }
}

impl From<(DiscordMessage, String)> for Message {
    fn from((message, name): (DiscordMessage, String)) -> Self {
        let parts = message
            .content
            .lines()
            .map(|line| {
                let (name, content) = if let Some((name, _)) = line.split_once(": ") {
                    (name.to_owned(), line.to_owned())
                } else {
                    (name.clone(), format!("{name}: {line}"))
                };
                let role = if message.author.bot || name.to_lowercase() == "ai" {
                    Role::Assistant
                } else if name.to_lowercase() == "system" {
                    Role::System
                } else {
                    Role::User
                };
                Part::builder()
                    .name(name)
                    .content(content)
                    .role(role)
                    .build()
            })
            .collect::<Vec<Part>>();
        Self::builder()
            .id(message.id)
            .parts(parts)
            .images(&message.attachments)
            .original(message)
            .build()
    }
}

impl From<Message> for NonEmpty<ChatCompletionRequestMessage> {
    fn from(input: Message) -> Self {
        match input.original {
            // TODO: refactor this whole thing holy shit
            Original::Discord { .. } | Original::ExampleMessage { .. } => {
                // i don't think using chosen_revision is really necessary here,
                // but still doing it for future-proofing or something
                let chosen = input.chosen_revision();
                let mut parts = Self::new(chosen.head().to_owned().into());
                parts.append(&mut chosen.tail().into_iter().cloned().map(Into::into).collect());
                // TODO: make this not always be user, sometimes assistant/system
                let mut content = vec![ChatCompletionRequestUserMessageContentPart::Text(
                    chosen.0.last().content.clone().into(),
                )];
                let mut images = input
                    .images
                    .iter()
                    .map(|image| {
                        ChatCompletionRequestUserMessageContentPart::ImageUrl(
                            ChatCompletionRequestMessageContentPartImage {
                                image_url: ImageUrl {
                                    url: image.to_owned(),
                                    detail: None,
                                },
                            },
                        )
                    })
                    .collect();
                content.append(&mut images);
                *parts.last_mut() =
                    ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                        content: content.into(),
                        name: Some(input.chosen_revision().head().name.clone()),
                    });
                parts
            }
            Original::OpenAi { ref name, .. } | Original::Character { ref name, .. } => Self::new(
                ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                    content: Some(input.chosen_revision().head().content.clone().into()),
                    name: Some(name.to_owned()),
                    ..Default::default()
                }),
            ),
            Original::System => Self::new(ChatCompletionRequestMessage::System(
                ChatCompletionRequestSystemMessage {
                    content: input.chosen_revision().head().content.clone().into(),
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
                name: Some(part.name),
            }),
            Role::Assistant => Self::Assistant(ChatCompletionRequestAssistantMessage {
                content: Some(part.content.into()),
                name: Some(part.name),
                ..Default::default()
            }),
            Role::System => Self::System(ChatCompletionRequestSystemMessage {
                content: part.content.into(),
                name: Some(part.name),
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
    fn from((name, content, role): (String, String, Role)) -> Self {
        Self(NonEmpty::new(
            Part::builder()
                .name(name)
                .content(content)
                .role(role)
                .build(),
        ))
    }
}
