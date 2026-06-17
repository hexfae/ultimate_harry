//! Shared Components V2 builders used across chat and character flows.

use serenity::all::{ButtonStyle, CreateButton, ReactionType};
use serenity::small_fixed_array::FixedString;

/// Builds a secondary-style button with the given custom ID and unicode emoji.
pub fn emoji_button(custom_id: String, emoji: &'static str) -> CreateButton<'static> {
    CreateButton::new(custom_id)
        .emoji(ReactionType::Unicode(FixedString::from_static_trunc(emoji)))
        .style(ButtonStyle::Secondary)
}
