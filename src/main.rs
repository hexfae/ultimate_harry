use std::{env::var, fs::read_to_string, sync::Arc};

use miette::{Diagnostic, IntoDiagnostic, Report};
use poise::{
    Framework, FrameworkOptions, PrefixFrameworkOptions,
    serenity_prelude::{ClientBuilder, GatewayIntents, Token},
};
use snafu::{ResultExt, Snafu};
use ultimate_harry::{
    app_state::AppState,
    commands::{character, chat, emoji, model, name, pin_channel},
    events::EventHandler,
};

const INTENTS: GatewayIntents =
    GatewayIntents::non_privileged().union(GatewayIntents::MESSAGE_CONTENT);

#[derive(Debug, Snafu, Diagnostic)]
enum TokenError {
    TokenPath { source: std::io::Error },
    Invalid { source: serenity::all::TokenError },
}

#[tokio::main]
async fn main() -> Result<(), Report> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install aws-lc-rs rustls provider");
    tracing_subscriber::fmt::init();

    let token_path = var("TOKEN_FILE").unwrap_or_else(|_| "token".to_owned());
    let token_string = read_to_string(token_path).context(TokenPathSnafu)?;
    let token = Token::try_from(token_string).context(InvalidSnafu)?;

    let app_state = AppState::new().await?;

    let commands = vec![character(), chat(), emoji(), model(), pin_channel(), name()];

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
