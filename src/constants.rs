//! A few constants used throughout the crate.

/// The emoji used for cancelling operations on characters.
pub const CANCEL: &str = "❌";
/// The emoji used for navigating backward in chats/character pages.
pub const PREVIOUS: &str = "◀";
/// The emoji used for navigating forward in chats/character pages.
pub const NEXT: &str = "▶";
/// The emoji used for editing a character or a message in a chat.
pub const EDIT: &str = "✏️";
/// The emoji used for undoing a message edit.
pub const UNDO: &str = "↩️";
/// The emoji used for redoing a message edit.
pub const REDO: &str = "↪️";
/// The emoji used for pinning a message to the pin channel.
pub const PIN: &str = "📌";
/// The emoji used for deleting a character.
pub const DELETE: &str = "🗑";

/// The max amount of characters an AI can respond with.
///
/// This is slightly below 4000 (Discord's limit) because their count includes all text of all
/// text displays on the embed/component, including e.g. the footer or the character's name.
pub const CHARACTER_LIMIT: usize = 3900;
