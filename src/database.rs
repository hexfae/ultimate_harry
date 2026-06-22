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
    native_db, transaction::RTransaction,
};
use native_model::{Model as _, native_model};
use nonempty::NonEmpty;
use serde::{Deserialize, Serialize};
use serenity::all::{ChannelId, MessageId, ReactionType, UserId};
use snafu::{IntoError, OptionExt as _, ResultExt as _, Snafu};
use std::collections::HashMap;
use std::sync::OnceLock;
use tracing::{error, warn};

use crate::constants::MAX_RESULTS;
use crate::llm::ModelSettings;
use crate::models::{
    character::{Character, CharacterOption},
    history::{History, StoredHistory, scaffolding},
    message::{DescribedAttachment, Message},
};

/// The path of the embedded `native_db` database file.
const DATABASE_PATH: &str = "harry_database.db";

/// The fixed primary key used for singleton records (model settings, pin channel).
const SINGLETON_KEY: &str = "global";

/// Resolves a single message ID against an open read transaction, warning (and
/// returning `None`) when the message is missing from the table.
async fn resolve_message(
    read: &RTransaction<'_>,
    id: &str,
) -> Result<Option<Message>, DatabaseError> {
    let found = read
        .get()
        .primary::<Message>(id.to_owned())
        .context(GetSnafu)?;
    if found.is_none() {
        warn!("history references a missing message: {id}");
    }
    Ok(found)
}

/// A newtype wrapper around a `native_db` database.
pub struct Database(NativeDatabase<'static>);

impl Debug for Database {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("Database").finish_non_exhaustive()
    }
}

