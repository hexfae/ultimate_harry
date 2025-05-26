use miette::Report;
use poise::{
    BoxFuture, FrameworkContext,
    serenity_prelude::{FullEvent, MessageId},
};

use interaction::interaction_create;
use message::message;
use ready::ready;
use ultimate_character::Character;
use ultimate_database::DB;
use ultimate_history::History;

mod interaction;
mod message;
mod ready;

// poise event_handler requires it to be borrowed
#[allow(clippy::trivially_copy_pass_by_ref)]
#[must_use]
pub fn event_handler<'a>(
    ctx: FrameworkContext<'a, (), Report>,
    event: &'a FullEvent,
) -> BoxFuture<'a, Result<(), Report>> {
    let ctx = ctx.serenity_context;
    match event {
        FullEvent::Ready { .. } => Box::pin(ready(ctx)),
        FullEvent::Message { new_message } => Box::pin(message(ctx, new_message)),
        FullEvent::InteractionCreate { interaction } => {
            Box::pin(interaction_create(ctx, interaction))
        }
        _ => Box::pin(async { Ok(()) }),
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
