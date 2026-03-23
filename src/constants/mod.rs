mod phrases;

pub use phrases::ASK_DELETE_PHRASES;
pub use phrases::ASK_EDIT_PHRASES;
pub use phrases::CANCELLED_PHRASES;
pub use phrases::CLICK_BELOW_PHRASES;
pub use phrases::CLICK_ME_PHRASES;
pub use phrases::CREATED_PHRASES;
pub use phrases::DELETED_PHRASES;
pub use phrases::EDITED_PHRASES;
pub use phrases::EDITING_PHRASES;
pub use phrases::NO_CHARACTER_PHRASES;
pub use phrases::NO_HISTORY_PHRASES;
pub use phrases::NO_PHRASES;
pub use phrases::TIMEOUT_PHRASES;
pub use phrases::YES_PHRASES;
pub use phrases::sample;
pub use phrases::sample_name;

pub const CANCEL: &str = "❌";
pub const PREVIOUS: &str = "◀";
pub const NEXT: &str = "▶";
pub const EDIT: &str = "✏️";
pub const UNDO: &str = "↩️";
pub const REDO: &str = "↪️";
pub const PIN: &str = "📌";
pub const DELETE: &str = "🗑";

/// The max amount of characters an AI can respond with.
///
/// This is slightly below 4000 (Discord's limit) because their count includes all text of all
/// text displays on the embed/component, including e.g. the footer or the character's name.
pub const CHARACTER_LIMIT: usize = 3900;