/// Builds the set of `native_db` models for every persisted type, caching it so
/// the set is defined once and shared across every database opened in the process.
async fn models() -> Result<&'static Models, DatabaseError> {
    static MODELS: OnceLock<Models> = OnceLock::new();
    if let Some(models) = MODELS.get() {
        return Ok(models);
    }
    let mut models = Models::new();
    models.define::<Character>().context(DefineModelSnafu)?;
    models.define::<StoredHistory>().context(DefineModelSnafu)?;
    models.define::<Message>().context(DefineModelSnafu)?;
    models.define::<GlobalModelSettings>().context(DefineModelSnafu)?;
    models.define::<PinChannel>().context(DefineModelSnafu)?;
    models.define::<UserName>().context(DefineModelSnafu)?;
    models.define::<UserEmoji>().context(DefineModelSnafu)?;
    Ok(MODELS.get_or_init(|| models))
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

    /// Loads the character `id`, applies `apply`, and upserts it in a single
    /// transaction, returning the mutated character. Returns `Ok(None)` if the
    /// character is missing. `context` selects the error variant for any
    /// transaction failure, so each caller keeps its own error message.
    async fn mutate_character<C>(
        &self,
        id: &str,
        context: C,
        apply: impl FnOnce(&mut Character),
    ) -> Result<Option<Character>, DatabaseError>
    where
        C: IntoError<DatabaseError, Source = NativeError> + Copy,
    {
        let write = self.0.rw_transaction().context(context)?;
        let Some(mut character) = write
            .get()
            .primary::<Character>(id.to_owned())
            .context(context)?
        else {
            return Ok(None);
        };
        apply(&mut character);
        write.upsert(character.clone()).context(context)?;
        write.commit().context(context)?;
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
            if let Some(message) = resolve_message(&read, id).await? {
                messages.push(message);
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
            } else if let Some(message) = resolve_message(&read, id).await? {
                context.push(message);
            }
        }
        Ok(context)
    }

    /// Updates or inserts a chat history, writing its choice and pending messages to the message
    /// table and storing the history as message-ID lists.
    ///
    /// Each handler loads its own [`History`], mutates it, and saves it here across a separate
    /// transaction, so two near-simultaneous button presses on the same reply race and the last
    /// write wins (a lost swipe/edit). This is accepted: there is no per-conversation lock or
    /// version guard, because the bot serves a handful of users and the worst case is a single
    /// dropped mutation, never corruption (every write is a whole, valid record).
    pub async fn upsert_history(&self, history: History) -> Result<(), DatabaseError> {
        let (stored, messages) = history.into_stored();
        let write = self.0.rw_transaction().context(InsertSnafu)?;
        for message in messages {
            write.upsert(message).context(InsertSnafu)?;
        }
        write.upsert(stored).context(InsertSnafu)?;
        write.commit().context(InsertSnafu)
    }

    /// Merges vision-model descriptions into a stored message, keyed by attachment URL.
    ///
    /// Loads the message from the table, adds any descriptions for attachments it owns, and writes
    /// it back. A message that is not yet stored (still pending this turn) is a no-op.
    pub async fn cache_attachment_descriptions(
        &self,
        message_id: &str,
        descriptions: &[DescribedAttachment],
    ) -> Result<(), DatabaseError> {
        let stored = {
            let read = self.0.r_transaction().context(GetSnafu)?;
            read.get()
                .primary::<Message>(message_id.to_owned())
                .context(GetSnafu)?
        };
        let Some(mut message) = stored else {
            return Ok(());
        };
        message.add_descriptions(descriptions);
        let write = self.0.rw_transaction().context(UpdateSnafu)?;
        write.upsert(message).context(UpdateSnafu)?;
        write.commit().context(UpdateSnafu)
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
        self.mutate_character(id, UpdateSnafu, |character| {
            character.set_model_settings(model_settings);
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

    /// Reads a singleton-style record by its primary key, returning a default value rather than
    /// blocking the bot.
    ///
    /// A legitimately absent record defaults silently; a genuine transaction or lookup *failure*
    /// (corruption, lock contention) is logged at error level under `label`, since it would
    /// otherwise surface downstream as a confusing unrelated error (e.g. an empty API key).
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
                error!("failed to read {label}, using default: {why}");
                return default();
            }
        };
        match read.get().primary::<T>(key) {
            Ok(found) => found.map_or_else(&default, extract),
            Err(why) => {
                error!("failed to read {label}, using default: {why}");
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
            Some(settings) => settings.clone(),
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
    use core::ptr;
    use core::slice::from_ref;
    use crate::llm::ModelSettings;
    use crate::models::{
        character::Character,
        history::{History, scaffolding},
        message::{DescribedAttachment, Message, Role},
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

    /// `cache_attachment_descriptions` persists a description onto an already-stored message and
    /// is a harmless no-op for a message that is not in the table.
    #[tokio::test]
    async fn cache_attachment_descriptions_persists_onto_stored_messages() {
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
        );
        let Ok(db) = opened else { return };

        let image_message = Message::builder()
            .id(MessageId::new(7))
            .parts(("Alice".to_owned(), "Alice: hi".to_owned(), Role::User))
            .attachments(vec!["https://cdn/cat.png".to_owned()])
            .build();
        let message_id = image_message.id().to_owned();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("char-id")
            .choices(NonEmpty::new(image_message))
            .build();
        assert!(
            db.upsert_history(history).await.is_ok(),
            "storing the image message should succeed"
        );

        let descriptions = vec![DescribedAttachment {
            url: "https://cdn/cat.png".to_owned(),
            description: "en katt".to_owned(),
        }];
        assert!(
            db.cache_attachment_descriptions(&message_id, &descriptions)
                .await
                .is_ok(),
            "caching descriptions should succeed"
        );

        let reloaded = db
            .messages(from_ref(&message_id))
            .await
            .unwrap_or_default();
        let described = reloaded
            .first()
            .and_then(|message| message.description_for("https://cdn/cat.png"));
        assert_eq!(
            described,
            Some("en katt"),
            "the cached description is persisted onto the stored message"
        );

        let missing = db
            .cache_attachment_descriptions("not-stored", &descriptions)
            .await;
        assert!(
            missing.is_ok(),
            "caching onto a missing message is a no-op, not an error"
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

    /// `restore_character` clears a soft-deleted character's deleted state and
    /// returns it visible again; a missing character returns `None`.
    #[tokio::test]
    async fn restore_character_restores_a_deleted_character() {
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
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
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
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
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
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
        let opened = Database::in_memory().await;
        assert!(
            opened.is_ok(),
            "opening an in-memory database should succeed"
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

    /// The model set is built once and shared: repeated calls return the same
    /// reference rather than leaking a fresh allocation each time.
    #[tokio::test]
    async fn models_are_built_once_and_shared() {
        let first_models = super::models().await;
        let second_models = super::models().await;
        assert!(
            first_models.is_ok() && second_models.is_ok(),
            "building the model set should succeed"
        );
        let (Ok(first_ref), Ok(second_ref)) = (first_models, second_models) else {
            return;
        };
        assert!(
            ptr::eq(first_ref, second_ref),
            "repeated models() calls should return the same shared reference"
        );
    }
}
