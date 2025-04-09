mod commands;

use std::time::Instant;

use commands::{character, chat};
use nanorand::{Rng, tls_rng};
use poise::{
    BoxFuture, Framework, FrameworkContext, FrameworkOptions, PrefixFrameworkOptions,
    serenity_prelude::{
        ActivityData, ActivityType, ClientBuilder, Context, FullEvent, GatewayIntents, GuildId,
    },
};
use ultimate_harry::{Error, ONE_MINUTE, Result};

const TOKEN: &str = "REDACTED_DISCORD_TOKEN";
const INTENTS: GatewayIntents =
    GatewayIntents::non_privileged().union(GatewayIntents::MESSAGE_CONTENT);
const GUILD_ID: GuildId = GuildId::new(1113998071194456195);

#[tokio::main]
async fn main() -> Result<()> {
    let commands = vec![character(), chat()];
    let framework: Framework<(), Error> = Framework::builder()
        .options(FrameworkOptions {
            commands,
            event_handler,
            prefix_options: PrefixFrameworkOptions {
                prefix: Some("+".to_owned()),
                ..Default::default()
            },
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

fn event_handler<'a>(
    ctx: &'a Context,
    event: &'a FullEvent,
    _framework: FrameworkContext<'_, (), Error>,
    _: &(),
) -> BoxFuture<'a, Result<(), Error>> {
    if let FullEvent::Ready { .. } = event {
        Box::pin(async move {
            let ctx = ctx.clone();
            tokio::spawn(async move {
                let start = Instant::now();
                let mut rng = tls_rng();
                let mut kills: u32 = 0;
                let mut assists: u32 = 0;
                let mut deaths: u32 = 0;
                loop {
                    let elapsed = start.elapsed();
                    let hours = elapsed.as_secs() / 3600;
                    let minutes = elapsed.as_secs() / 60;

                    ctx.set_activity(Some(ActivityData {
                        name: "Heroes of the Storm".to_owned(),
                        kind: ActivityType::Playing,
                        state: Some(format!(
                            "{kills}-{assists}-{deaths} ({hours:02}:{minutes:02})",
                        )),
                        url: None,
                    }));
                    std::thread::sleep(ONE_MINUTE);
                    let new_kills: u32 = rng.generate_range(0..=1010);
                    let new_assists: u32 = rng.generate_range(0..=1020);
                    let new_deaths: u32 = rng.generate_range(0..=1040);
                    kills += new_kills / 1000;
                    assists += new_assists / 1000;
                    deaths += new_deaths / 1000;
                }
            });
            Ok(())
        })
    } else {
        Box::pin(async { Ok(()) })
    }
}
