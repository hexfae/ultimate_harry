use crate::commands::{character, chat, emoji, model, name, pin_channel};
use miette::{Diagnostic, Report};
use nanorand::{Rng as _, WyRand};
use poise::serenity_prelude::{
    ActivityData, ActivityType, Context, small_fixed_array::FixedString,
};
use serenity::all::Ready;
use snafu::{ResultExt as _, Snafu};
use std::time::{Duration, Instant};
use tokio::time::sleep;
use tracing::info;

#[derive(Debug, Snafu, Diagnostic)]
pub struct RegisterCommandInGuildError {
    source: poise::serenity_prelude::Error,
}

pub async fn ready(ctx: &Context, data_about_bot: &Ready) -> Result<(), Report> {
    info!("ready");
    let ctx = ctx.clone();
    let commands = vec![character(), chat(), emoji(), model(), pin_channel(), name()];
    for guild in &data_about_bot.guilds {
        poise::builtins::register_in_guild(&ctx.http, &commands, guild.id)
            .await
            .context(RegisterCommandInGuildSnafu)?;
    }
    tokio::spawn(async move {
        let start = Instant::now();
        let mut rng = WyRand::new();
        let mut kills: u32 = 0;
        let mut assists: u32 = 0;
        let mut deaths: u32 = 0;
        loop {
            let elapsed = start.elapsed();
            let hours = elapsed.as_secs() / 3600;
            let minutes = (elapsed.as_secs() % 3600) / 60;

            ctx.set_activity(Some(ActivityData {
                name: FixedString::from_static_trunc("Heroes of the Storm"),
                kind: ActivityType::Playing,
                state: Some(FixedString::from_string_trunc(format!(
                    "{kills}-{assists}-{deaths} ({hours:02}:{minutes:02})",
                ))),
                url: None,
            }));
            sleep(Duration::from_mins(1)).await;
            let new_kills: u32 = rng.generate_range(0..=1010);
            let new_assists: u32 = rng.generate_range(0..=1020);
            let new_deaths: u32 = rng.generate_range(0..=1040);
            kills += new_kills / 1000;
            assists += new_assists / 1000;
            deaths += new_deaths / 1000;
        }
    });
    Ok(())
}
