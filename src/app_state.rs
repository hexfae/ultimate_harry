//! The state of the app, containing the database.

use crate::database::{Database, DatabaseError};
use tokio_util::task::TaskTracker;

/// The shared state of the bot.
#[derive(Debug)]
pub struct AppState {
    /// The JSON-file database.
    pub db: Database,
    /// Tracks in-flight event handlers so a graceful shutdown can wait for
    /// streaming replies to finish persisting their `History` before exiting.
    pub tasks: TaskTracker,
}

impl AppState {
    /// Creates a new `AppState` by opening the embedded database.
    ///
    /// # Errors
    ///
    /// Returns an error if opening the embedded database fails.
    pub async fn new() -> Result<Self, DatabaseError> {
        let db = Database::new().await?;

        Ok(Self {
            db,
            tasks: TaskTracker::new(),
        })
    }
}
