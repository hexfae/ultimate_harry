//! A convenience wrapper over a directory of JSON files used as the bot's storage.

mod store;

use core::fmt::{Debug, Formatter, Result as FmtResult};
use miette::{Diagnostic, SourceSpan};
use nanorand::Rng as _;
use serde::Serialize;
use serenity::all::{ChannelId, Color, MessageId, ReactionType, UserId};
use snafu::{IntoError, OptionExt as _, ResultExt as _, Snafu};
use std::io;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::sync::Mutex;
use tracing::warn;

pub use store::StoreError;
use store::{is_safe_id, read_json, read_or, scan_dir};

use crate::constants::MAX_RESULTS;
use crate::llm::{CharacterModelSettings, ModelSettings};
use crate::tts::{TtsSettings, VoiceEntry};
use crate::models::{
    character::{Character, CharacterOption},
    config::UserPrefs,
    history::{History, StoredHistory},
};

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

    /// The path of the file storing the character with the given ID.
    fn character_path(&self, id: &str) -> PathBuf {
        self.root.join(CHARACTERS_DIR).join(format!("{id}.json"))
    }

    /// The path of the file storing the chat history with the given ID.
    fn chat_path(&self, id: &str) -> PathBuf {
        self.root.join(CHATS_DIR).join(format!("{id}.json"))
    }

    /// The path of the file storing the given user's preferences.
    fn user_path(&self, id: &str) -> PathBuf {
        self.root.join(USERS_DIR).join(format!("{id}.json"))
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

    /// Returns every character currently stored, regardless of visibility.
    async fn all_characters(&self) -> Result<Vec<Character>, DatabaseError> {
        scan_dir(&self.root.join(CHARACTERS_DIR))
            .await
            .context(GetSnafu)
    }

    /// Returns a single character by its ID.
    pub async fn character(&self, id: &str) -> Result<Option<Character>, DatabaseError> {
        if !is_safe_id(id) {
            return Ok(None);
        }
        read_json(&self.character_path(id)).await.context(GetSnafu)
    }

    /// Returns up to 25 characters, sorted by the most similar ones to the given name.
    pub async fn characters_by_similarity<T: Into<String>>(
        &self,
        name: T,
    ) -> Result<Vec<Character>, DatabaseError> {
        Ok(Character::rank_by_similarity(
            self.all_characters().await?,
            &name.into(),
        ))
    }

    /// Returns up to 25 soft-deleted characters, sorted by the most similar ones
    /// to the given name. Used by the restore command, since deleted characters
    /// are hidden from every other listing.
    pub async fn deleted_characters_by_similarity<T: Into<String>>(
        &self,
        name: T,
    ) -> Result<Vec<Character>, DatabaseError> {
        Ok(Character::rank_deleted_by_similarity(
            self.all_characters().await?,
            &name.into(),
        ))
    }

    /// Returns every visible (not deleted, not superseded) character.
    async fn visible_characters(&self) -> Result<Vec<Character>, DatabaseError> {
        Ok(self
            .all_characters()
            .await?
            .into_iter()
            .filter(Character::is_visible)
            .collect())
    }

    /// Returns up to 25 visible characters, sorted by the most commonly used ones.
    pub async fn characters_by_usage(&self) -> Result<Vec<Character>, DatabaseError> {
        let mut characters = self.visible_characters().await?;
        characters.sort_by(|left, right| {
            right.conversations_had().cmp(&left.conversations_had())
        });
        characters.truncate(MAX_RESULTS);
        Ok(characters)
    }

    /// Returns up to 25 visible characters as lightweight hand-off menu options,
    /// sorted by the most commonly used ones.
    ///
    /// This is the shape the chat select menu needs; computing it once per reply
    /// (rather than re-scanning the whole character table on every streaming tick
    /// and button press) is the point of the projection.
    pub async fn character_menu_options(&self) -> Result<Vec<CharacterOption>, DatabaseError> {
        Ok(self
            .characters_by_usage()
            .await?
            .iter()
            .map(Character::to_menu_option)
            .collect())
    }

    /// Returns the configured voice palette for the reply's voice dropdown.
    ///
    /// Empty when no voices are configured, in which case the dropdown is hidden.
    pub async fn voice_options(&self) -> Vec<VoiceEntry> {
        self.tts_settings().await.voices
    }

    /// Returns up to 25 visible characters, sorted randomly.
    pub async fn random_characters(&self) -> Result<Vec<Character>, DatabaseError> {
        let mut characters = self.visible_characters().await?;
        let mut rng = nanorand::tls_rng();
        rng.shuffle(&mut characters);
        characters.truncate(MAX_RESULTS);
        Ok(characters)
    }

    /// Inserts a character.
    pub async fn insert_character(&self, character: Character) -> Result<(), DatabaseError> {
        self.write_json(&self.character_path(character.id()), &character)
            .await
            .context(InsertSnafu)
    }

    /// Loads the character `id`, applies `apply`, and writes it back, returning the mutated
    /// character. Returns `Ok(None)` if the character is missing. `context` selects the error
    /// variant for any failure, so each caller keeps its own error message.
    async fn mutate_character<C>(
        &self,
        id: &str,
        context: C,
        apply: impl FnOnce(&mut Character),
    ) -> Result<Option<Character>, DatabaseError>
    where
        C: IntoError<DatabaseError, Source = StoreError> + Copy,
    {
        if !is_safe_id(id) {
            return Ok(None);
        }
        let path = self.character_path(id);
        let Some(mut character) = read_json::<Character>(&path).await.context(context)? else {
            return Ok(None);
        };
        apply(&mut character);
        self.write_json(&path, &character).await.context(context)?;
        Ok(Some(character))
    }

    /// Soft-deletes a character by recording its deleter and the time of deletion.
    pub async fn delete_character<T: Into<UserId>>(
        &self,
        id: &str,
        deleted_by: T,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, DeleteSnafu, |character| {
            character.mark_deleted(deleted_by.into());
        })
        .await
    }

    /// Restores a soft-deleted character by clearing its deleted state. Returns
    /// the restored character, or `Ok(None)` if it is missing.
    pub async fn restore_character(
        &self,
        id: &str,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, Character::restore)
            .await
    }

    /// Returns every version of the character chain containing `id`, ordered
    /// oldest to newest. Walks back to the chain's root via `previous_version`,
    /// then forward via `next_version`. Returns an empty vector if `id` is
    /// missing, and stops walking at the first dangling link.
    pub async fn character_versions(
        &self,
        id: &str,
    ) -> Result<Vec<Character>, DatabaseError> {
        let Some(start) = self.character(id).await? else {
            return Ok(Vec::new());
        };
        let mut root = start;
        while let Some(previous_id) = root.previous_version().map(str::to_owned) {
            match self.character(&previous_id).await? {
                Some(previous) => root = previous,
                None => break,
            }
        }
        let mut chain = vec![root.clone()];
        let mut current = root;
        while let Some(next_id) = current.next_version().map(str::to_owned) {
            match self.character(&next_id).await? {
                Some(next) => {
                    chain.push(next.clone());
                    current = next;
                }
                None => break,
            }
        }
        Ok(chain)
    }

    /// Rolls a character back to the content of an older version `target_id`.
    ///
    /// Creates a new head atop `head_id` with the target version's content (see
    /// [`Character::rollback_to`]), keeping the head's accumulated stats, then
    /// supersedes the head with it. Returns the new head, or `Ok(None)` if either
    /// the head or the target version is missing.
    pub async fn rollback_character(
        &self,
        head_id: &str,
        target_id: &str,
        editor: UserId,
    ) -> Result<Option<Character>, DatabaseError> {
        let Some(target) = self.character(target_id).await? else {
            return Ok(None);
        };
        let Some(mut head) = self.character(head_id).await? else {
            return Ok(None);
        };
        head.rollback_to(editor, &target);
        let new_id = head.id().to_owned();
        self.supersede_character(new_id, head_id).await?;
        self.insert_character(head.clone()).await?;
        Ok(Some(head))
    }

    /// Sets the `next_version` field on the given old character ID to point to the given new character ID.
    pub async fn supersede_character(
        &self,
        new_id: String,
        old_id: &str,
    ) -> Result<Option<Character>, DatabaseError> {
        let superseded = self
            .mutate_character(old_id, UpdateSnafu, |character| {
                character.set_next_version(new_id);
            })
            .await?
            .with_context(|| NoCharacterSnafu {
                found: old_id.to_owned(),
                span: 0..old_id.len(),
            })?;
        Ok(Some(superseded))
    }

    /// Returns a chat history by its ID, embedding its messages.
    pub async fn history<T: Into<MessageId>>(
        &self,
        id: T,
    ) -> Result<Option<History>, DatabaseError> {
        let path = self.chat_path(&id.into().to_string());
        let Some(stored) = read_json::<StoredHistory>(&path).await.context(GetSnafu)? else {
            return Ok(None);
        };
        let history_id = stored.id.clone();
        let Some(history) = History::hydrate(stored) else {
            warn!("history {history_id} has no resolvable choices");
            return Ok(None);
        };
        Ok(Some(history))
    }

    /// Updates or inserts a chat history, storing the full messages inline in a single file.
    ///
    /// Each handler loads its own [`History`], mutates it, and saves it here separately, so two
    /// near-simultaneous button presses on the same reply race and the last write wins (a lost
    /// swipe/edit). This is accepted: there is no per-conversation lock or version guard, because the
    /// bot serves a handful of users and the worst case is a single dropped mutation, never
    /// corruption (every write is a whole, valid record renamed into place atomically).
    pub async fn upsert_history(&self, history: History) -> Result<(), DatabaseError> {
        let stored = history.into_stored();
        self.write_json(&self.chat_path(&stored.id), &stored)
            .await
            .context(InsertSnafu)
    }

    /// Loads the given user's preferences (or a fresh default), applies `apply`, and writes
    /// them back. Mirrors [`Self::mutate_character`] for the single-file user records.
    async fn mutate_user_prefs<T: Into<UserId>>(
        &self,
        user_id: T,
        apply: impl FnOnce(&mut UserPrefs),
    ) -> Result<(), DatabaseError> {
        let id = user_id.into().to_string();
        let path = self.user_path(&id);
        let mut prefs = read_json::<UserPrefs>(&path)
            .await
            .context(GetSnafu)?
            .unwrap_or_else(|| UserPrefs::new(id));
        apply(&mut prefs);
        self.write_json(&path, &prefs).await.context(InsertSnafu)
    }

    /// Updates or inserts a user's emoji, preserving their stored display name.
    pub async fn upsert_user_emoji<T: Into<UserId>>(
        &self,
        user_id: T,
        emoji: ReactionType,
    ) -> Result<(), DatabaseError> {
        self.mutate_user_prefs(user_id, |prefs| prefs.emoji = Some(emoji))
            .await
    }

    /// Returns every user's set emoji, paired with their Discord user ID.
    pub async fn user_emoji(&self) -> Result<Vec<(String, ReactionType)>, DatabaseError> {
        let prefs: Vec<UserPrefs> = scan_dir(&self.root.join(USERS_DIR)).await.context(GetSnafu)?;
        Ok(prefs
            .into_iter()
            .filter_map(|pref| pref.emoji.map(|emoji| (pref.user_id, emoji)))
            .collect())
    }

    /// Updates or inserts the bot's AI model settings.
    pub async fn upsert_model_settings(
        &self,
        model_settings: ModelSettings,
    ) -> Result<(), DatabaseError> {
        self.write_json(&self.model_settings_path(), &model_settings)
            .await
            .context(SetModelSettingsSnafu)
    }

    /// Sets a character's AI model settings override.
    pub async fn set_character_model_settings(
        &self,
        id: &str,
        model_settings: CharacterModelSettings,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.set_model_settings(model_settings);
        })
        .await
    }

    /// Links (or, with `None`, clears) a character's `ElevenLabs` voice. Returns the
    /// updated character, or `Ok(None)` if the character is missing.
    pub async fn set_character_voice(
        &self,
        id: &str,
        voice: Option<String>,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.set_voice(voice);
        })
        .await
    }

    /// Sets a character's embed color. Returns the updated character, or `Ok(None)`
    /// if the character is missing.
    pub async fn set_character_color(
        &self,
        id: &str,
        color: Color,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.set_color(color);
        })
        .await
    }

    /// Appends an example-message pair (an optional user line and the character's
    /// response) to a character. Returns the updated character, or `Ok(None)` if the
    /// character is missing.
    pub async fn add_character_example(
        &self,
        id: &str,
        user: Option<String>,
        response: String,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.add_example_message(user, response);
        })
        .await
    }

    /// Removes the example-message pair at `index` (zero-based) from a character.
    /// Returns the updated character, or `Ok(None)` if the character is missing; an
    /// out-of-range index leaves the examples untouched.
    pub async fn remove_character_example(
        &self,
        id: &str,
        index: usize,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.remove_example_message(index);
        })
        .await
    }

    /// Records a character spawn (a new conversation) for the given user.
    ///
    /// The stats land on the character's latest version (walking the version
    /// chain past any edits), so they carry across edits even when the
    /// conversation is pinned to an older version. Best-effort: warns and
    /// returns `Ok` if the character is missing.
    pub async fn record_character_spawn(
        &self,
        id: &str,
        user: UserId,
    ) -> Result<(), DatabaseError> {
        self.record_on_latest_version(id, "a spawn", |character| character.record_spawn(user))
            .await
    }

    /// Records the words and tokens a character generated in a single reply.
    ///
    /// The stats land on the character's latest version (walking the version
    /// chain past any edits), so they carry across edits even when the
    /// conversation is pinned to an older version. Best-effort: warns and
    /// returns `Ok` if the character is missing.
    pub async fn record_character_generation(
        &self,
        id: &str,
        words: u32,
        tokens: u32,
    ) -> Result<(), DatabaseError> {
        self.record_on_latest_version(id, "generation", |character| {
            character.record_generation(words, tokens);
        })
        .await
    }

    /// Walks `id` to its latest version and applies `apply` to that character,
    /// writing the result back. `label` names the stat for the missing-character
    /// warning. Best-effort: warns and returns `Ok` if the character is missing.
    async fn record_on_latest_version(
        &self,
        id: &str,
        label: &str,
        apply: impl FnOnce(&mut Character),
    ) -> Result<(), DatabaseError> {
        let Some(mut character) = self.character(id).await? else {
            warn!("tried to record {label} for a missing character: {id}");
            return Ok(());
        };
        while let Some(next_id) = character.next_version().map(str::to_owned) {
            let Some(next) = self.character(&next_id).await? else {
                break;
            };
            character = next;
        }
        apply(&mut character);
        self.write_json(&self.character_path(character.id()), &character)
            .await
            .context(UpdateSnafu)
    }

    /// Returns the bot's AI model settings.
    pub async fn model_settings(&self) -> ModelSettings {
        read_or(
            &self.model_settings_path(),
            "model settings",
            ModelSettings::default,
        )
        .await
    }

    /// Returns the global model settings with the character's per-character
    /// overrides (model and temperature) applied on top, if it has any.
    pub async fn resolved_model_settings(&self, character: &Character) -> ModelSettings {
        let mut settings = self.model_settings().await;
        if let Some(overrides) = character.model_settings() {
            if let Some(model) = overrides.model.clone() {
                settings.model = model;
            }
            if let Some(temperature) = overrides.temperature {
                settings.temperature = temperature;
            }
        }
        settings
    }

    /// Returns the bot's text-to-speech settings.
    pub async fn tts_settings(&self) -> TtsSettings {
        read_or(
            &self.tts_settings_path(),
            "tts settings",
            TtsSettings::default,
        )
        .await
    }

    /// Updates or inserts the bot's text-to-speech settings.
    pub async fn upsert_tts_settings(
        &self,
        tts_settings: TtsSettings,
    ) -> Result<(), DatabaseError> {
        self.write_json(&self.tts_settings_path(), &tts_settings)
            .await
            .context(SetTtsSettingsSnafu)
    }

    /// Returns the bot's pin channel.
    pub async fn pins_channel(&self) -> ChannelId {
        read_or(&self.pin_channel_path(), "pin channel", ChannelId::default).await
    }

    /// Updates or inserts the bot's pin channel.
    pub async fn upsert_pin_channel(
        &self,
        channel_id: ChannelId,
    ) -> Result<ChannelId, DatabaseError> {
        self.write_json(&self.pin_channel_path(), &channel_id)
            .await
            .context(SetPinsChannelSnafu)?;
        Ok(channel_id)
    }

    /// Returns a user's display name by their Discord user ID, defaulting to "User".
    pub async fn substitute_name<T: Into<UserId>>(&self, user_id: T) -> String {
        let path = self.user_path(&user_id.into().to_string());
        read_or(&path, "substitute name", UserPrefs::default)
            .await
            .name
            .unwrap_or_else(|| "User".to_owned())
    }

    /// Updates or inserts a user's display name, preserving their stored emoji.
    pub async fn upsert_user_name<T: Into<UserId>>(
        &self,
        user_id: T,
        name: String,
    ) -> Result<(), DatabaseError> {
        self.mutate_user_prefs(user_id, |prefs| prefs.name = Some(name))
            .await
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
    #[diagnostic(
        help("Kontrollera att posten finns."),
        code(database::get)
    )]
    Get {
        /// The source of the error.
        source: StoreError,
    },
    /// Inserting a record failed.
    #[snafu(display("Kunde inte infoga i databasen"))]
    #[diagnostic(
        help("Kontrollera att datat är korrekt."),
        code(database::insert)
    )]
    Insert {
        /// The source of the error.
        source: StoreError,
    },
    /// Deleting a record failed.
    #[snafu(display("Kunde inte ta bort från databasen"))]
    #[diagnostic(
        help("Kontrollera att posten finns."),
        code(database::delete)
    )]
    Delete {
        /// The source of the error.
        source: StoreError,
    },
    /// Updating a record failed.
    #[snafu(display("Kunde inte uppdatera databasen"))]
    #[diagnostic(
        help("Kontrollera att posten finns."),
        code(database::update)
    )]
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
    /// Whether retrying might succeed (a transient storage failure) rather than a
    /// permanent condition (an inaccessible directory or a record that is absent).
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Get { .. }
                | Self::Insert { .. }
                | Self::Delete { .. }
                | Self::Update { .. }
                | Self::SetModelSettings { .. }
                | Self::SetPinsChannel { .. }
                | Self::SetTtsSettings { .. }
        )
    }
}

