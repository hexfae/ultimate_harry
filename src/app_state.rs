//! The state of the app, containing the database and statistics.

use crate::database::{Database, DatabaseError};
use tokio_util::task::TaskTracker;

/// The database and statistics of the bot.
#[derive(Debug)]
pub struct AppState {
    /// The `native_db` database.
    pub db: Database,
    /// Tracks in-flight event handlers so a graceful shutdown can wait for
    /// streaming replies to finish persisting their `History` before exiting.
    pub tasks: TaskTracker,
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

        Ok(Self {
            db,
            tasks: TaskTracker::new(),
        })
    }
}
