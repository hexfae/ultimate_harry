use std::{
    thread::sleep,
    time::{Duration, Instant},
};

use crate::{ONE_MINUTE, SendMessageSnafu};
use miette::{IntoDiagnostic, Report};
use nanorand::{Rng, tls_rng};
use poise::{
    BoxFuture, FrameworkContext,
    serenity_prelude::{ActivityData, ActivityType, Context, EditMessage, FullEvent, Message},
};
use snafu::ResultExt;
use tracing::info;
use ultimate_character::{Character, HasFinished};
use ultimate_config::CONFIG;
use ultimate_database::DB;
use ultimate_history::History;
use ultimate_message::Message as UltimateMessage;
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

async fn message_received(event: &FullEvent, ctx: &Context) -> Result<(), Report> {
    let Some((message, mut history, character)) = event.message_history_character().await? else {
        return Ok(());
    };

    history.push(history.chosen_choice());
    history.push((message.clone(), DB.name(&message.author).await?));

    let response = character
        .to_response(
            message.id,
            (0, 0),
            (0, 0, None::<u64>),
            Some("…"),
            Some(Duration::from_secs(0)),
            HasFinished::No,
        )
        .to_prefix((&message).into());

    let mut response_message = message
        .channel_id
        .send_message(ctx, response)
        .await
        .context(SendMessageSnafu)?;

    let now = Instant::now();
    let requester = Requester::new(
        character
            .model_settings()
            .unwrap_or_else(|| CONFIG.read().model_settings()),
    );

    let (content, response) = requester.request(history.clone()).await.into_diagnostic()?;

    let edit = character
        .to_response(
            message.id,
            (0, 0),
            (0, 0, None::<u64>),
            Some(&content),
            Some(now.elapsed()),
            HasFinished::Yes,
        )
        .to_prefix_edit(EditMessage::new());

    response_message.edit(ctx, edit).await.into_diagnostic()?;

    history.set_choices(UltimateMessage::try_from((
        character.clone(),
        response.clone(),
    ))?);

    history.set_id(response_message);

    DB.insert_history(history).await?;
    Ok(())
}

// event_handler has to return a future
#[allow(clippy::unused_async)]
async fn set_uptime_as_activity(ctx: &Context) -> Result<(), Report> {
    info!("ready");
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

trait MessageFromEvent {
    fn message(&self) -> Option<Message>;
}

trait ReplyFromMessage {
    fn get_reply(&self) -> Option<Message>;
}

trait HistoryFromReply {
    async fn reply_history(&self) -> Result<Option<History>, Report>;
}

trait MessageHistoryCharacter {
    async fn message_history_character(
        &self,
    ) -> Result<Option<(Message, History, Character)>, Report>;
}

impl MessageFromEvent for FullEvent {
    fn message(&self) -> Option<Message> {
        match self {
            Self::Message { new_message } => Some(new_message.clone()),
            _ => None,
        }
    }
}

impl ReplyFromMessage for Message {
    fn get_reply(&self) -> Option<Message> {
        self.referenced_message.as_deref().cloned()
    }
}

impl HistoryFromReply for Message {
    async fn reply_history(&self) -> Result<Option<History>, Report> {
        let Some(reply) = self.get_reply() else {
            return Ok(None);
        };
        Ok(DB.history(reply.id).await?)
    }
}

impl MessageHistoryCharacter for FullEvent {
    async fn message_history_character(
        &self,
    ) -> Result<Option<(Message, History, Character)>, Report> {
        let Some(message) = self.message() else {
            return Ok(None);
        };
        let Some(history) = message.reply_history().await? else {
            return Ok(None);
        };
        let Some(character) = DB.character(history.character()).await? else {
            return Ok(None);
        };
        Ok(Some((message, history, character)))
    }
}
