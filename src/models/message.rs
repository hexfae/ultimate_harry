use std::time::Duration;

use bon::Builder;
use jiff::Zoned;
use nonempty::NonEmpty;
use rig::{
    OneOrMany,
    agent::Text,
    message::{AssistantContent, Message as RigMessage, UserContent},
};
use serde::{Deserialize, Serialize};
use serenity::all::{Attachment, Message as DiscordMessage, MessageId, UserId};
use surrealdb::RecordId;
use ulid::Ulid;

use crate::models::character::Character;

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
    /// ```text
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
    #[builder(default, with = |attachments: &[Attachment]| attachments.iter().map(|a| a.url.to_string()).collect::<Vec<String>>()  )]
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
    editor: Option<UserId>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub enum Role {
    #[default]
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Original {
    Discord {
        user_id: UserId,
        // message: Box<DiscordMessage>,
    },
    OpenAi {
        character: RecordId,
        name: String,
    },
    Rig {
        character: RecordId,
        name: String,
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
        editor: Option<impl Into<UserId>>,
    ) {
        self.revisions.push(
            Revision::builder()
                .parts((author.into(), content.into(), Role::Assistant))
                .maybe_editor(editor)
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
            _ => self.revisions[version.into() - 1].editor,
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

    pub fn to_rig_messages(&self) -> Vec<RigMessage> {
        let chosen = self.chosen_revision();

        chosen
            .0
            .iter()
            .map(|part| match part.role {
                Role::System => RigMessage::System {
                    content: part.content.clone(),
                },
                Role::Assistant => RigMessage::Assistant {
                    id: None,
                    content: OneOrMany::one(AssistantContent::Text(Text {
                        text: part.content.clone(),
                    })),
                },
                Role::User => {
                    let mut content = OneOrMany::one(UserContent::text(&part.content));

                    for img_url in &self.images {
                        content.push(UserContent::image_url(img_url, None, None));
                    }

                    RigMessage::User { content }
                }
            })
            .collect()
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

impl From<(Character, String, Duration)> for Message {
    fn from((character, response, time_taken): (Character, String, Duration)) -> Self {
        Self::builder()
            .parts((character.name().to_owned(), response, Role::Assistant))
            .original(Original::Rig {
                character: character.id().to_owned(),
                name: character.name().to_owned(),
            })
            .elapsed(time_taken)
            .build()
    }
}

impl From<(&DiscordMessage, String)> for Message {
    fn from((message, name): (&DiscordMessage, String)) -> Self {
        let parts = message
            .content
            .lines()
            .map(|line| {
                let (name, content) = if let Some((name, _)) = line.split_once(": ") {
                    (name.to_owned(), line.to_owned())
                } else {
                    (name.clone(), format!("{name}: {line}"))
                };
                let role = if message.author.bot() || name.to_lowercase() == "ai" {
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

impl From<&DiscordMessage> for Original {
    fn from(input: &DiscordMessage) -> Self {
        Self::Discord {
            user_id: input.author.id,
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
