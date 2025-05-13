use std::sync::LazyLock;

use miette::Diagnostic;
use serde::Serialize;
use serenity::all::{MessageId, UserId};
use snafu::{OptionExt, ResultExt, Snafu};
use surrealdb::{RecordId, Surreal, engine::remote::ws::Client};
use ultimate_character::Character;
use ultimate_history::History;

pub static DB: LazyLock<Database> = LazyLock::new(Database::init);

const MOST_SIMILAR_TO: &str = "
SELECT
    *,
    string::distance::normalized_damerau_levenshtein(name, $input) AS similarity_score
FROM character
WHERE
    deleted_at IS NONE
    AND next_version IS NONE
ORDER BY
    similarity_score DESC,
    conversations_had DESC;
";

const BY_USAGE: &str = "
SELECT
    *
FROM character
WHERE
    deleted_at IS NONE
    AND next_version IS NONE
ORDER BY
    conversations_had DESC;
";

pub struct Database(Surreal<Client>);

impl Database {
    pub fn init() -> Self {
        Self(Surreal::init())
    }

    pub async fn character(&self, id: RecordId) -> Result<Option<Character>, Error> {
        self.0.select(id).await.context(GetSnafu)
    }

    pub async fn characters_by_similarity(
        &self,
        name: impl Into<String>,
    ) -> Result<Vec<Character>, Error> {
        self.0
            .query(MOST_SIMILAR_TO)
            .bind(("name", name.into()))
            .await
            .context(GetSnafu)?
            .take(0)
            .context(GetSnafu)
    }

    pub async fn characters_by_usage(&self) -> Result<Vec<Character>, Error> {
        self.0
            .query(BY_USAGE)
            .await
            .context(GetSnafu)?
            .take(0)
            .context(GetSnafu)
    }

    pub async fn insert_character(&self, character: Character) -> Result<Option<Character>, Error> {
        self.0
            .insert(character.id())
            .content(character)
            .await
            .context(InsertSnafu)
    }

    pub async fn delete_character(
        &self,
        id: RecordId,
        deleted_by: impl Into<UserId>,
    ) -> Result<Option<Character>, Error> {
        self.0
            .update(id)
            .content(DeletedBy {
                deleted_by: deleted_by.into(),
            })
            .await
            .context(DeleteSnafu)
    }

    pub async fn supersede_character(
        &self,
        new_id: RecordId,
        old_id: RecordId,
    ) -> Result<(), Error> {
        self.0
            .update(old_id)
            .content(NextVersion {
                next_version: Some(new_id),
            })
            .await
            .context(UpdateSnafu)?
            .context(NoCharacterSnafu)
    }

    pub async fn history(&self, id: impl Into<MessageId>) -> Result<Option<History>, Error> {
        self.0
            .select(RecordId::from(("history", id.into().to_string())))
            .await
            .context(GetSnafu)
    }

    pub async fn insert_history(&self, history: History) -> Result<Option<History>, Error> {
        self.0
            .insert(history.id())
            .content(history)
            .await
            .context(InsertSnafu)
    }
}

#[derive(Serialize)]
struct DeletedBy {
    deleted_by: UserId,
}

#[derive(Serialize)]
struct NextVersion {
    next_version: Option<RecordId>,
}

#[derive(Debug, Snafu, Diagnostic)]
pub enum Error {
    #[snafu(display("Error getting from the database: {source}"))]
    Get { source: surrealdb::Error },
    #[snafu(display("Error inserting into the database: {source}"))]
    Insert { source: surrealdb::Error },
    #[snafu(display("Error deleting from the database: {source}"))]
    Delete { source: surrealdb::Error },
    #[snafu(display("Error updating the database: {source}"))]
    Update { source: surrealdb::Error },
    #[snafu(display("No such character found in the database"))]
    NoCharacter,
}
