//! Shared Components V2 builders used across chat and character flows.

use alloc::borrow::Cow;
use serenity::all::{
    ButtonStyle, CreateActionRow, CreateButton, CreateComponent, ReactionType,
};
use serenity::small_fixed_array::FixedString;

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
