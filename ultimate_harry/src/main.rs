use std::borrow::Cow;

use commands::{character, chat, pin};
use miette::{IntoDiagnostic, Report};
use poise::{
    Framework, FrameworkOptions, PrefixFrameworkOptions,
    serenity_prelude::{ClientBuilder, GatewayIntents, GuildId},
};
use ultimate_database::DB;
use ultimate_harry::{
    Result,
    commands::{self},
    event_handler,
};

const TOKEN: &str = "REDACTED_DISCORD_TOKEN";
const INTENTS: GatewayIntents =
    GatewayIntents::non_privileged().union(GatewayIntents::MESSAGE_CONTENT);
#[allow(clippy::unreadable_literal)] // doesn't make sense for a guild id
const GUILD_ID: GuildId = GuildId::new(1113998071194456195);

#[tokio::main]
async fn main() -> Result<(), Report> {
    tracing_subscriber::fmt::init();
    DB.connect().await?;
    let commands = vec![character(), chat(), pin()];
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
                poise::builtins::register_in_guild(ctx, &framework.options().commands, GUILD_ID)
                    .await
                    .into_diagnostic()?;
                Ok(())
            })
        })
        .build();
    ClientBuilder::new(TOKEN, INTENTS)
        .framework(framework)
        .await
        .into_diagnostic()?
        .start()
        .await
        .into_diagnostic()
}
