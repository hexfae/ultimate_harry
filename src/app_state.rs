use crate::{
    config::Config,
    db::{Database, DatabaseError},
    models::statistics::Statistics,
};
use std::sync::Arc;

pub struct AppState {
    pub config: Config,
    pub db: Database,
    pub stats: Arc<Statistics>,
}

pub type Error = miette::Report;
pub type Context<'a> = poise::Context<'a, AppState, Error>;

impl AppState {
    pub async fn new(config: Config) -> Result<Self, DatabaseError> {
        let db = Database::new().await?;
        let stats = Arc::new(Statistics::load().unwrap_or_default());

        Ok(Self { config, db, stats })
    }
}
