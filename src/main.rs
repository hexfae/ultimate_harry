//! Ultimate Harry, Discord bot for AI character chats.

extern crate alloc;

mod app_state;
mod commands;
mod components;
mod constants;
mod database;
mod error;
mod error_display;
mod events;
mod llm;
mod models;
mod phrases;
mod traits;
mod tts;
mod util;
mod vision;

use crate::{
    app_state::AppState,
    commands::commands,
    error::AppError,
    events::{EventHandler, on_error},
    util::report_error,
};
use alloc::sync::Arc;
use core::{result::Result as CoreResult, time::Duration};
use miette::{Diagnostic, Result};
use poise::{
    Framework, FrameworkOptions,
    serenity_prelude::{ClientBuilder, GatewayIntents, Token, TokenError as SerenityTokenError},
};
use rustls::crypto::aws_lc_rs;
use snafu::{ResultExt as _, Snafu};
use std::{env::var, fs::read_to_string, io};
use tokio::{signal, time::timeout};
use tracing::{info, warn};
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

    let app_state = Arc::new(AppState::new().await?);

    let framework = Framework::builder()
        .options(FrameworkOptions {
            commands: commands(),
            on_error: |error| {
                Box::pin(async {
                    if let Err(why) = on_error(error).await {
                        warn!("error in event handler");
                        report_error(why);
                    }
                })
            },
            ..Default::default()
        })
        .build();

    let mut client = ClientBuilder::new(token, INTENTS)
        .framework(Box::new(framework))
        .data(Arc::clone(&app_state))
        .event_handler(Arc::new(EventHandler))
        .await
        .context(BuildSnafu)?;

    let shutdown = client.shard_manager.get_shutdown_trigger();
    drop(tokio::spawn(async move {
        wait_for_shutdown().await;
        info!("shutdown signal received, disconnecting from the gateway");
        if !shutdown() {
            warn!("the shard manager had already shut down");
        }
    }));

    client.start().await.context(StartSnafu)?;

    // the gateway has stopped, so no new replies will start; wait for the
    // in-flight ones to finish persisting their History before exiting.
    app_state.tasks.close();
    if timeout(SHUTDOWN_GRACE, app_state.tasks.wait()).await.is_err() {
        warn!("timed out waiting for in-flight replies to finish");
    }

    Ok(())
}

/// How long to wait for in-flight replies to finish after the gateway stops
/// before exiting anyway.
const SHUTDOWN_GRACE: Duration = Duration::from_mins(1);

/// Resolves once the process is asked to shut down, via SIGINT (ctrl-c) or,
/// on Unix, SIGTERM.
async fn wait_for_shutdown() {
    let interrupt = async {
        if let Err(why) = signal::ctrl_c().await {
            warn!("failed to listen for ctrl-c: {why}");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match signal::unix::signal(signal::unix::SignalKind::terminate()) {
            Ok(mut term) => drop(term.recv().await),
            Err(why) => warn!("failed to listen for SIGTERM: {why}"),
        }
    };

    #[cfg(not(unix))]
    let terminate = core::future::pending::<()>();

    tokio::select! {
        () = interrupt => {},
        () = terminate => {},
    }
}

/// Installing `aws-lc-rs` as the default `rustls` provider failed.
#[derive(Debug, Snafu, Diagnostic)]
#[snafu(display("Failed to install `aws-lc-rs` as the default Rustls provider"))]
#[diagnostic(code(main::rustls))]
struct RustlsError;

/// All errors that can happen when reading the bot's Discord token.
#[derive(Debug, Snafu, Diagnostic)]
enum TokenError {
    /// The path to the token file was invalid.
    #[snafu(display("Could not read the token file"))]
    #[diagnostic(
        help("Make sure the token file exists and is readable, or set TOKEN_FILE"),
        code(main::token_path)
    )]
    TokenPath {
        /// The source of the error.
        source: io::Error,
    },
    /// The token itself was invalid.
    #[snafu(display("Invalid Discord token: {source}"))]
    #[diagnostic(
        help("Check that the file contains a valid Discord bot token"),
        code(main::invalid_token)
    )]
    Invalid {
        /// The source of the error.
        source: SerenityTokenError,
    },
}

/// All errors that can happen when building the serenity client.
#[derive(Debug, Snafu, Diagnostic)]
enum ClientError {
    /// Building the client failed.
    #[snafu(display("Failed to build the Discord client: {source}"))]
    #[diagnostic(
        help("Check the Discord token and network connection"),
        code(main::build_client)
    )]
    Build {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Starting the client failed.
    #[snafu(display("Failed to start the Discord client: {source}"))]
    #[diagnostic(
        help("Check the network connection and that the token's intents are enabled"),
        code(main::start_client)
    )]
    Start {
        /// The source of the error.
        source: serenity::Error,
    },
}
