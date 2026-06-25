//! The ready event handler for when the bot connects to Discord.

use crate::{AppResult, commands::partitioned_commands, error::RegisterCommandSnafu};
use core::time::Duration;
use nanorand::{Rng as _, WyRand};
use poise::{
    samples::{register_globally, register_in_guild},
    serenity_prelude::{ActivityData, ActivityType, Context, small_fixed_array::FixedString},
};
use serenity::all::Ready;
use snafu::ResultExt as _;
use std::time::Instant;
use tokio::time::sleep;
use tracing::info;

/// Handles the ready event when the bot connects to Discord.
///
/// This function registers all slash commands in each guild and starts
/// a background task that updates the bot's activity status.
#[expect(
    clippy::integer_division,
    reason = "the loss of precision is desired, we divide by constant, non-zero numbers"
)]
pub async fn ready(ctx: &Context, data_about_bot: &Ready) -> AppResult {
    info!("ready");
    let ctx_clone = ctx.clone();
    let (global_commands, guild_commands) = partitioned_commands();
    register_globally(&ctx_clone.http, &global_commands)
        .await
        .context(RegisterCommandSnafu)?;
    for guild in &data_about_bot.guilds {
        register_in_guild(&ctx_clone.http, &guild_commands, guild.id)
            .await
            .context(RegisterCommandSnafu)?;
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

            ctx_clone.set_activity(Some(ActivityData {
                name: FixedString::from_static_trunc("Heroes of the Storm"),
                kind: ActivityType::Playing,
                state: Some(FixedString::from_string_trunc(format!(
                    "{kills}-{assists}-{deaths} ({hours:02}:{minutes:02})",
                ))),
                url: None,
            }));
            sleep(Duration::from_mins(1)).await;
            // small per-minute chance to tick each stat up (1% kills, 2%
            // assists, 4% deaths), so harry slowly accrues a mediocre
            // scoreline over a long match
            if rng.generate_range(0..=99_u32) < 1 {
                kills = kills.saturating_add(1);
            }
            if rng.generate_range(0..=99_u32) < 2 {
                assists = assists.saturating_add(1);
            }
            if rng.generate_range(0..=99_u32) < 4 {
                deaths = deaths.saturating_add(1);
            }
        }
    });
    Ok(())
}
