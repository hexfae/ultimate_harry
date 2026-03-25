use jiff::Zoned;
use miette::Diagnostic;
use serde::{Deserialize, Serialize};
use serenity::all::{ChannelId, MessageId, ReactionType, UserId};
use snafu::{OptionExt as _, ResultExt as _, Snafu};
use surrealdb::{RecordId, engine::any::Any};
use surrealdb::{Surreal, opt::auth::Root};

use crate::llm::ModelSettings;
use crate::models::{
    character::{Character, CharacterPages},
    history::History,
};

const ROOT: Root<'_> = Root {
    username: "root",
    password: "root",
};

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

pub struct Database(Surreal<Any>);

impl Database {
    /// Connects to a local database in debug mode, or creates a local
    /// ``SurrealKV`` one in release mode.
    pub async fn new() -> Result<Self, DatabaseError> {
        let db: Surreal<Any> = Surreal::init();
        #[cfg(debug_assertions)]
        {
            db.connect("ws://localhost:8000")
                .await
                .context(ConnectSnafu)?;
            db.signin(ROOT).await.context(ConnectSnafu)?;
        }
        #[cfg(not(debug_assertions))]
        db.connect("surrealkv://harry_database")
            .await
            .context(ConnectSnafu)?;
        db.use_ns("harry".to_owned())
            .use_db("harry".to_owned())
            .await
            .context(ConnectSnafu)?;
        Ok(Self(db))
    }

    pub async fn character(&self, id: &RecordId) -> Result<Option<Character>, DatabaseError> {
        self.0.select(id).await.context(GetSnafu)
    }

    pub async fn characters_by_similarity(
        &self,
        name: impl Into<String>,
    ) -> Result<Vec<Character>, DatabaseError> {
        self.0
            .query(MOST_SIMILAR_TO)
            .bind(("input", name.into()))
            .await
            .context(GetSnafu)?
            .take(2)
            .context(GetSnafu)
    }

    pub async fn characters_by_usage(&self) -> Result<Vec<Character>, DatabaseError> {
        self.0
            .query(BY_USAGE)
            .await
            .context(GetSnafu)?
            .take(0)
            .context(GetSnafu)
    }

    pub async fn random_characters(&self) -> Result<Vec<Character>, DatabaseError> {
        self.0
            .query(RANDOM)
            .await
            .context(GetSnafu)?
            .take(0)
            .context(GetSnafu)
    }

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

    pub async fn delete_character(
        &self,
        id: &RecordId,
        deleted_by: impl Into<UserId>,
    ) -> Result<Option<Character>, DatabaseError> {
        self.0
            .update(id)
            .merge(DeletedBy::from(deleted_by.into()))
            .await
            .context(DeleteSnafu)
    }

    pub async fn supersede_character(
        &self,
        new_id: RecordId,
        old_id: &RecordId,
    ) -> Result<Option<Character>, DatabaseError> {
        self.0
            .update(old_id)
            .merge(NextVersion::from(new_id))
            .await
            .context(UpdateSnafu)?
            .context(NoCharacterSnafu)
    }

    pub async fn insert_character_pages(
        &self,
        character_page: CharacterPages,
    ) -> Result<Option<CharacterPages>, DatabaseError> {
        self.0
            .insert(character_page.id())
            .content(character_page)
            .await
            .context(InsertSnafu)
    }

    pub async fn history(
        &self,
        id: impl Into<MessageId>,
    ) -> Result<Option<History>, DatabaseError> {
        self.0
            .select(RecordId::from(("history", id.into().to_string())))
            .await
            .context(GetSnafu)
    }

    pub async fn insert_history(&self, history: History) -> Result<Option<History>, DatabaseError> {
        self.0
            .insert(history.id())
            .content(history)
            .await
            .context(InsertSnafu)
    }

    pub async fn update_history(&self, history: History) -> Result<Option<History>, DatabaseError> {
        self.0
            .update(history.id())
            .content(history)
            .await
            .context(UpdateSnafu)
    }

