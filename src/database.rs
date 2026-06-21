//! A convenience wrapper implementing several methods on a `native_db` database.
#![expect(
    clippy::unused_async,
    reason = "the native_db backend is synchronous, but the Database API is kept async for uniformity and to avoid rippling .await removal across every call site"
)]

use core::fmt::{Debug, Formatter, Result as FmtResult};
use miette::{Diagnostic, SourceSpan};
use nanorand::Rng as _;
use native_db::{
    Builder, Database as NativeDatabase, Models, ToInput, ToKey as _, db_type::Error as NativeError,
    native_db,
};
use native_model::{Model as _, native_model};
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use serenity::all::{ChannelId, MessageId, ReactionType, UserId};
use snafu::{OptionExt as _, ResultExt as _, Snafu};
use std::collections::HashMap;
use tracing::warn;

use crate::llm::ModelSettings;
use crate::models::{
    character::{Character, CharacterOption},
    history::{History, StoredHistory, scaffolding},
    message::Message,
};

/// The path of the embedded `native_db` database file.
const DATABASE_PATH: &str = "harry_database.db";

/// The fixed primary key used for singleton records (model settings, pin channel).
const SINGLETON_KEY: &str = "global";

/// The maximum number of characters returned by the listing queries.
const MAX_RESULTS: usize = 25;