/// Characterization tests pinning the load-mutate-write methods.
/// They run against a fresh temporary database directory.
#[cfg(test)]
mod tests {
    use super::{Database, is_safe_id};
    use crate::llm::CharacterModelSettings;
    use crate::models::character::Character;
    use serenity::all::{Color, UserId};

    /// A client-supplied character ID with path-traversal components is rejected,
    /// so a crafted select value cannot read a file outside the characters dir.
    #[test]
    fn is_safe_id_rejects_path_traversal() {
        assert!(is_safe_id("01J0ABCDEF"), "a plain ULID is a safe id");
        assert!(is_safe_id("123456789"), "a numeric snowflake is a safe id");
        assert!(!is_safe_id(""), "an empty id is rejected");
        assert!(!is_safe_id("../config/tts_settings"), "a parent traversal is rejected");
        assert!(!is_safe_id("sub/dir"), "a separator is rejected");
    }

    /// A path-traversal character ID must not reach a record on a *write* path,
    /// the way it cannot on the [`Database::character`] read path. An `id` like
    /// `../characters/<real>` resolves back to a stored record, so without the
    /// guard in `mutate_character` a crafted select value would mutate it; the
    /// guard now short-circuits every mutation to `None`, leaving it untouched.
    #[tokio::test]
    async fn mutations_reject_path_traversal_ids() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        insert(&db, character("sentinel", "Harry")).await;

