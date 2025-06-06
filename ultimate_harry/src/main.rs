use std::borrow::Cow;

use commands::{character, chat, emoji, pin};
use miette::{IntoDiagnostic, Report};
use poise::{
    Framework, FrameworkOptions, PrefixFrameworkOptions,
    serenity_prelude::{ClientBuilder, GatewayIntents},
};
use ultimate_config::CONFIG;
use ultimate_database::DB;
use ultimate_harry::{
    Result,
    commands::{self},
    event_handler,
};

const INTENTS: GatewayIntents =
    GatewayIntents::non_privileged().union(GatewayIntents::MESSAGE_CONTENT);

#[tokio::main]
async fn main() -> Result<(), Report> {
    tracing_subscriber::fmt::init();
    DB.connect().await?;
    let (token, guild_ids) = {
        let config = CONFIG.read();
        (config.bot_token(), config.guild_ids())
    };
    let commands = vec![character(), chat(), pin(), emoji()];
    let framework: Framework<(), Report> = Framework::builder()
        .options(FrameworkOptions {
            commands,
            event_handler,
            prefix_options: PrefixFrameworkOptions {
                prefix: Some(Cow::Borrowed("+")),
                ..Default::default()
            },
            ..Default::default()
        })
        .setup(|ctx, _, framework| {
            Box::pin(async move {
                for id in guild_ids {
                    poise::builtins::register_in_guild(ctx, &framework.options().commands, id)
                        .await
                        .into_diagnostic()?;
                }
                Ok(())
            })
        })
        .build();
    ClientBuilder::new(token, INTENTS)
        .framework(framework)
        .await
        .into_diagnostic()?
        .start()
        .await
        .into_diagnostic()
}
