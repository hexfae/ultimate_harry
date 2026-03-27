//! A convenience wrapper implementing several methods on a `SurrealDB` database.

use jiff::Zoned;
use miette::{Diagnostic, SourceSpan};
use serde::{Deserialize, Serialize};
use serenity::all::{ChannelId, MessageId, ReactionType, UserId};
use snafu::{OptionExt as _, ResultExt as _, Snafu};
use surrealdb::{RecordId, engine::any::Any};
use surrealdb::{Surreal, opt::auth::Root};

use crate::llm::ModelSettings;
use crate::models::{
    character::{Character, ViewCharacterPages},
    history::History,
};

/// The credentials for the root user of the (local) remote `SurrealDB` database. Used in debug mode.
const ROOT: Root<'_> = Root {
    username: "root",
    password: "root",
};

/// The default address of the (local) remote `SurrealDB` database. Used in debug mode.
const REMOTE_DATABASE_PATH: &str = "ws://localhost:8000";
/// The default path of the embedded `SurrealDB` database. Used in release mode.
#[cfg(not(debug_assertions))]
const EMBEDDED_DATABASE_PATH: &str = "surrealkv://harry_database";

/// The default `SurrealDB` namespace.
const NAMESPACE: &str = "harry";
/// The default `SurrealDB` database.
const DATABASE: &str = "harry";

/// A newtype wrapper around `Surreal<Any>`.
#[derive(Debug)]
pub struct Database(Surreal<Any>);

impl Database {
    /// Connects to a local database in debug mode, or creates a local
    /// `SurrealKV` one in release mode.
    pub async fn new() -> Result<Self, DatabaseError> {
        let db: Surreal<Any> = Surreal::init();
        #[cfg(debug_assertions)]
        {
            db.connect(REMOTE_DATABASE_PATH)
                .await
                .context(ConnectSnafu {
                    found: REMOTE_DATABASE_PATH,
                    span: 0..REMOTE_DATABASE_PATH.len(),
                })?;
            db.signin(ROOT).await.context(SignInSnafu)?;
        };
        #[cfg(not(debug_assertions))]
        db.connect(EMBEDDED_DATABASE_PATH)
            .await
            .context(ConnectSnafu {
                found: EMBEDDED_DATABASE_PATH,
                span: 0..EMBEDDED_DATABASE_PATH.len(),
            })?;
        db.use_ns(NAMESPACE)
            .use_db(DATABASE)
            .await
            .with_context(|_| UseNamespaceDatabaseSnafu {
                namespace: NAMESPACE.to_owned(),
                database: DATABASE.to_owned(),
            })?;
        Ok(Self(db))
    }

    /// Returns a single character by its ID.
    pub async fn character(&self, id: &RecordId) -> Result<Option<Character>, DatabaseError> {
        self.0.select(id).await.context(GetSnafu)
    }

    /// Returns up to 25 characters, sorted by the most similar ones to the given name.
    pub async fn characters_by_similarity<T: Into<String>>(
        &self,
        name: T,
    ) -> Result<Vec<Character>, DatabaseError> {
        self.0
            .query(MOST_SIMILAR_TO)
            .bind(("input", name.into()))
            .await
            .context(GetSnafu)?
            .take(2)
            .context(GetSnafu)
    }

    /// Returns up to 25 characters, sorted by the most commonly used ones.
    pub async fn characters_by_usage(&self) -> Result<Vec<Character>, DatabaseError> {
        self.0
            .query(BY_USAGE)
            .await
            .context(GetSnafu)?
            .take(0)
            .context(GetSnafu)
    }

    /// Returns up to 25 characters, sorted randomly.
    pub async fn random_characters(&self) -> Result<Vec<Character>, DatabaseError> {
        self.0
            .query(RANDOM)
            .await
            .context(GetSnafu)?
            .take(0)
            .context(GetSnafu)
    }

    /// Inserts a character.
    pub async fn insert_character(
        &self,
        character: Character,
    ) -> Result<Option<Character>, DatabaseError> {
        self.0
            .insert(character.id())
            .content(character)
            .await
            .context(InsertSnafu)
    }

    /// Deletes a character.
    pub async fn delete_character<T: Into<UserId>>(
        &self,
        id: &RecordId,
        deleted_by: T,
    ) -> Result<Option<Character>, DatabaseError> {
        self.0
            .update(id)
            .merge(DeletedBy::from(deleted_by.into()))
            .await
            .context(DeleteSnafu)
    }

    /// Sets the `next_version` field on the given old character ID to point to the given new character ID.
    pub async fn supersede_character(
        &self,
        new_id: RecordId,
        old_id: &RecordId,
    ) -> Result<Option<Character>, DatabaseError> {
        self.0
            .update(old_id)
            .merge(NextVersion::from(new_id.clone()))
            .await
            .context(UpdateSnafu)?
            .with_context(|| NoCharacterSnafu {
                found: new_id.to_string(),
                span: (0..new_id.to_string().len()),
            })
    }

