//! Shared Components V2 builders used across chat and character flows.

use serenity::all::{ButtonStyle, CreateButton, ReactionType};
use serenity::small_fixed_array::FixedString;

/// Builds a secondary-style button with the given custom ID and unicode emoji.
pub fn emoji_button(custom_id: String, emoji: &'static str) -> CreateButton<'static> {
    CreateButton::new(custom_id)
        .emoji(ReactionType::Unicode(FixedString::from_static_trunc(emoji)))
        .style(ButtonStyle::Secondary)
}

/// Parses a stored emoji string into a `ReactionType`, returning `None` when it
/// is not actually an emoji. Serenity's `try_from` accepts any non-`<` string
/// as a unicode emoji without checking, so Discord later rejects garbage such as
/// stray text or unexpanded `:shortcodes:`; this validates the unicode case
/// against the `emojis` table and leaves custom `<:name:id>` emoji to serenity.
pub fn reaction_from(emoji: &str) -> Option<ReactionType> {
    if !emoji.starts_with('<') && emojis::get(emoji).is_none() {
        return None;
    }
    ReactionType::try_from(emoji.to_owned()).ok()
}

#[cfg(test)]
mod tests {
    use super::reaction_from;

    #[test]
    fn accepts_real_unicode_emoji_including_variation_selectors() {
        for emoji in ["🪨", "🎤", "😡", "⬆️", "▶️", "🥷", "🧌", "🤖", "🎭"] {
            assert!(
                reaction_from(emoji).is_some(),
                "expected {emoji} to be a valid emoji"
            );
        }
    }

    #[test]
    fn accepts_custom_wire_format_emoji() {
        assert!(reaction_from("<:warcraft:1324532445730570333>").is_some());
    }

    #[test]
    fn rejects_non_emoji_free_text() {
        for garbage in [
            "Ansikte",
            ":brain:",
            ":angry:",
            "🧌🧌",
            "😍SIMON😍 Sten, Simons Bästaste Vän",
            "",
        ] {
            assert!(
                reaction_from(garbage).is_none(),
                "expected {garbage:?} to be rejected"
            );
        }
    }
}
