//! User-facing error presentation.
//!
//! A failure should look like a failure, not like the character speaking. These
//! builders render an error's user message into a distinct red Components V2
//! container (a danger-coloured accent bar and a warning heading) and adapt it
//! to the surface that failed: a message edit for an in-flight chat reply, or an
//! ephemeral interaction response/followup for a button press.

use serenity::all::{
    CreateComponent, CreateContainer, CreateContainerComponent, CreateInteractionResponse,
    CreateInteractionResponseFollowup, CreateInteractionResponseMessage, CreateTextDisplay,
    EditMessage, MessageFlags,
};

use crate::constants::ERROR_COLOUR;

/// The heading shown atop every error container, marking it as a failure rather
/// than a character's reply.
const ERROR_HEADING: &str = "## ⚠️ Något gick fel";

/// Builds the red error container's components from a user-facing message.
fn error_components(message: String) -> Vec<CreateComponent<'static>> {
    let body = vec![
        CreateContainerComponent::TextDisplay(CreateTextDisplay::new(ERROR_HEADING)),
        CreateContainerComponent::TextDisplay(CreateTextDisplay::new(message)),
    ];
    vec![CreateComponent::Container(
        CreateContainer::new(body).accent_colour(ERROR_COLOUR),
    )]
}

/// Builds a message edit that replaces an in-flight reply with the error container.
pub fn error_message_edit(message: String) -> EditMessage<'static> {
    EditMessage::new()
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
