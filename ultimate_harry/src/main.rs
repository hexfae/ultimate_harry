use std::{borrow::Cow, sync::Arc};

use commands::{character, chat, emoji, pin};
use miette::{Diagnostic, IntoDiagnostic, Report};
use poise::{
    Framework, FrameworkOptions, PrefixFrameworkOptions,
    serenity_prelude::{ClientBuilder, GatewayIntents, Token},
};
use snafu::{ResultExt, Snafu};
use ultimate_config::CONFIG;
use ultimate_database::DB;
use ultimate_harry::{
    EventHandler, Result,
    commands::{self},
};

const INTENTS: GatewayIntents =
    GatewayIntents::non_privileged().union(GatewayIntents::MESSAGE_CONTENT);

#[derive(Debug, Snafu, Diagnostic)]
pub struct InvalidTokenError {
    source: poise::serenity_prelude::TokenError,
}

#[tokio::main]
async fn main() -> Result<(), Report> {
    // our dependencies pull in rustls with both the ring and aws-lc-rs features, so install aws-lc-rs specifically
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install aws-lc-rs rustls provider");
    tracing_subscriber::fmt::init();
    DB.connect().await?;
    let token = Token::try_from(CONFIG.read().bot_token()).context(InvalidTokenSnafu)?;
    let commands = vec![character(), chat(), pin(), emoji()];
    let framework: Framework<(), Report> = Framework::builder()
        .options(FrameworkOptions {
            commands,
            prefix_options: PrefixFrameworkOptions {
                prefix: Some(Cow::Borrowed("+")),
                ..Default::default()
            },
            ..Default::default()
        })
        .build();
    ClientBuilder::new(token, INTENTS)
        .framework(Box::new(framework))
        .event_handler(Arc::new(EventHandler))
        .await
        .into_diagnostic()?
        .start()
        .await
        .into_diagnostic()
}
