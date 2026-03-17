use std::sync::Arc;

use miette::{IntoDiagnostic, Report};
use poise::{
    Framework, FrameworkOptions, PrefixFrameworkOptions,
    serenity_prelude::{ClientBuilder, GatewayIntents, Token},
};
use ultimate_harry::{
    app_state::AppState,
    commands::{character, chat, emoji, model, pin},
    config::Config,
    events::EventHandler,
};

const INTENTS: GatewayIntents =
    GatewayIntents::non_privileged().union(GatewayIntents::MESSAGE_CONTENT);

#[tokio::main]
async fn main() -> Result<(), Report> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install aws-lc-rs rustls provider");
    tracing_subscriber::fmt::init();

    // 1. Load config exactly once.
    let config_path = std::env::var("CONFIG_FILE").unwrap_or_else(|_| "config.toml".to_owned());
    let config = Config::load(&config_path).into_diagnostic()?;

    let token = Token::try_from(config.bot_token.clone())
        .map_err(|e| miette::miette!("Invalid token: {}", e))?;

    // 2. Initialize AppState (connects to DB)
    let app_state = AppState::new(config).await.into_diagnostic()?;

    let commands = vec![character(), chat(), pin(), emoji(), model()];

    let framework = Framework::builder()
        .options(FrameworkOptions {
            commands,
            prefix_options: PrefixFrameworkOptions {
                prefix: Some("+".into()),
                ..Default::default()
            },
            ..Default::default()
        })
        .build();

    ClientBuilder::new(token, INTENTS)
        .framework(Box::new(framework))
        .data(Arc::new(app_state))
        .event_handler(Arc::new(EventHandler))
        .await
        .into_diagnostic()?
        .start()
        .await
        .into_diagnostic()
}
