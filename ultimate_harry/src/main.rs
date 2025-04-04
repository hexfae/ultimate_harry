mod commands;

use commands::character::character;
use poise::{
    Framework, FrameworkOptions,
    serenity_prelude::{ClientBuilder, GatewayIntents, GuildId},
};
use ultimate_harry::{Error, Result};

const TOKEN: &str = "REDACTED_DISCORD_TOKEN";
const INTENTS: GatewayIntents = GatewayIntents::non_privileged();
const GUILD_ID: GuildId = GuildId::new(1113998071194456195);

#[tokio::main]
async fn main() -> Result<()> {
    let commands = vec![character()];
    let framework: Framework<(), Error> = Framework::builder()
        .options(FrameworkOptions {
            commands,
            ..Default::default()
        })
        .setup(|ctx, _, framework| {
            Box::pin(async move {
                poise::builtins::register_in_guild(ctx, &framework.options().commands, GUILD_ID)
                    .await?;
                Ok(())
            })
        })
        .build();
    Ok(ClientBuilder::new(TOKEN, INTENTS)
        .framework(framework)
        .await?
        .start()
        .await?)
}