        let traversal = "../characters/sentinel";

        let deleted = db.delete_character(traversal, UserId::new(2)).await;
        assert!(
            matches!(deleted, Ok(None)),
            "a traversal id is rejected on the delete path, returning None instead of mutating"
        );

        let recolored = db
            .set_character_color(traversal, Color::new(0x00ff_0000))
            .await;
        assert!(
            matches!(recolored, Ok(None)),
            "a traversal id is rejected on the color write path too"
        );

        let stored = db.character("sentinel").await.ok().flatten();
        assert!(
            stored.is_some_and(|record| record.is_visible() && record.color().is_none()),
            "the real character is left untouched by the traversal attempts"
        );
    }

    /// Builds a minimal visible character with the given ID and name.
    fn character(id: &str, name: &str) -> Character {
        Character::builder()
            .id(id.to_owned())
            .name(name)
            .greeting("hello")
            .creator(UserId::new(1))
            .build()
    }

    /// Inserts a character, asserting the write succeeds.
    async fn insert(db: &Database, character: Character) {
        assert!(
            db.insert_character(character).await.is_ok(),
            "inserting a character should succeed"
        );
    }

    /// `delete_character` soft-deletes the character and returns it; the stored
    /// record becomes invisible.
    #[tokio::test]
    async fn delete_character_soft_deletes_and_returns_the_character() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        insert(&db, character("char-id", "Harry")).await;

        let deleted = db.delete_character("char-id", UserId::new(2)).await;
        assert!(
            deleted
                .as_ref()
                .is_ok_and(|found| found.as_ref().is_some_and(|record| !record.is_visible())),
            "deleting returns the now-invisible character"
        );

        let stored = db.character("char-id").await.ok().flatten();
        assert!(
            stored.is_some_and(|record| !record.is_visible()),
            "the stored character is left invisible"
        );
    }

    /// Deleting a missing character returns `None` rather than erroring.
    #[tokio::test]
    async fn delete_character_returns_none_when_missing() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        let deleted = db.delete_character("missing", UserId::new(2)).await;
        assert!(
            matches!(deleted, Ok(None)),
            "deleting a missing character is a no-op returning None"
        );
    }

    /// `supersede_character` links the old record to the new version's ID.
    #[tokio::test]
    async fn supersede_character_sets_the_next_version() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        insert(&db, character("old-id", "Harry")).await;

        let superseded = db
            .supersede_character("new-id".to_owned(), "old-id")
            .await;
        assert!(
            superseded
                .as_ref()
                .is_ok_and(|found| found.as_ref().is_some_and(|record| record.next_version() == Some("new-id"))),
            "superseding records the new version's ID on the old character"
        );
    }

    /// Superseding a missing character is an error (unlike delete/model-settings,
    /// which return None).
    #[tokio::test]
    async fn supersede_character_errors_when_missing() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        let superseded = db
            .supersede_character("new-id".to_owned(), "missing")
            .await;
        assert!(
            superseded.is_err(),
            "superseding a missing character errors"
        );
    }

    /// `set_character_model_settings` records a per-character override and returns
    /// the updated character; a missing character returns `None`.
    #[tokio::test]
    async fn set_character_model_settings_records_an_override() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        insert(&db, character("char-id", "Harry")).await;

        let updated = db
            .set_character_model_settings(
                "char-id",
                CharacterModelSettings {
                    model: Some("char-model".to_owned()),
                    temperature: Some(0.5),
                },
            )
            .await;
        assert!(
            updated.as_ref().is_ok_and(|found| found.as_ref().is_some_and(|record| record
                .model_settings()
                .and_then(|settings| settings.model.as_deref())
                == Some("char-model"))),
            "setting model settings records the override's contents and returns the character"
        );

        let missing = db
            .set_character_model_settings("missing", CharacterModelSettings::default())
            .await;
        assert!(
            matches!(missing, Ok(None)),
            "setting model settings on a missing character returns None"
        );
    }

    /// `set_character_voice` links a voice to a character and returns it; clearing
    /// it removes the link, and a missing character returns `None`.
    #[tokio::test]
    async fn set_character_voice_links_and_clears_a_voice() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        insert(&db, character("char-id", "Harry")).await;

        let linked = db
            .set_character_voice("char-id", Some("voice-abc".to_owned()))
            .await;
        assert!(
            linked
                .as_ref()
                .is_ok_and(|found| found.as_ref().is_some_and(|record| record.voice()
                    == Some("voice-abc"))),
            "linking a voice records it and returns the character"
        );

        let cleared = db.set_character_voice("char-id", None).await;
        assert!(
            cleared
                .as_ref()
                .is_ok_and(|found| found.as_ref().is_some_and(|record| record.voice().is_none())),
            "clearing the voice removes the link"
        );

        let missing = db
            .set_character_voice("missing", Some("voice".to_owned()))
            .await;
        assert!(
            matches!(missing, Ok(None)),
            "setting a voice on a missing character returns None"
        );
    }

    /// `set_character_color` records a color on a character and returns it; a missing
    /// character returns `None`.
    #[tokio::test]
    async fn set_character_color_records_a_color() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        insert(&db, character("char-id", "Harry")).await;

        let set = db
            .set_character_color("char-id", Color::new(0x00ff_0000))
            .await;
        assert!(
            set.as_ref().is_ok_and(|found| found
                .as_ref()
                .is_some_and(|record| record.color() == Some(Color::new(0x00ff_0000)))),
            "setting a color records it and returns the character"
        );

        let missing = db
            .set_character_color("missing", Color::new(0x00ff_0000))
            .await;
        assert!(
            matches!(missing, Ok(None)),
            "setting a color on a missing character returns None"
        );
    }

    /// TTS settings round-trip through the config file, defaulting before any are saved.
    #[tokio::test]
    async fn tts_settings_round_trip_through_the_config_file() {
        use crate::tts::{TtsSettings, VoiceEntry};
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };

        assert!(
            db.tts_settings().await.api_key.is_empty(),
            "the default settings have no API key before any are saved"
        );

        let saved = db
            .upsert_tts_settings(TtsSettings {
                api_key: "secret".to_owned(),
                default_voice: Some("voice-1".to_owned()),
                model: "eleven_multilingual_v2".to_owned(),
                tag_model: Some("vendor/tagger".to_owned()),
                voices: vec![VoiceEntry {
                    name: "Anna".to_owned(),
                    voice_id: "voice-anna".to_owned(),
                    emoji: "🎭".to_owned(),
                    description: "lugn".to_owned(),
                    model: None,
                }],
            })
            .await;
        assert!(saved.is_ok(), "saving the settings should succeed");

        let loaded = db.tts_settings().await;
        assert_eq!(loaded.api_key, "secret", "the saved API key is read back");
        assert_eq!(
            loaded.default_voice.as_deref(),
            Some("voice-1"),
            "the saved default voice is read back"
        );
        assert_eq!(
            loaded.model, "eleven_multilingual_v2",
            "the saved synthesis model is read back"
        );
        assert_eq!(
            loaded.tag_model.as_deref(),
            Some("vendor/tagger"),
            "the saved audio-tag model is read back"
        );
        assert_eq!(
            loaded.voices.iter().map(|voice| voice.voice_id.as_str()).collect::<Vec<_>>(),
            vec!["voice-anna"],
            "the saved voice palette is read back"
        );
    }

    /// `record_character_generation` applies stats to the latest version, walking
    /// past an edit in the version chain.
    #[tokio::test]
    async fn record_character_generation_lands_on_the_latest_version() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        let mut old = character("old-id", "Harry");
        old.set_next_version("new-id".to_owned());
        insert(&db, old).await;
        insert(&db, character("new-id", "Harry")).await;

        assert!(
            db.record_character_generation("old-id", 3, 9).await.is_ok(),
            "recording generation succeeds"
        );

        let latest = db.character("new-id").await.ok().flatten();
        assert!(
            latest.is_some_and(|record| record.words_generated() == 3
                && record.tokens_generated() == 9),
            "generation stats land on the latest version"
        );
        let original = db.character("old-id").await.ok().flatten();
        assert!(
            original.is_some_and(|record| record.words_generated() == 0
                && record.tokens_generated() == 0),
            "the original version receives no stats"
        );
    }

    /// `restore_character` clears a soft-deleted character's deleted state and
    /// returns it visible again; a missing character returns `None`.
    #[tokio::test]
    async fn restore_character_restores_a_deleted_character() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        insert(&db, character("char-id", "Harry")).await;
        let deleted = db.delete_character("char-id", UserId::new(2)).await;
        assert!(deleted.is_ok(), "deleting the character should succeed");

        let restored = db.restore_character("char-id").await;
        assert!(
            restored
                .as_ref()
                .is_ok_and(|found| found.as_ref().is_some_and(Character::is_visible)),
            "restoring returns the now-visible character"
        );

        let stored = db.character("char-id").await.ok().flatten();
        assert!(
            stored.is_some_and(|record| record.is_visible()),
            "the stored character is visible again"
        );

        let missing = db.restore_character("missing").await;
        assert!(
            matches!(missing, Ok(None)),
            "restoring a missing character returns None"
        );
    }

    /// `deleted_characters_by_similarity` ranks only the soft-deleted characters,
    /// never the visible ones.
    #[tokio::test]
    async fn deleted_characters_by_similarity_returns_only_deleted() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        insert(&db, character("visible", "Harry")).await;
        insert(&db, character("ghost", "Harry")).await;
        let deleted = db.delete_character("ghost", UserId::new(2)).await;
        assert!(deleted.is_ok(), "deleting the character should succeed");

        let found = db
            .deleted_characters_by_similarity("Harry")
            .await
            .unwrap_or_default();
        let ids = found
            .iter()
            .map(|record| record.id().to_owned())
            .collect::<Vec<String>>();
        assert_eq!(
            ids,
            vec!["ghost".to_owned()],
            "only the deleted character is returned, never the visible one"
        );
    }

    /// `character_versions` walks the version chain to its root and back, returning
    /// every version oldest to newest regardless of which version it starts from.
    #[tokio::test]
    async fn character_versions_returns_the_chain_oldest_to_newest() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        let mut old = character("v0", "Harry");
        old.set_next_version("v1".to_owned());
        insert(&db, old).await;
        let new = Character::builder()
            .id("v1".to_owned())
            .name("Harry")
            .greeting("hello")
            .creator(UserId::new(1))
            .version(1_u32)
            .previous_version("v0".to_owned())
            .build();
        insert(&db, new).await;

        let from_new = db.character_versions("v1").await.unwrap_or_default();
        let ids_from_new = from_new
            .iter()
            .map(|record| record.id().to_owned())
            .collect::<Vec<String>>();
        assert_eq!(
            ids_from_new,
            vec!["v0".to_owned(), "v1".to_owned()],
            "the chain is returned oldest to newest"
        );

        let from_old = db.character_versions("v0").await.unwrap_or_default();
        let ids_from_old = from_old
            .iter()
            .map(|record| record.id().to_owned())
            .collect::<Vec<String>>();
        assert_eq!(
            ids_from_old,
            vec!["v0".to_owned(), "v1".to_owned()],
            "walking from any version returns the full chain"
        );
    }

    /// `rollback_character` creates a new head with the old version's content,
    /// keeps the previous head's accumulated stats, and supersedes that head.
    #[tokio::test]
    async fn rollback_character_supersedes_the_head_with_an_old_version() {
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        let mut old = Character::builder()
            .id("v0".to_owned())
            .name("Old")
            .greeting("old greeting")
            .creator(UserId::new(1))
            .personality("old personality".to_owned())
            .build();
        old.set_next_version("v1".to_owned());
        insert(&db, old).await;
        let head = Character::builder()
            .id("v1".to_owned())
            .name("New")
            .greeting("new greeting")
            .creator(UserId::new(1))
            .version(1_u32)
            .previous_version("v0".to_owned())
            .personality("new personality".to_owned())
            .conversations_had(7_u32)
            .build();
        insert(&db, head).await;

        let rolled = db.rollback_character("v1", "v0", UserId::new(3)).await;
        assert!(
            rolled.as_ref().is_ok_and(|found| found
                .as_ref()
                .is_some_and(|record| record.name() == "Old"
                    && record.is_visible()
                    && record.conversations_had() == 7)),
            "rolling back returns a visible new head with the old content and kept stats"
        );

        let head_now = db.character("v1").await.ok().flatten();
        assert!(
            head_now.is_some_and(|record| record.next_version().is_some()),
            "the previous head is superseded by the rolled-back version"
        );

        let missing = db.rollback_character("missing", "v0", UserId::new(3)).await;
        assert!(
            matches!(missing, Ok(None)),
            "rolling back a missing head returns None"
        );
    }

    /// A history round-trips through `upsert_history`/`history`, preserving its
    /// choices and context messages (content, not just IDs).
    #[tokio::test]
    async fn history_round_trips_through_the_chat_file() {
        use crate::models::history::History;
        use crate::models::message::Message;
        use nonempty::NonEmpty;
        use serenity::all::MessageId;

        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };

        let mut choices = NonEmpty::new(Message::new_system("first choice"));
        choices.push(Message::new_system("second choice"));
        let history = History::builder()
            .id(MessageId::new(42))
            .character("character-id")
            .choices(choices)
            .current(1_usize)
            .previous(vec![Message::new_user("Alice", "hello")])
            .build();
        assert!(
            db.upsert_history(history).await.is_ok(),
            "storing a history should succeed"
        );

        let loaded = db.history(MessageId::new(42)).await.ok().flatten();
        assert!(loaded.is_some(), "the stored history reads back");
        let Some(reloaded) = loaded else { return };
        assert_eq!(
            reloaded.character(),
            "character-id",
            "the character link survives the round-trip"
        );
        assert_eq!(
            reloaded.current_choice(),
            1,
            "the chosen index survives the round-trip"
        );
        assert_eq!(
            reloaded.chosen_message().chosen_revision().head().content(),
            "second choice",
            "the chosen choice's content survives the round-trip"
        );
        assert_eq!(
            reloaded
                .previous_messages()
                .first()
                .map(|message| message.chosen_revision().head().content()),
            Some("hello"),
            "the context message's content survives the round-trip"
        );
    }

    /// A reading for a missing chat ID yields `None` rather than erroring.
    #[tokio::test]
    async fn history_returns_none_when_missing() {
        use serenity::all::MessageId;
        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        let loaded = db.history(MessageId::new(999)).await;
        assert!(
            matches!(loaded, Ok(None)),
            "a missing history reads back as None"
        );
    }

    /// Per-character overrides replace only the fields they supply, leaving the
    /// other global settings untouched.
    #[tokio::test]
    async fn resolved_model_settings_applies_only_supplied_overrides() {
        use crate::llm::ModelSettings;

        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        let global = ModelSettings {
            model: "global-model".to_owned(),
            temperature: 0.5_f32,
            ..ModelSettings::default()
        };
        assert!(
            db.upsert_model_settings(global).await.is_ok(),
            "storing the global settings should succeed"
        );

        let mut model_only = character("model-only", "Harry");
        model_only.set_model_settings(CharacterModelSettings {
            model: Some("char-model".to_owned()),
            temperature: None,
        });
        let model_resolved = db.resolved_model_settings(&model_only).await;
        assert_eq!(
            model_resolved.model, "char-model",
            "a model override replaces the global model"
        );
        assert_eq!(
            model_resolved.temperature.to_bits(),
            0.5_f32.to_bits(),
            "the global temperature is kept when only the model is overridden"
        );

        let mut temperature_only = character("temperature-only", "Harry");
        temperature_only.set_model_settings(CharacterModelSettings {
            model: None,
            temperature: Some(0.9_f32),
        });
        let temperature_resolved = db.resolved_model_settings(&temperature_only).await;
        assert_eq!(
            temperature_resolved.model, "global-model",
            "the global model is kept when only the temperature is overridden"
        );
        assert_eq!(
            temperature_resolved.temperature.to_bits(),
            0.9_f32.to_bits(),
            "a temperature override replaces the global temperature"
        );

        let plain_resolved = db.resolved_model_settings(&character("plain", "Harry")).await;
        assert_eq!(
            plain_resolved.model, "global-model",
            "a character with no override leaves the global model unchanged"
        );
    }

    /// A corrupt config file falls back to the default rather than erroring, so a
    /// bad write cannot wedge the bot.
    #[tokio::test]
    async fn model_settings_falls_back_to_default_on_corruption() {
        use crate::llm::ModelSettings;
        use tokio::fs;

        let opened = Database::temporary().await;
        assert!(
            opened.is_ok(),
            "opening a temporary database should succeed"
        );
        let Ok(db) = opened else { return };
        let written = fs::write(&db.model_settings_path(), b"{ not valid json").await;
        assert!(written.is_ok(), "writing the corrupt file should succeed");

        let settings = db.model_settings().await;
        assert_eq!(
            settings.model,
            ModelSettings::default().model,
            "a corrupt model-settings file falls back to the default"
        );
    }
}
