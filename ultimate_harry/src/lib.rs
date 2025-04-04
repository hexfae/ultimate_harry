pub type Context<'a> = poise::Context<'a, (), Error>;
pub type Result<T, E = Error> = std::result::Result<T, E>;
pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