/// A newtype wrapper around a `native_db` database.
pub struct Database(NativeDatabase<'static>);

impl Debug for Database {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

/// Builds the set of `native_db` models for every persisted type.
async fn models() -> Result<&'static Models, DatabaseError> {
    let mut models = Models::new();
    models.define::<Character>().context(DefineModelSnafu)?;
    models.define::<StoredHistory>().context(DefineModelSnafu)?;
    models.define::<Message>().context(DefineModelSnafu)?;
    models.define::<GlobalModelSettings>().context(DefineModelSnafu)?;
    models.define::<PinChannel>().context(DefineModelSnafu)?;
    models.define::<UserName>().context(DefineModelSnafu)?;
    models.define::<UserEmoji>().context(DefineModelSnafu)?;
    Ok(Box::leak(Box::new(models)))
}

impl Database {
    /// Opens (or creates) the embedded database file.
    pub async fn new() -> Result<Self, DatabaseError> {
        let models = models().await?;
        let db = Builder::new()
            .create(models, DATABASE_PATH)
            .context(ConnectSnafu)?;
        Ok(Self(db))
    }

    /// Opens an ephemeral in-memory database for tests.
    #[cfg(test)]
    async fn in_memory() -> Result<Self, DatabaseError> {
        let models = models().await?;
        let db = Builder::new()
            .create_in_memory(models)
            .context(ConnectSnafu)?;
        Ok(Self(db))
    }

    /// Returns every character currently stored, regardless of visibility.
    async fn all_characters(&self) -> Result<Vec<Character>, DatabaseError> {
        let read = self.0.r_transaction().context(GetSnafu)?;
        read.scan()
            .primary::<Character>()
            .context(GetSnafu)?
            .all()
            .context(GetSnafu)?
            .collect::<Result<Vec<Character>, NativeError>>()
            .context(GetSnafu)
    }

    /// Returns a single character by its ID.
    pub async fn character(&self, id: &str) -> Result<Option<Character>, DatabaseError> {
        let read = self.0.r_transaction().context(GetSnafu)?;
        read.get().primary(id.to_owned()).context(GetSnafu)
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
        let write = self.0.rw_transaction().context(InsertSnafu)?;
        write.upsert(character).context(InsertSnafu)?;
        write.commit().context(InsertSnafu)
    }

    /// Soft-deletes a character by recording its deleter and the time of deletion.
    pub async fn delete_character<T: Into<UserId>>(
        &self,
        id: &str,
        deleted_by: T,
    ) -> Result<Option<Character>, DatabaseError> {
        let write = self.0.rw_transaction().context(DeleteSnafu)?;
        let Some(mut character) = write
            .get()
            .primary::<Character>(id.to_owned())
            .context(DeleteSnafu)?
        else {
            return Ok(None);
        };
        character.mark_deleted(deleted_by.into());
        write.upsert(character.clone()).context(DeleteSnafu)?;
        write.commit().context(DeleteSnafu)?;
        Ok(Some(character))
    }

    /// Sets the `next_version` field on the given old character ID to point to the given new character ID.
    pub async fn supersede_character(
        &self,
        new_id: String,
        old_id: &str,
    ) -> Result<Option<Character>, DatabaseError> {
        let write = self.0.rw_transaction().context(UpdateSnafu)?;
        let mut character = write
            .get()
            .primary::<Character>(old_id.to_owned())
            .context(UpdateSnafu)?
            .with_context(|| NoCharacterSnafu {
                found: old_id.to_owned(),
                span: 0..old_id.len(),
            })?;
        character.set_next_version(new_id);
        write.upsert(character.clone()).context(UpdateSnafu)?;
        write.commit().context(UpdateSnafu)?;
        Ok(Some(character))
    }

    /// Returns a chat history by its ID, hydrating its choice messages from the message table.
    pub async fn history<T: Into<MessageId>>(
        &self,
        id: T,
    ) -> Result<Option<History>, DatabaseError> {
        let maybe_stored = {
            let read = self.0.r_transaction().context(GetSnafu)?;
            read.get()
                .primary::<StoredHistory>(id.into().to_string())
                .context(GetSnafu)?
        };
        let Some(stored) = maybe_stored else {
            return Ok(None);
        };
        let Some(choices) = NonEmpty::from_vec(self.messages(&stored.choices).await?) else {
            warn!("history {} has no resolvable choices", stored.id);
            return Ok(None);
        };
        Ok(Some(History::hydrate(stored, choices)))
    }

    /// Resolves a list of message IDs into messages, in order, skipping any that are missing.
    pub async fn messages(&self, ids: &[String]) -> Result<Vec<Message>, DatabaseError> {
        let read = self.0.r_transaction().context(GetSnafu)?;
        let mut messages = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(message) = read
                .get()
                .primary::<Message>(id.clone())
                .context(GetSnafu)?
            {
                messages.push(message);
            } else {
                warn!("history references a missing message: {id}");
            }
        }
        Ok(messages)
    }

    /// Builds the full LLM context for a history: the character's scaffolding followed by the
    /// previous messages, in order.
    ///
    /// Previous messages just pushed this turn live in the history's `pending` buffer (not yet in
    /// the table), so those are taken from memory and the rest are resolved from the message table.
    pub async fn build_context(
        &self,
        history: &History,
        character: &Character,
    ) -> Result<Vec<Message>, DatabaseError> {
        let mut context = scaffolding(character);
        let pending: HashMap<&str, &Message> = history
            .pending()
            .iter()
            .map(|message| (message.id(), message))
            .collect();
        let read = self.0.r_transaction().context(GetSnafu)?;
        for id in history.previous_ids() {
            if let Some(message) = pending.get(id.as_str()) {
                context.push((*message).clone());
            } else if let Some(message) =
                read.get().primary::<Message>(id.clone()).context(GetSnafu)?
            {
                context.push(message);
            } else {
                warn!("history references a missing message: {id}");
            }
        }
        Ok(context)
    }

    /// Updates or inserts a chat history, writing its choice and pending messages to the message
    /// table and storing the history as message-ID lists.
    pub async fn upsert_history(&self, history: History) -> Result<(), DatabaseError> {
        let (stored, messages) = history.into_stored();
        let write = self.0.rw_transaction().context(InsertSnafu)?;
        for message in messages {
            write.upsert(message).context(InsertSnafu)?;
        }
        write.upsert(stored).context(InsertSnafu)?;
        write.commit().context(InsertSnafu)
    }

    /// Updates or inserts a user's emoji.
    pub async fn upsert_user_emoji<T: Into<UserId>>(
        &self,
        id: T,
        emoji: ReactionType,
    ) -> Result<(), DatabaseError> {
        let user_emoji = UserEmoji {
            emoji,
            user_id: id.into().to_string(),
        };
        let write = self.0.rw_transaction().context(InsertSnafu)?;
        write.upsert(user_emoji).context(InsertSnafu)?;
        write.commit().context(InsertSnafu)
    }

    /// Returns all set user emoji.
    pub async fn user_emoji(&self) -> Result<Vec<UserEmoji>, DatabaseError> {
        let read = self.0.r_transaction().context(GetSnafu)?;
        read.scan()
            .primary::<UserEmoji>()
            .context(GetSnafu)?
            .all()
            .context(GetSnafu)?
            .collect::<Result<Vec<UserEmoji>, NativeError>>()
            .context(GetSnafu)
    }

    /// Updates or inserts the bot's AI model settings.
    pub async fn upsert_model_settings(
        &self,
        model_settings: ModelSettings,
    ) -> Result<(), DatabaseError> {
        let stored = GlobalModelSettings {
            id: SINGLETON_KEY.to_owned(),
            settings: model_settings,
        };
        let write = self.0.rw_transaction().context(SetModelSettingsSnafu)?;
        write.upsert(stored).context(SetModelSettingsSnafu)?;
        write.commit().context(SetModelSettingsSnafu)
    }

    /// Sets a character's AI model settings override.
    pub async fn set_character_model_settings(
        &self,
        id: &str,
        model_settings: ModelSettings,
    ) -> Result<Option<Character>, DatabaseError> {
        let write = self.0.rw_transaction().context(UpdateSnafu)?;
        let Some(mut character) = write
            .get()
            .primary::<Character>(id.to_owned())
            .context(UpdateSnafu)?
        else {
            return Ok(None);
        };
        character.set_model_settings(model_settings);
        write.upsert(character.clone()).context(UpdateSnafu)?;
        write.commit().context(UpdateSnafu)?;
        Ok(Some(character))
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
    /// upserting the result in a single transaction. `label` names the stat for
    /// the missing-character warning. Best-effort: warns and returns `Ok` if the
    /// character is missing.
    async fn record_on_latest_version(
        &self,
        id: &str,
        label: &str,
        apply: impl FnOnce(&mut Character),
    ) -> Result<(), DatabaseError> {
        let write = self.0.rw_transaction().context(UpdateSnafu)?;
        let Some(mut character) = write
            .get()
            .primary::<Character>(id.to_owned())
            .context(UpdateSnafu)?
        else {
            warn!("tried to record {label} for a missing character: {id}");
            return Ok(());
        };
        while let Some(next_id) = character.next_version().map(str::to_owned) {
            let Some(next) = write
                .get()
                .primary::<Character>(next_id)
                .context(UpdateSnafu)?
            else {
                break;
            };
            character = next;
        }
        apply(&mut character);
        write.upsert(character).context(UpdateSnafu)?;
        write.commit().context(UpdateSnafu)
    }

    /// Reads a singleton-style record by its primary key, returning a default
    /// value (after logging a warning under `label`) on any transaction or
    /// lookup failure.
    fn read_or<T: ToInput, R>(
        &self,
        key: String,
        label: &str,
        default: impl Fn() -> R,
        extract: impl FnOnce(T) -> R,
    ) -> R {
        let read = match self.0.r_transaction() {
            Ok(read) => read,
            Err(why) => {
                warn!("failed to read {label}, using default: {why}");
                return default();
            }
        };
        match read.get().primary::<T>(key) {
            Ok(found) => found.map_or_else(&default, extract),
            Err(why) => {
                warn!("failed to read {label}, using default: {why}");
                default()
            }
        }
    }

    /// Returns the bot's AI model settings.
    pub async fn model_settings(&self) -> ModelSettings {
        self.read_or::<GlobalModelSettings, _>(
            SINGLETON_KEY.to_owned(),
            "model settings",
            ModelSettings::default,
            |found| found.settings,
        )
    }

    /// Returns the character's own model settings, falling back to the
    /// bot's global settings only when the character has no override.
    pub async fn resolved_model_settings(&self, character: &Character) -> ModelSettings {
        match character.model_settings() {
            Some(settings) => settings,
            None => self.model_settings().await,
        }
    }

    /// Returns the bot's pin channel.
    pub async fn pins_channel(&self) -> ChannelId {
        self.read_or::<PinChannel, _>(
            SINGLETON_KEY.to_owned(),
            "pin channel",
            ChannelId::default,
            |found| found.channel_id,
        )
    }

    /// Updates or inserts the bot's pin channel.
    pub async fn upsert_pin_channel(
        &self,
        channel_id: ChannelId,
    ) -> Result<ChannelId, DatabaseError> {
        let pin_channel = PinChannel {
            id: SINGLETON_KEY.to_owned(),
            channel_id,
        };
        let write = self.0.rw_transaction().context(SetPinsChannelSnafu)?;
        write.upsert(pin_channel).context(SetPinsChannelSnafu)?;
        write.commit().context(SetPinsChannelSnafu)?;
        Ok(channel_id)
    }

    /// Returns a user's display name by their Discord user ID.
    pub async fn substitute_name<T: Into<UserId>>(&self, user_id: T) -> String {
        self.read_or::<UserName, _>(
            user_id.into().to_string(),
            "substitute name",
            || "User".to_owned(),
            |found| found.name,
        )
    }

    /// Updates or inserts a user's display name by their Discord user ID.
    pub async fn upsert_user_name<T: Into<UserId>>(
        &self,
        user_id: T,
        name: String,
    ) -> Result<(), DatabaseError> {
        let user_name = UserName {
            user_id: user_id.into().to_string(),
            name,
        };
        let write = self.0.rw_transaction().context(InsertSnafu)?;
        write.upsert(user_name).context(InsertSnafu)?;
        write.commit().context(InsertSnafu)
    }
}

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

/// The bot's AI model settings, stored as a singleton.
#[derive(Debug, Serialize, Deserialize)]
#[native_model(id = 5, version = 1, with = crate::codec::Json)]
#[native_db]
struct GlobalModelSettings {
    /// The fixed singleton primary key.
    #[primary_key]
    id: String,
    /// The model settings.
    settings: ModelSettings,
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

/// All errors that can happen when interacting with the database.
#[derive(Debug, Snafu, Diagnostic)]
pub enum DatabaseError {
    /// Defining a database model failed.
    #[snafu(display("Could not define a database model"))]
    #[diagnostic(
        help("This is a programming error: two models likely share a native_model id"),
        code(database::define_model)
    )]
    DefineModel {
        /// The source of the error.
        source: NativeError,
    },
    /// Opening the database failed.
    #[snafu(display("Could not open the database"))]
    #[diagnostic(
        help("Make sure the database file is accessible and not corrupted"),
        code(database::connect)
    )]
    Connect {
        /// The source of the error.
        source: NativeError,
    },
    /// Getting a record failed.
    #[snafu(display("Kunde inte hämta från databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att posten finns"),
        code(database::get)
    )]
    Get {
        /// The source of the error.
        source: NativeError,
    },
    /// Inserting a record failed.
    #[snafu(display("Kunde inte infoga i databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att datat är korrekt"),
        code(database::insert)
    )]
    Insert {
        /// The source of the error.
        source: NativeError,
    },
    /// Deleting a record failed.
    #[snafu(display("Kunde inte ta bort från databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att posten finns"),
        code(database::delete)
    )]
    Delete {
        /// The source of the error.
        source: NativeError,
    },
    /// Updating a record failed.
    #[snafu(display("Kunde inte uppdatera databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att posten finns"),
        code(database::update)
    )]
    Update {
        /// The source of the error.
        source: NativeError,
    },
    /// No character by the given ID was found.
    #[snafu(display("Ingen sådan karaktär hittades i databasen: {found}"))]
    #[diagnostic(
        help("Kontrollera namnet eller skapa en ny karaktär först"),
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
    #[snafu(display("Kunde inte spara modellinställningar: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att inställningarna är giltiga"),
        code(database::set_model_settings)
    )]
    SetModelSettings {
        /// The source of the error.
        source: NativeError,
    },
    /// Setting the bot's pin Discord channel failed.
    #[snafu(display("Kunde inte spara kanal för pins: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är giltig"),
        code(database::set_pins_channel)
    )]
    SetPinsChannel {
        /// The source of the error.
        source: NativeError,
    },
}

