//! Small keyed configuration and preference records: the bot-global singletons
//! (model settings, pin channel) and the per-user name/emoji records.

use native_db::{ToKey as _, native_db};
use native_model::{Model as _, native_model};
use serde::{Deserialize, Serialize};
use serenity::all::{ChannelId, ReactionType};

use crate::llm::ModelSettings;

/// The fixed primary key used for singleton records (model settings, pin channel).
pub const SINGLETON_KEY: &str = "global";

/// The Discord channel where pins should go, stored as a singleton.
#[derive(Debug, Default, Serialize, Deserialize)]
#[native_model(id = 4, version = 1, with = crate::codec::Json)]
#[native_db]
pub struct PinChannel {
    /// The fixed singleton primary key.
    #[primary_key]
    id: String,
    /// The Discord channel's id.
    channel_id: ChannelId,
}

impl PinChannel {
    /// Creates the pin-channel singleton record for the given channel.
    #[must_use]
    pub fn new(channel_id: ChannelId) -> Self {
        Self {
            id: SINGLETON_KEY.to_owned(),
            channel_id,
        }
    }

    /// Returns the stored channel.
    #[must_use]
    pub const fn channel_id(&self) -> ChannelId {
        self.channel_id
    }
}

/// The bot's AI model settings, stored as a singleton.
#[derive(Debug, Serialize, Deserialize)]
#[native_model(id = 5, version = 1, with = crate::codec::Json)]
#[native_db]
pub struct GlobalModelSettings {
    /// The fixed singleton primary key.
    #[primary_key]
    id: String,
    /// The model settings.
    settings: ModelSettings,
}

impl GlobalModelSettings {
    /// Creates the model-settings singleton record.
    #[must_use]
    pub fn new(settings: ModelSettings) -> Self {
        Self {
            id: SINGLETON_KEY.to_owned(),
            settings,
        }
    }

    /// Consumes the record, returning the stored model settings.
    #[must_use]
    pub fn into_settings(self) -> ModelSettings {
        self.settings
    }
}

/// A user's display name, keyed by their Discord user ID.
#[derive(Debug, Serialize, Deserialize)]
#[native_model(id = 6, version = 1, with = crate::codec::Json)]
#[native_db]
pub struct UserName {
    /// The user's discord ID, used as the primary key.
    #[primary_key]
    pub user_id: String,
    /// The user's set display name.
    pub name: String,
}

impl UserName {
    /// Creates a display-name record for the given user.
    #[must_use]
    pub const fn new(user_id: String, name: String) -> Self {
        Self { user_id, name }
    }
}

/// A user's emoji, keyed by their Discord user ID.
#[derive(Debug, Serialize, Deserialize)]
#[native_model(id = 7, version = 1, with = crate::codec::Json)]
#[native_db]
pub struct UserEmoji {
    /// The user's discord ID, used as the primary key.
    #[primary_key]
    pub user_id: String,
    /// The user's set emoji.
    pub emoji: ReactionType,
}

impl UserEmoji {
    /// Creates an emoji record for the given user.
    #[must_use]
    pub const fn new(user_id: String, emoji: ReactionType) -> Self {
        Self { user_id, emoji }
    }
}
