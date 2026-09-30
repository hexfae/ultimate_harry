//! A convenience wrapper over a directory of JSON files used as the bot's storage.
//!
//! One file per record, each written atomically. The [`Database`] methods are
//! grouped by the record they touch, one child module per record type:
//! [`characters`], [`chats`], [`users`], and [`settings`]. This module keeps the
//! type itself, the on-disk layout, and the error type.

mod characters;
mod chats;
mod settings;
mod store;
#[cfg(test)]
mod tests;
mod users;

use core::fmt::{Debug, Formatter, Result as FmtResult};
use miette::{Diagnostic, SourceSpan};
use serde::Serialize;
use snafu::{ResultExt as _, Snafu};
use std::io;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::sync::Mutex;

pub use store::StoreError;

/// The root directory of the JSON-file database.
const DATABASE_DIR: &str = "harry_database";
/// The subdirectory holding one JSON file per character version.
const CHARACTERS_DIR: &str = "characters";
/// The subdirectory holding one JSON file per stored chat history.
const CHATS_DIR: &str = "chats";
/// The subdirectory holding the bot-global singleton config files.
const CONFIG_DIR: &str = "config";
/// The subdirectory holding one JSON file per user's preferences.
const USERS_DIR: &str = "users";
/// The config file storing the bot's AI model settings.
const MODEL_SETTINGS_FILE: &str = "model_settings.json";
/// The config file storing the bot's pin channel.
const PIN_CHANNEL_FILE: &str = "pin_channel.json";
/// The config file storing the bot's text-to-speech settings.
const TTS_SETTINGS_FILE: &str = "tts_settings.json";

/// A directory of JSON files used as the bot's storage.
pub struct Database {
    /// The root directory under which every record file lives.
    root: PathBuf,
    /// Serializes writes so two concurrent saves cannot interleave their temp-file renames.
    write_lock: Mutex<()>,
}

impl Debug for Database {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

impl Database {
    /// Opens (creating if needed) the JSON-file database in the default directory.
    pub async fn new() -> Result<Self, DatabaseError> {
        Self::open(PathBuf::from(DATABASE_DIR)).await
    }

    /// Opens (creating if needed) the JSON-file database rooted at `root`, ensuring every
    /// subdirectory exists.
    async fn open(root: PathBuf) -> Result<Self, DatabaseError> {
        for subdir in [CHARACTERS_DIR, CHATS_DIR, CONFIG_DIR, USERS_DIR] {
            fs::create_dir_all(root.join(subdir))
                .await
                .context(ConnectSnafu)?;
        }
        Ok(Self {
            root,
            write_lock: Mutex::new(()),
        })
    }

    /// Opens a fresh temporary database in a unique directory for tests.
    #[cfg(test)]
    pub(crate) async fn temporary() -> Result<Self, DatabaseError> {
        use core::sync::atomic::{AtomicU64, Ordering};
        use std::{env, process};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
        let pid = process::id();
        let root = env::temp_dir().join(format!("harry-test-{pid}-{unique}"));
        Self::open(root).await
    }

    /// The directory holding one JSON file per character version.
    fn characters_dir(&self) -> PathBuf {
        self.root.join(CHARACTERS_DIR)
    }

    /// The directory holding one JSON file per user's preferences.
    fn users_dir(&self) -> PathBuf {
        self.root.join(USERS_DIR)
    }

    /// The path of the file storing the character with the given ID.
    fn character_path(&self, id: &str) -> PathBuf {
        self.characters_dir().join(format!("{id}.json"))
    }

    /// The path of the file storing the chat history with the given ID.
    fn chat_path(&self, id: &str) -> PathBuf {
        self.root.join(CHATS_DIR).join(format!("{id}.json"))
    }

    /// The path of the file storing the given user's preferences.
    fn user_path(&self, id: &str) -> PathBuf {
        self.users_dir().join(format!("{id}.json"))
    }

    /// The path of the bot's model-settings config file.
    fn model_settings_path(&self) -> PathBuf {
        self.root.join(CONFIG_DIR).join(MODEL_SETTINGS_FILE)
    }

    /// The path of the bot's pin-channel config file.
    fn pin_channel_path(&self) -> PathBuf {
        self.root.join(CONFIG_DIR).join(PIN_CHANNEL_FILE)
    }

