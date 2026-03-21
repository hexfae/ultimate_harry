use crate::{
    db::{Database, DatabaseError},
    models::statistics::Statistics,
};
use std::sync::Arc;

pub struct AppState {
    pub db: Database,
    pub stats: Arc<Statistics>,
}

pub type Error = miette::Report;
pub type Context<'a> = poise::Context<'a, AppState, Error>;

impl AppState {
    pub async fn new() -> Result<Self, DatabaseError> {
        let db = Database::new().await?;
        let stats = Arc::new(Statistics::load().unwrap_or_default());

        Ok(Self { db, stats })
    }
}
