use std::{thread::sleep, time::Instant};

use crate::ONE_MINUTE;
use miette::{IntoDiagnostic, Report};
use nanorand::{Rng, tls_rng};
use poise::{
    BoxFuture, FrameworkContext,
    serenity_prelude::{ActivityData, ActivityType, Context, EditMessage, FullEvent, Message},
};
use ultimate_character::{CHARACTERS, Character};
use ultimate_config::CONFIG;
use ultimate_history::HISTORIES;
use ultimate_requester::Requester;

// poise event_handler requires it to be borrowed
#[allow(clippy::trivially_copy_pass_by_ref)]
#[must_use]
pub fn event_handler<'a>(
    ctx: &'a Context,
    event: &'a FullEvent,
    _framework: FrameworkContext<'_, (), Report>,
    _: &'a (),
) -> BoxFuture<'a, Result<(), Report>> {
    if let FullEvent::Ready { .. } = event {
        Box::pin(set_uptime_as_activity(ctx))
    } else if let FullEvent::Message { .. } = event {
        Box::pin(message_received(event, ctx))
    } else {
        Box::pin(async { Ok(()) })
    }
}

trait MessageFromEvent {
    fn message(&self) -> Option<&Message>;
}

impl MessageFromEvent for FullEvent {
    fn message(&self) -> Option<&Message> {
        match self {
            Self::Message { new_message } => Some(new_message),
            _ => None,
        }
    }
}

trait ReplyFromMessage {
    fn get_reply(&self) -> Option<&Message>;
}

impl ReplyFromMessage for Message {
    fn get_reply(&self) -> Option<&Message> {
        self.referenced_message.as_deref()
    }
}

async fn create_initial_message(
    ctx: &Context,
    message: &Message,
    character: Character,
) -> Result<Message, Report> {
    message
        .channel_id
        .send_message(ctx, character.to_create_message(message.id))
        .await
        .into_diagnostic()
}

async fn message_received(event: &FullEvent, ctx: &Context) -> Result<(), Report> {
    let Some(message) = event.message() else {
        return Ok(());
    };
    let Some(reply) = message.get_reply() else {
        return Ok(());
    };
    let Some(mut history) = HISTORIES.read().get(reply.id) else {
        return Ok(());
    };
    let Some(character) = CHARACTERS.read().get_by_id(history.character()) else {
        return Ok(());
    };

    history.push(message.to_owned());

    let mut response_message = create_initial_message(ctx, message, character.clone()).await?;

    let requester = Requester::new(
        character
            .model_settings()
            .unwrap_or_else(|| CONFIG.read().model_settings()),
    );

    let (content, response) = requester.request(history.clone()).await.into_diagnostic()?;

    response_message
        .edit(ctx, EditMessage::new().content(content))
        .await
        .into_diagnostic()?;

    history.push(ultimate_message::Message::try_from((character, response)).into_diagnostic()?);

    history.set_id(message);

    HISTORIES.write().insert(history);

    Ok(())
}

// event_handler has to return a future
#[allow(clippy::unused_async)]
async fn set_uptime_as_activity(ctx: &Context) -> Result<(), Report> {
    println!("ready");
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
}