    /// The path of the bot's text-to-speech-settings config file.
    fn tts_settings_path(&self) -> PathBuf {
        self.root.join(CONFIG_DIR).join(TTS_SETTINGS_FILE)
    }

    /// Atomically serializes `value` to pretty JSON at `path`, serializing concurrent writes
    /// through the database's write lock. See [`store::write_json`].
    async fn write_json<T: Serialize + Sync>(
        &self,
        path: &Path,
        value: &T,
    ) -> Result<(), StoreError> {
        store::write_json(&self.write_lock, path, value).await
    }
}

/// All errors that can happen when interacting with the database.
#[derive(Debug, Snafu, Diagnostic)]
pub enum DatabaseError {
    /// Creating the database directory failed.
    #[snafu(display("Kunde inte öppna databasmappen"))]
    #[diagnostic(
        help("Kontrollera att databasmappen går att läsa och skriva till."),
        code(database::connect)
    )]
    Connect {
        /// The source of the error.
        source: io::Error,
    },
    /// Getting a record failed.
    #[snafu(display("Kunde inte hämta från databasen"))]
    #[diagnostic(help("Kontrollera att posten finns."), code(database::get))]
    Get {
        /// The source of the error.
        source: StoreError,
    },
    /// Inserting a record failed.
    #[snafu(display("Kunde inte infoga i databasen"))]
    #[diagnostic(help("Kontrollera att datat är korrekt."), code(database::insert))]
    Insert {
        /// The source of the error.
        source: StoreError,
    },
    /// Deleting a record failed.
    #[snafu(display("Kunde inte ta bort från databasen"))]
    #[diagnostic(help("Kontrollera att posten finns."), code(database::delete))]
    Delete {
        /// The source of the error.
        source: StoreError,
    },
    /// Updating a record failed.
    #[snafu(display("Kunde inte uppdatera databasen"))]
    #[diagnostic(help("Kontrollera att posten finns."), code(database::update))]
    Update {
        /// The source of the error.
        source: StoreError,
    },
    /// No character by the given ID was found.
    #[snafu(display("Ingen sådan gubbe hittades i databasen: {found}"))]
    #[diagnostic(
        help("Kontrollera namnet eller skapa en ny gubbe först."),
        code(database::no_character)
    )]
    NoCharacter {
        /// The ID that was tried.
        #[source_code]
        found: String,
        /// The part that was wrong (in practice, the entire ID is selected).
        #[label]
        span: SourceSpan,
    },
    /// Setting the bot's AI model settings failed.
    #[snafu(display("Kunde inte spara modellinställningar"))]
    #[diagnostic(
        help("Kontrollera att inställningarna är giltiga."),
        code(database::set_model_settings)
    )]
    SetModelSettings {
        /// The source of the error.
        source: StoreError,
    },
    /// Setting the bot's pin Discord channel failed.
    #[snafu(display("Kunde inte spara fästkanalen"))]
    #[diagnostic(
        help("Kontrollera att kanalen är giltig."),
        code(database::set_pins_channel)
    )]
    SetPinsChannel {
        /// The source of the error.
        source: StoreError,
    },
    /// Setting the bot's text-to-speech settings failed.
    #[snafu(display("Kunde inte spara uppläsningsinställningar"))]
    #[diagnostic(
        help("Kontrollera att inställningarna är giltiga."),
        code(database::set_tts_settings)
    )]
    SetTtsSettings {
        /// The source of the error.
        source: StoreError,
    },
}

impl DatabaseError {
    /// Whether retrying might succeed (a transient filesystem failure) rather than a
    /// permanent condition (an inaccessible directory, a record that is absent, or a
    /// record whose stored JSON cannot be parsed).
    #[must_use]
    pub const fn retryable(&self) -> bool {
        match self {
            Self::Get { source }
            | Self::Insert { source }
            | Self::Delete { source }
            | Self::Update { source }
            | Self::SetModelSettings { source }
            | Self::SetPinsChannel { source }
            | Self::SetTtsSettings { source } => source.retryable(),
            Self::Connect { .. } | Self::NoCharacter { .. } => false,
        }
    }
}
