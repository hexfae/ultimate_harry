//! Ultimate Harry, Discord bot for AI character chats.

extern crate alloc;

mod app_state;
mod codec;
mod commands;
mod constants;
mod database;
mod error;
mod events;
mod llm;
mod models;
mod phrases;
mod traits;

use crate::{
    app_state::AppState,
    commands::{character, chat, emoji, model, name, pin_channel},
    error::AppError,
    events::{EventHandler, on_error},
};
use alloc::sync::Arc;
use core::result::Result as CoreResult;
use miette::{Diagnostic, Result};
use poise::{
    Framework, FrameworkOptions,
    serenity_prelude::{ClientBuilder, GatewayIntents, Token, TokenError as SerenityTokenError},
};
use rustls::crypto::aws_lc_rs;
use snafu::{ResultExt as _, Snafu};
use std::{env::var, fs::read_to_string, io};
use tracing::warn;
use tracing_subscriber::fmt::init as tracing_init;

/// The default `Result` type used throughout most of the bot.
pub type AppResult<T = (), E = AppError> = CoreResult<T, E>;
/// The default `Context` type used in most slash commands.
pub type Context<'a> = poise::Context<'a, AppState, AppError>;
/// The default `ApplicationContext` type used in certain slash commands.
pub type ApplicationContext<'a> = poise::ApplicationContext<'a, AppState, AppError>;

/// The Discord intents needed for Ultimate Harry to function.
///
/// Non-privileged and message content.
const INTENTS: GatewayIntents =
    GatewayIntents::non_privileged().union(GatewayIntents::MESSAGE_CONTENT);

#[tokio::main]
async fn main() -> Result<()> {
    aws_lc_rs::default_provider()
        .install_default()
        .map_err(|_why| RustlsError)?;
    tracing_init();

    let token_path = var("TOKEN_FILE").unwrap_or_else(|_| "token".to_owned());
    let token_string = read_to_string(token_path).context(TokenPathSnafu)?;
    let token = Token::try_from(token_string).context(InvalidSnafu)?;

    let app_state = AppState::new().await?;

    let commands = vec![character(), chat(), emoji(), model(), pin_channel(), name()];

    let framework = Framework::builder()
        .options(FrameworkOptions {
            commands,
            on_error: |error| {
                Box::pin(async {
                    if let Err(why) = on_error(error).await {
                        warn!("error in event handler: {why}");
                    }
                })
            },
            ..Default::default()
        })
        .build();

    Ok(ClientBuilder::new(token, INTENTS)
        .framework(Box::new(framework))
        .data(Arc::new(app_state))
        .event_handler(Arc::new(EventHandler))
        .await
        .context(BuildSnafu)?
        .start()
        .await
        .context(StartSnafu)?)
}

/// Installing `aws-lc-rs` as the default `rustls` provider failed.
#[derive(Debug, Snafu, Diagnostic)]
#[snafu(display("Failed to install `aws-lc-rs` as the default Rustls provider"))]
struct RustlsError;

/// All errors that can happen when reading the bot's Discord token.
#[derive(Debug, Snafu, Diagnostic)]
enum TokenError {
    /// The path to the token file was invalid.
    #[snafu(display("Could not read the token file"))]
    #[diagnostic(code(main::token_path))]
    TokenPath {
        /// The source of the error.
        source: io::Error,
    },
    /// The token itself was invalid.
    #[snafu(display("Invalid Discord token: {source}"))]
    #[diagnostic(code(main::invalid_token))]
    Invalid {
        /// The source of the error.
        source: SerenityTokenError,
    },
}

/// All errors that can happen when building the serenity client.
#[derive(Debug, Snafu, Diagnostic)]
enum ClientError {
    /// Building the client failed.
    Build {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Starting the client failed.
    Start {
        /// The source of the error.
        source: serenity::Error,
    },
}
