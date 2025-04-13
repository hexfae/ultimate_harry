use commands::{character, chat};
use miette::{IntoDiagnostic, Report};
use nanorand::{Rng, tls_rng};
use poise::{
    BoxFuture, Framework, FrameworkContext, FrameworkOptions, PrefixFrameworkOptions,
    serenity_prelude::{
        ActivityData, ActivityType, ClientBuilder, Context, FullEvent, GatewayIntents, GuildId,
    },
};
use std::{thread::sleep, time::Instant};
use ultimate_harry::{ONE_MINUTE, Result, commands};

const TOKEN: &str = "REDACTED_DISCORD_TOKEN";
const INTENTS: GatewayIntents =
    GatewayIntents::non_privileged().union(GatewayIntents::MESSAGE_CONTENT);
#[allow(clippy::unreadable_literal)] // doesn't make sense for a guild id
const GUILD_ID: GuildId = GuildId::new(1113998071194456195);

#[tokio::main]
async fn main() -> Result<(), Report> {
    let commands = vec![character(), chat()];
    let framework: Framework<(), Report> = Framework::builder()
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

// poise event_handler requires it to be borrowed
#[allow(clippy::trivially_copy_pass_by_ref)]
fn event_handler<'a>(
    ctx: &'a Context,
    event: &'a FullEvent,
    _framework: FrameworkContext<'_, (), Report>,
    _: &(),
) -> BoxFuture<'a, Result<(), Report>> {
    if let FullEvent::Ready { .. } = event {
        println!("ready");
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
                    sleep(ONE_MINUTE);
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
