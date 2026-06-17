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

    /// Returns up to 25 visible characters, sorted by the most commonly used ones.
    pub async fn characters_by_usage(&self) -> Result<Vec<Character>, DatabaseError> {
        let mut characters: Vec<Character> = self
            .all_characters()
            .await?
            .into_iter()
            .filter(Character::is_visible)
            .collect();
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
        let mut characters: Vec<Character> = self
            .all_characters()
            .await?
            .into_iter()
            .filter(Character::is_visible)
            .collect();
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
