pub mod app_state;
pub mod commands;
pub mod constants;
pub mod db;
pub mod events;
pub mod llm;
pub mod models;
pub mod traits;

pub type Result<T, E = miette::Report> = std::result::Result<T, E>;
pub type Context<'a> = poise::Context<'a, app_state::AppState, miette::Report>;