/// Characterization tests pinning the transaction methods that the
/// load-mutate-upsert and message-resolution refactors touch. They run against
/// an ephemeral in-memory database.
#[cfg(test)]
mod tests {
    use super::Database;
    use crate::llm::ModelSettings;
    use crate::models::{
        character::Character,
        history::{History, scaffolding},
        message::Message,
    };
    use nonempty::NonEmpty;
    use serenity::all::{MessageId, UserId};

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
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
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
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
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
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
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
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
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
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
        );
        let Ok(db) = opened else { return };
        insert(&db, character("char-id", "Harry")).await;

        let updated = db
            .set_character_model_settings("char-id", ModelSettings::default())
            .await;
        assert!(
            updated
                .as_ref()
                .is_ok_and(|found| found.as_ref().is_some_and(Character::has_model_settings)),
            "setting model settings records the override and returns the character"
        );

        let missing = db
            .set_character_model_settings("missing", ModelSettings::default())
            .await;
        assert!(
            matches!(missing, Ok(None)),
            "setting model settings on a missing character returns None"
        );
    }

    /// `record_character_generation` applies stats to the latest version, walking
    /// past an edit in the version chain.
    #[tokio::test]
    async fn record_character_generation_lands_on_the_latest_version() {
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
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
            latest.is_some_and(|record| record.conversations_had() == 0),
            "generation stats land on the latest version, not the spawn count"
        );
        let original = db.character("old-id").await.ok().flatten();
        assert!(
            original.is_some(),
            "the original version is left intact"
        );
    }

    /// `messages` resolves IDs in order and silently skips any that are missing.
    #[tokio::test]
    async fn messages_resolve_in_order_and_skip_missing() {
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
        );
        let Ok(db) = opened else { return };
        let first = Message::new_system("first");
        let second = Message::new_system("second");
        let (first_id, second_id) = (first.id().to_owned(), second.id().to_owned());

        let history = History::builder()
            .id(MessageId::new(1))
            .character("char-id")
            .choices({
                let mut choices = NonEmpty::new(first);
                choices.push(second);
                choices
            })
            .build();
        assert!(
            db.upsert_history(history).await.is_ok(),
            "writing the messages should succeed"
        );

        let ids = vec![first_id.clone(), "missing".to_owned(), second_id.clone()];
        let resolved = db.messages(&ids).await.unwrap_or_default();
        let resolved_ids = resolved
            .iter()
            .map(|message| message.id().to_owned())
            .collect::<Vec<String>>();
        assert_eq!(
            resolved_ids,
            vec![first_id, second_id],
            "messages resolve in request order, skipping the missing ID"
        );
    }

    /// `build_context` prefixes the character scaffolding, then resolves each
    /// previous ID, preferring the in-memory pending buffer over the table and
    /// skipping missing IDs.
    #[tokio::test]
    async fn build_context_prefixes_scaffolding_and_prefers_pending() {
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
        );
        let Ok(db) = opened else { return };
        let character = character("char-id", "Harry");

        let table_message = Message::new_user("Alice", "from the table");
        let table_id = table_message.id().to_owned();
        let choice = Message::new_system("greeting");
        let table_history = History::builder()
            .id(MessageId::new(1))
            .character("char-id")
            .choices(NonEmpty::new(choice))
            .pending(vec![table_message])
            .previous(vec![table_id.clone()])
            .build();
        assert!(
            db.upsert_history(table_history).await.is_ok(),
            "writing the table message should succeed"
        );

        let pending_message = Message::new_user("Bob", "from pending");
        let pending_id = pending_message.id().to_owned();
        let context_history = History::builder()
            .id(MessageId::new(2))
            .character("char-id")
            .choices(NonEmpty::new(Message::new_system("greeting")))
            .pending(vec![pending_message])
            .previous(vec![pending_id.clone(), table_id.clone(), "missing".to_owned()])
            .build();

        let context = db
            .build_context(&context_history, &character)
            .await
            .unwrap_or_default();
        let scaffolding_len = scaffolding(&character).len();
        assert_eq!(
            context.len(),
            scaffolding_len.saturating_add(2),
            "context is scaffolding plus the two resolvable previous messages"
        );
        let tail_ids = context
            .iter()
            .skip(scaffolding_len)
            .map(|message| message.id().to_owned())
            .collect::<Vec<String>>();
        assert_eq!(
            tail_ids,
            vec![pending_id, table_id],
            "the pending message is taken from memory, the other from the table, missing skipped"
        );
    }
}
