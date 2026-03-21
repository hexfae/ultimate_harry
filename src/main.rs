use std::{env::var, sync::Arc};

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
    EnvVarNotSet { source: std::env::VarError },
    Invalid { source: serenity::all::TokenError },
}

#[tokio::main]
async fn main() -> Result<(), Report> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install aws-lc-rs rustls provider");
    tracing_subscriber::fmt::init();

    let token = var("TOKEN_FILE").context(EnvVarNotSetSnafu)?;
    let token = Token::try_from(token).context(InvalidSnafu)?;

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
