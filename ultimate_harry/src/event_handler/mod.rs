use miette::Report;
use poise::serenity_prelude::{Context, FullEvent, MessageId, async_trait};

use interaction::interaction_create;
use message::message;
use ready::ready;
use tracing::warn;
use ultimate_character::Character;
use ultimate_database::DB;
use ultimate_history::History;

mod interaction;
mod message;
mod ready;

pub struct EventHandler;

#[async_trait]
impl poise::serenity_prelude::EventHandler for EventHandler {
    async fn dispatch(&self, ctx: &Context, event: &FullEvent) {
        if let Err(why) = match event {
            FullEvent::Ready { .. } => ready(ctx).await,
            FullEvent::Message { new_message, .. } => message(ctx, new_message).await,
            FullEvent::InteractionCreate { interaction, .. } => {
                interaction_create(ctx, interaction).await
            }
            _ => Ok(()),
        } {
            warn!("error in event handler: {why}");
        }
    }
}

pub trait HistoryCharacter {
    async fn history_character(&self) -> Result<Option<(History, Character)>, Report>;
}

impl HistoryCharacter for MessageId {
    async fn history_character(&self) -> Result<Option<(History, Character)>, Report> {
        let Some(history) = DB.history(self).await? else {
            return Ok(None);
        };
        let Some(character) = DB.character(history.character()).await? else {
            return Ok(None);
        };
        Ok(Some((history, character)))
    }
}