    /// Inserts a list of character pages.
    pub async fn insert_character_pages(
        &self,
        character_page: ViewCharacterPages,
    ) -> Result<Option<ViewCharacterPages>, DatabaseError> {
        self.0
            .insert(character_page.id())
            .content(character_page)
            .await
            .context(InsertSnafu)
    }

    /// Returns a chat history by its ID.
    pub async fn history<T: Into<MessageId>>(
        &self,
        id: T,
    ) -> Result<Option<History>, DatabaseError> {
        self.0
            .select(RecordId::from(("history", id.into().to_string())))
            .await
            .context(GetSnafu)
    }

    /// Updates or inserts a chat history.
    pub async fn upsert_history(&self, history: History) -> Result<Option<History>, DatabaseError> {
        self.0
            .upsert(history.id())
            .content(history)
            .await
            .context(InsertSnafu)
    }

    /// Updates or inserts a user's emoji.
    pub async fn upsert_user_emoji<T: Into<UserId>>(
        &self,
        id: T,
        emoji: ReactionType,
    ) -> Result<Option<UserEmoji>, DatabaseError> {
        let user_id = id.into();
        let user_emoji = UserEmoji { emoji, user_id };
        self.0
            .upsert(("user_emoji", user_id.to_string()))
            .content(user_emoji)
            .await
            .context(InsertSnafu)
    }

    /// Returns all set user emoji.
    pub async fn user_emoji(&self) -> Result<Vec<UserEmoji>, DatabaseError> {
        self.0.select("user_emoji").await.context(GetSnafu)
    }

    /// Updates or inserts the bot's AI model settings.
    pub async fn upsert_model_settings(
        &self,
        model_settings: ModelSettings,
    ) -> Result<Option<ModelSettings>, DatabaseError> {
        self.0
            .upsert(("model_settings", "model_settings"))
            .content(model_settings)
            .await
            .context(SetModelSettingsSnafu)
    }

    /// Returns the bot's AI model settings.
    pub async fn model_settings(&self) -> ModelSettings {
        self.0
            .select(("model_settings", "model_settings"))
            .await
            .unwrap_or_default()
            .unwrap_or_default()
    }

    /// Returns the bot's pin channel.
    pub async fn pins_channel(&self) -> ChannelId {
        let pin_channel: PinChannel = self
            .0
            .select(("pins_channel_id", "pins_channel_id"))
            .await
            .unwrap_or_default()
            .unwrap_or_default();
        pin_channel.channel_id
    }

    /// Updates or inserts the bot's pin channel.
    pub async fn upsert_pin_channel(
        &self,
        channel_id: ChannelId,
    ) -> Result<Option<ChannelId>, DatabaseError> {
        self.0
            .upsert(("pins_channel_id", "pins_channel_id"))
            .content(PinChannel { channel_id })
            .await
            .context(SetPinsChannelSnafu)
            .map(|option_channel| {
                option_channel.map(|pin_channel: PinChannel| pin_channel.channel_id)
            })
    }

    /// Returns a user's display name by their Discord user ID.
    pub async fn substitute_name<T: Into<UserId>>(&self, user_id: T) -> String {
        self.0
            .select::<Option<UserName>>(("user_name", user_id.into().to_string()))
            .await
            .unwrap_or_default()
            .map_or_else(|| "User".to_owned(), |user| user.name)
    }

    /// Updates or inserts a user's display name by their Discord user ID.
    pub async fn upsert_user_name<T: Into<UserId>>(
        &self,
        user_id: T,
        name: String,
    ) -> Result<Option<UserName>, DatabaseError> {
        let id = user_id.into();
        let user_name = UserName { user_id: id, name };
        self.0
            .upsert(("user_name", id.to_string()))
            .content(user_name)
            .await
            .context(InsertSnafu)
    }
}

/// The Discord channel where pins should go.
#[derive(Default, Serialize, Deserialize)]
pub struct PinChannel {
    /// The Discord channel's id.
    channel_id: ChannelId,
}

/// A user's display name and Discord user ID.
#[derive(Debug, Serialize, Deserialize)]
pub struct UserName {
    /// The user's set display name.
    pub name: String,
    /// The user's discord ID.
    pub user_id: UserId,
}

// TODO: this should probably be a HashMap<UserId, ReactionType> instead
/// A user's emoji and Discord user ID.
#[derive(Debug, Serialize, Deserialize)]
pub struct UserEmoji {
    /// The user's set emoji.
    pub emoji: ReactionType,
    /// The user's discord ID.
    pub user_id: UserId,
}

