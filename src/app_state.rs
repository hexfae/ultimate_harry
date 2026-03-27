//! The state of the app, containing the database and statistics.

use crate::database::{Database, DatabaseError};

/// The database and statistics of the bot.
#[derive(Debug)]
pub struct AppState {
    /// The `SurrealDB` database.
    pub db: Database,
}

impl AppState {
    /// Creates a new `AppState` by creating an embedded database and loading the statistics file.
    ///
    /// # Errors
    ///
    /// Returns an error if creating the embedded database (in release mode) or connecting
    /// to the remote database fails (in debug mode) fails.
    pub async fn new() -> Result<Self, DatabaseError> {
        let db = Database::new().await?;

        Ok(Self { db })
    }
}
