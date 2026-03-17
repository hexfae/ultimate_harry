use std::sync::Arc;

use serenity::{
    all::{Context, FullEvent, Interaction},
    async_trait,
};
use tracing::warn;

use crate::app_state::AppState;

pub mod interaction;
pub mod message;
pub mod ready;

pub struct EventHandler;

#[async_trait]
impl poise::serenity_prelude::EventHandler for EventHandler {
    async fn dispatch(&self, ctx: &Context, event: &FullEvent) {
        let data: Arc<AppState> = ctx.data();
        if let Err(why) = match event {
            FullEvent::Ready { data_about_bot, .. } => ready::ready(ctx, data_about_bot).await,
            FullEvent::Message { new_message, .. } => {
                message::message(ctx, new_message, &data.db).await
            }
            FullEvent::InteractionCreate { interaction, .. } => {
                if let Interaction::Component(component) = interaction {
                    interaction::interaction_create(ctx, component, &data.db).await
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        } {
            warn!("error in event handler: {why}");
        }
    }
}
