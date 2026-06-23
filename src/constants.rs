//! A few constants used throughout the crate.

use core::time::Duration;

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
/// The emoji used for restoring a deleted character.
pub const RESTORE: &str = "♻️";
/// The emoji used for navigating to an older version of a character.
pub const OLDER_VERSION: &str = "⏮";
/// The emoji used for navigating to a newer version of a character.
pub const NEWER_VERSION: &str = "⏭";
/// The emoji used for rolling a character back to the shown older version.
pub const ROLLBACK: &str = "⏪";

/// The max amount of characters an AI can respond with.
///
/// This is slightly below 4000 (Discord's limit) because their count includes all text of all
/// text displays on the embed/component, including e.g. the footer or the character's name.
pub const CHARACTER_LIMIT: usize = 3900;

/// The maximum number of characters returned by the listing/ranking queries.
///
/// Capped at Discord's hard limit of 25 options per select menu, so both the database listing
/// queries and `Character::rank_by_similarity` must agree on this value.
pub const MAX_RESULTS: usize = 25;

/// How long a transient confirmation or notice message lingers before it is deleted.
pub const TRANSIENT_LINGER: Duration = Duration::from_secs(5);

/// The accent colour of a user-facing error container (Discord's danger red), so
/// a failure reads as an error rather than as the character speaking.
pub const ERROR_COLOUR: u32 = 0x00ED_4245;

/// The heading shown atop a user-facing error, marking it as a failure rather
/// than a character's reply.
pub const ERROR_HEADING: &str = "## ⚠️ Något gick fel";