/// The timestamp of a deleted character and the Discord user ID of the user who deleted it.
#[derive(Serialize)]
struct DeletedBy {
    /// The user's discord ID.
    deleted_by: UserId,
    /// The time the character was deleted.
    deleted_at: Zoned,
}

/// The ID of the next version of a character.
#[derive(Serialize)]
struct NextVersion {
    /// The new version's ID.
    next_version: RecordId,
}

impl From<UserId> for DeletedBy {
    fn from(deleted_by: UserId) -> Self {
        let deleted_at = Zoned::now();
        Self {
            deleted_by,
            deleted_at,
        }
    }
}

impl From<RecordId> for NextVersion {
    fn from(next_version: RecordId) -> Self {
        Self { next_version }
    }
}

/// All errors that can happen when interacting with the database.
#[derive(Debug, Snafu, Diagnostic)]
pub enum DatabaseError {
    /// Connecting to a database failed.
    #[snafu(display("Could not connect to the database"))]
    #[diagnostic(
        help("Make sure the database is running and available at the given address"),
        code(database::connect)
    )]
    Connect {
        /// The source of the error.
        source: surrealdb::Error,
        /// The address that was tried.
        #[source_code]
        found: String,
        /// The part that was wrong (in practice, the entire address is selected).
        #[label]
        span: SourceSpan,
    },
    /// Signing in to a remote database failed.
    #[snafu(display("Could not sign in to the database"))]
    #[diagnostic(
        help("Make sure the database has the correct username and password set"),
        code(database::sign_in)
    )]
    SignIn {
        /// The source of the error.
        source: surrealdb::Error,
    },
    /// Using a namespace or database failed.
    #[snafu(display("Could not use the namespace {namespace} or database {database}"))]
    #[diagnostic(
        help("Make sure the database has the given namespace or database"),
        code(database::use_namespace_database)
    )]
    UseNamespaceDatabase {
        /// The source of the error.
        source: surrealdb::Error,
        /// The namespace that was tried.
        namespace: String,
        /// The database that was tried.
        database: String,
    },
    /// Getting a record failed.
    #[snafu(display("Kunde inte hämta från databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att posten finns"),
        code(database::get)
    )]
    Get {
        /// The source of the error.
        source: surrealdb::Error,
    },
    /// Inserting a record failed.
    #[snafu(display("Kunde inte infoga i databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att datat är korrekt"),
        code(database::insert)
    )]
    Insert {
        /// The source of the error.
        source: surrealdb::Error,
    },
    /// Deleting a record failed.
    #[snafu(display("Kunde inte ta bort från databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att posten finns"),
        code(database::delete)
    )]
    Delete {
        /// The source of the error.
        source: surrealdb::Error,
    },
    /// Updating a record failed.
    #[snafu(display("Kunde inte uppdatera databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att posten finns"),
        code(database::update)
    )]
    Update {
        /// The source of the error.
        source: surrealdb::Error,
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
        source: surrealdb::Error,
    },
    /// Setting the bot's pin Discord channel failed.
    #[snafu(display("Kunde inte spara kanal för pins: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är giltig"),
        code(database::set_pins_channel)
    )]
    SetPinsChannel {
        /// The source of the error.
        source: surrealdb::Error,
    },
}

/// `SurrealQL` query returning up to 25 characters, sorted by the similarity of the names to the input.
///
/// It uses normalized damerau levenshtein distance.
const MOST_SIMILAR_TO: &str = "
LET $names = SELECT
    *,
    string::distance::normalized_damerau_levenshtein(name, $input) AS similarity
FROM
    character
WHERE
    deleted_at IS NONE
    AND next_version IS NONE
    AND nickname IS NONE
;

let $nicknames = SELECT
    *,
    math::max([
        string::distance::normalized_damerau_levenshtein(name, $input),
        string::distance::normalized_damerau_levenshtein(nickname, $input)
    ]) AS similarity
FROM
    character
WHERE
    deleted_at IS NONE
    AND next_version IS NONE
    AND nickname IS not(NONE)
;

RETURN (
    SELECT
        *
    FROM
        array::concat($names, $nicknames)
    ORDER BY
        similarity DESC,
        conversations_had DESC
    LIMIT 25
);
";

/// `SurrealQL` query returning up to 25 characters, sorted randomly.
const RANDOM: &str = "
SELECT
    *
FROM
    character
WHERE
    deleted_at IS NONE
    AND next_version IS NONE
ORDER BY
    rand()
LIMIT 25
";

/// `SurrealQL` query returning up to 25 characters, sorted by how commonly they're used.
const BY_USAGE: &str = "
SELECT
    *
FROM
    character
WHERE
    deleted_at IS NONE
    AND next_version IS NONE
ORDER BY
    conversations_had DESC;
";
