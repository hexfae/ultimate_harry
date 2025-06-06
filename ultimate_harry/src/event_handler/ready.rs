use crate::ONE_MINUTE;
use miette::Report;
use nanorand::{Rng, WyRand};
use poise::serenity_prelude::{ActivityData, ActivityType, Context};
use std::time::Instant;
use tokio::time::sleep;
use tracing::info;

// event_handler has to return a future
#[allow(clippy::unused_async)]
pub async fn ready(ctx: &Context) -> Result<(), Report> {
    info!("ready");
    let ctx = ctx.clone();
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
                name: "Heroes of the Storm".to_owned(),
                kind: ActivityType::Playing,
                state: Some(format!(
                    "{kills}-{assists}-{deaths} ({hours:02}:{minutes:02})",
                )),
                url: None,
            }));
            sleep(ONE_MINUTE).await;
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