    pub async fn upsert_user_emoji(
        &self,
        id: impl Into<UserId>,
        emoji: ReactionType,
    ) -> Result<Option<UserEmoji>, DatabaseError> {
        let user_id = id.into();
        let user_emoji = UserEmoji { user_id, emoji };
        self.0
            .upsert(("user_emoji", user_id.to_string()))
            .content(user_emoji)
            .await
            .context(InsertSnafu)
    }

    pub async fn user_emoji(&self) -> Result<Vec<UserEmoji>, DatabaseError> {
        self.0.select("user_emoji").await.context(GetSnafu)
    }

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

    pub async fn model_settings(&self) -> ModelSettings {
        self.0
            .select(("model_settings", "model_settings"))
            .await
            .unwrap_or_default()
            .unwrap_or_default()
    }

    pub async fn pins_channel(&self) -> ChannelId {
        let pin_channel: PinChannel = self
            .0
            .select(("pins_channel_id", "pins_channel_id"))
            .await
            .unwrap_or_default()
            .unwrap_or_default();
        pin_channel.channel_id
    }

    pub async fn upsert_pin_channel(
        &self,
        channel_id: ChannelId,
    ) -> Result<Option<ChannelId>, DatabaseError> {
        self.0
            .upsert(("pins_channel_id", "pins_channel_id"))
            .content(PinChannel { channel_id })
            .await
            .context(SetPinsChannelSnafu)
            .map(|c| c.map(|c: PinChannel| c.channel_id))
    }

    pub async fn substitute_name(&self, user_id: impl Into<UserId>) -> String {
        self.0
            .select::<Option<UserName>>(("user_name", user_id.into().to_string()))
            .await
            .unwrap_or_default()
            .map_or_else(|| "User".to_owned(), |u| u.name)
    }

    pub async fn upsert_user_name(
        &self,
        user_id: impl Into<UserId>,
        name: String,
    ) -> Result<Option<UserName>, DatabaseError> {
        let user_id = user_id.into();
        let user_name = UserName { user_id, name };
        self.0
            .upsert(("user_name", user_id.to_string()))
            .content(user_name)
            .await
            .context(InsertSnafu)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct PinChannel {
    channel_id: ChannelId,
}

#[derive(Serialize, Deserialize)]
pub struct UserName {
    pub user_id: UserId,
    pub name: String,
}

// TODO: this should probably be a HashMap<UserId, ReactionType> instead
#[derive(Serialize, Deserialize)]
pub struct UserEmoji {
    pub user_id: UserId,
    pub emoji: ReactionType,
}

#[derive(Serialize)]
struct DeletedBy {
    deleted_by: UserId,
    deleted_at: Zoned,
}

#[derive(Serialize)]
struct NextVersion {
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

#[derive(Debug, Snafu, Diagnostic)]
pub enum DatabaseError {
    #[snafu(display("Kunde inte ansluta till databasen: {source}"))]
    #[diagnostic(
        help("Kontrollera att databasen körs och är tillgänglig på adressen"),
        code(database::connect)
    )]
    Connect { source: surrealdb::Error },
    #[snafu(display("Kunde inte hämta från databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att posten finns"),
        code(database::get)
    )]
    Get { source: surrealdb::Error },
    #[snafu(display("Kunde inte infoga i databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att datat är korrekt"),
        code(database::insert)
    )]
    Insert { source: surrealdb::Error },
    #[snafu(display("Kunde inte ta bort från databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att posten finns"),
        code(database::delete)
    )]
    Delete { source: surrealdb::Error },
    #[snafu(display("Kunde inte uppdatera databasen: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att posten finns"),
        code(database::update)
    )]
    Update { source: surrealdb::Error },
    #[snafu(display("Ingen sådan karaktär hittades i databasen"))]
    #[diagnostic(
        help("Kontrollera namnet eller skapa en ny karaktär först"),
        code(database::no_character)
    )]
    NoCharacter,
    #[snafu(display("Kunde inte spara modellinställningar: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att inställningarna är giltiga"),
        code(database::set_model_settings)
    )]
    SetModelSettings { source: surrealdb::Error },
    #[snafu(display("Kunde inte spara kanal för pins: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är giltig"),
        code(database::set_pins_channel)
    )]
    SetPinsChannel { source: surrealdb::Error },
}
