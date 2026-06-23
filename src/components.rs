//! Shared Components V2 builders used across chat and character flows.

use alloc::borrow::Cow;
use serenity::all::{
    ButtonStyle, CreateActionRow, CreateButton, CreateComponent, CreateInteractionResponse,
    CreateInteractionResponseMessage, ReactionType,
};
use serenity::small_fixed_array::FixedString;

use crate::events::interaction::InteractionKind;
use crate::phrases::{no, yes};

/// Builds a secondary-style button with the given custom ID and unicode emoji.
pub fn emoji_button(custom_id: String, emoji: &'static str) -> CreateButton<'static> {
    CreateButton::new(custom_id)
        .emoji(ReactionType::Unicode(FixedString::from_static_trunc(emoji)))
        .style(ButtonStyle::Secondary)
}

/// Builds a single-button action row labelled `label` and keyed on `custom_id`.
pub fn single_button_row<'a, I, L>(custom_id: I, label: L) -> Vec<CreateComponent<'a>>
where
    I: Into<Cow<'a, str>>,
    L: Into<Cow<'a, str>>,
{
    let button = vec![CreateButton::new(custom_id).label(label)].into();
    vec![CreateComponent::ActionRow(CreateActionRow::Buttons(button))]
}

/// Builds a confirmation update-message response with confirm/cancel buttons keyed on `id`.
#[must_use]
pub fn confirm_interaction_response<I, C>(id: I, content: C) -> CreateInteractionResponse<'static>
where
    I: Into<u64>,
    C: Into<String>,
{
    let buttons = confirm_buttons(id);
    CreateInteractionResponse::UpdateMessage(
        CreateInteractionResponseMessage::new()
            .content(content.into())
            .components(buttons),
    )
}

/// Builds confirmation buttons with "confirm" and "cancel" actions keyed on `into_id`.
fn confirm_buttons(into_id: impl Into<u64>) -> Vec<CreateComponent<'static>> {
    let id: u64 = into_id.into();
    let confirm_id = InteractionKind::Confirm.custom_id(id);
    let cancel_id = InteractionKind::Cancel.custom_id(id);
    vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        vec![
            CreateButton::new(confirm_id)
                .style(ButtonStyle::Secondary)
                .label(yes()),
            CreateButton::new(cancel_id)
                .style(ButtonStyle::Secondary)
                .label(no()),
        ]
        .into(),
    ))]
}
