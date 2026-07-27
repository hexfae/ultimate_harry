//! User-facing error presentation.
//!
//! A failure should look like a failure, not like the character speaking. These
//! builders render an error's user message into a distinct red Components V2
//! container (a danger-coloured accent bar and a warning heading) and adapt it
//! to the surface that failed: a message edit for an in-flight chat reply, or an
//! ephemeral interaction response/followup for a button press.

use poise::CreateReply;
use serenity::all::{
    CreateComponent, CreateContainer, CreateContainerComponent, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateInteractionResponseMessage, CreateMessage,
    CreateTextDisplay, EditMessage, MessageFlags,
};

use crate::constants::{ERROR_COLOUR, ERROR_HEADING};

/// Wraps `body` in the red error container, below the warning heading.
pub fn error_container(body: Vec<CreateContainerComponent<'_>>) -> CreateComponent<'_> {
    let mut components = vec![CreateContainerComponent::TextDisplay(
        CreateTextDisplay::new(ERROR_HEADING),
    )];
    components.extend(body);
    CreateComponent::Container(CreateContainer::new(components).accent_colour(ERROR_COLOUR))
}

/// Builds the red error container's components from a user-facing message.
fn error_components(message: String) -> Vec<CreateComponent<'static>> {
    vec![error_container(vec![
        CreateContainerComponent::TextDisplay(CreateTextDisplay::new(message)),
    ])]
}

/// Builds a message edit that replaces an in-flight reply with the error container.
pub fn error_message_edit(message: String) -> EditMessage<'static> {
    EditMessage::new()
        .flags(MessageFlags::IS_COMPONENTS_V2)
        .components(error_components(message))
}

/// Builds a channel message showing the error container, for a failure that
/// happens before there is an in-flight reply to edit.
pub fn error_message(message: String) -> CreateMessage<'static> {
    CreateMessage::new()
        .flags(MessageFlags::IS_COMPONENTS_V2)
        .components(error_components(message))
}

/// Builds an ephemeral interaction response showing the error container, for a
/// failure that happens before the interaction has been acknowledged.
pub fn error_response(message: String) -> CreateInteractionResponse<'static> {
    CreateInteractionResponse::Message(
        CreateInteractionResponseMessage::new()
            .flags(MessageFlags::IS_COMPONENTS_V2)
            .ephemeral(true)
            .components(error_components(message)),
    )
}

/// Builds an ephemeral interaction followup showing the error container, for a
/// failure that happens after the interaction has already been acknowledged.
pub fn error_followup(message: String) -> CreateInteractionResponseFollowup<'static> {
    CreateInteractionResponseFollowup::new()
        .flags(MessageFlags::IS_COMPONENTS_V2)
        .ephemeral(true)
        .components(error_components(message))
}

/// Builds an ephemeral poise reply showing the error container, for a slash
/// command that failed.
pub fn error_reply(message: String) -> CreateReply<'static> {
    CreateReply::default()
        .flags(MessageFlags::IS_COMPONENTS_V2)
        .ephemeral(true)
        .components(error_components(message))
}
