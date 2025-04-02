use async_openai::types::{
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
    ChatCompletionRequestUserMessage, ChatCompletionRequestUserMessageContent,
    CreateChatCompletionResponse,
};
use jiff::Zoned;
use serenity::all::{Message as DiscordMessage, UserId};
use snafu::{OptionExt, Snafu};
use ultimate_character::Character;
use ultimate_config::CONFIG;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Expected an OpenAI response, found a discord message"))]
    NotOpenAiResponse,
    #[snafu(display("The OpenAI response has no choices"))]
    NoChoices,
    #[snafu(display("The OpenAI response has no content"))]
    NoContent,
}

pub struct Message {
    content: String,
    role: Role,
    timestamp: Zoned,
    original_message: OriginalMessage,
}

pub enum Role {
    User,
    Assistant,
    System,
}

enum OriginalMessage {
    Discord {
        user_id: UserId,
        // TODO: maybe store message ids and make a MESSAGES file/static instead?
        message: Box<DiscordMessage>,
    },
    OpenAi {
        // TODO: maybe store character ids (ulids) instead?
        character: Box<Character>,
        response: CreateChatCompletionResponse,
    },
}

impl TryFrom<(Character, CreateChatCompletionResponse)> for Message {
    type Error = Error;

    fn try_from(input: (Character, CreateChatCompletionResponse)) -> Result<Self, Self::Error> {
        let (character, response) = input;
        Ok(Self {
            content: response.clone().try_into_string()?,
            role: Role::Assistant,
            timestamp: Zoned::now(),
            original_message: OriginalMessage::from((character, response)),
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
            original_message: OriginalMessage::from(input),
        }
    }
}

impl From<Message> for ChatCompletionRequestMessage {
    fn from(input: Message) -> Self {
        match input.original_message {
            OriginalMessage::Discord { user_id, .. } => {
                Self::User(ChatCompletionRequestUserMessage {
                    content: ChatCompletionRequestUserMessageContent::Text(input.content),
                    name: Some(CONFIG.read().substitute_name(user_id)),
                })
            }
            OriginalMessage::OpenAi { character, .. } => {
                Self::Assistant(ChatCompletionRequestAssistantMessage {
                    content: Some(input.content.into()),
                    name: Some(character.name()),
                    ..Default::default()
                })
            }
        }
    }
}

impl From<DiscordMessage> for OriginalMessage {
    fn from(input: DiscordMessage) -> Self {
        Self::Discord {
            user_id: input.author.id,
            message: Box::new(input),
        }
    }
}

impl From<(Character, CreateChatCompletionResponse)> for OriginalMessage {
    fn from(input: (Character, CreateChatCompletionResponse)) -> Self {
        let (character, response) = input;
        Self::OpenAi {
            character: Box::new(character),
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
