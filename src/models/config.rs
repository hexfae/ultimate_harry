//! Per-user preferences (display name and reaction emoji), keyed by Discord user ID.

use serde::{Deserialize, Serialize};
use serenity::all::ReactionType;

/// A user's preferences: their display name and reaction emoji, keyed by their Discord user ID.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct UserPrefs {
    /// The user's Discord ID.
    pub user_id: String,
    /// The user's set display name, if any.
    pub name: Option<String>,
    /// The user's set reaction emoji, if any.
    pub emoji: Option<ReactionType>,
}

impl UserPrefs {
    /// Creates an empty preferences record for the given user.
    #[must_use]
    pub const fn new(user_id: String) -> Self {
        Self {
            user_id,
            name: None,
            emoji: None,
        }
    }
}
