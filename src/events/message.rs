use miette::{Diagnostic, Report};
use poise::serenity_prelude::{Context, EditMessage, Message};
use rig::{agent::MultiTurnStreamItem, streaming::StreamedAssistantContent};
use serenity::{all::MessageId, futures::StreamExt};
use snafu::{ResultExt, Snafu};
use std::time::{Duration, Instant};

use crate::{
    EditMessageSnafu, ReactSnafu, SendMessageSnafu,
    db::Database,
    llm::LlmManager,
    models::{character::Character, history::History},
};

#[derive(Debug, Snafu, Diagnostic)]
pub struct StreamingError {
    source: rig::agent::StreamingError,
}

pub async fn message(ctx: &Context, new_message: &Message, db: &Database) -> Result<(), Report> {
    {
        let emojis = db.user_emoji().await?;
        if new_message.mention_everyone() {
            for user_emoji in &emojis {
                new_message
                    .react(&ctx.http, user_emoji.emoji.clone())
                    .await
                    .context(ReactSnafu)?;
            }
        } else {
            if let Some(replied_to) = &new_message.referenced_message
                && let Some(user_emoji) = emojis.iter().find(|e| e.user_id == replied_to.author.id)
            {
                new_message
                    .react(&ctx.http, user_emoji.emoji.clone())
                    .await
                    .context(ReactSnafu)?;
            }
            for mention in &new_message.mentions {
                if let Some(user_emoji) = emojis.iter().find(|e| e.user_id == mention.id) {
                    new_message
                        .react(&ctx.http, user_emoji.emoji.clone())
                        .await
                        .context(ReactSnafu)?;
                }
            }
        }
    }
    let Some((mut history, character)): Option<(History, Character)> =
        new_message.history_character(db).await?
    else {
        return Ok(());
    };

    history.push(history.chosen_choice_message().to_owned());
    history.push((
        new_message.clone(),
        db.substitute_name(new_message.author.id).await,
    ));
    history.reset_choices();

    let placeholder = history
        .to_placeholder(&character)
        .to_prefix(new_message.into());

    let mut response_message = new_message
        .channel_id
        .send_message(&ctx.http, placeholder)
        .await
        .context(SendMessageSnafu)?;

    let requester = LlmManager::new(
        character
            .model_settings()
            .unwrap_or(db.model_settings().await),
    );

    let now = Instant::now();
    let mut time_since_last_edit = now;
    let mut response = requester.request_stream(&history, None).await;

    let mut total = String::new();

    while let Some(delta) = response.next().await {
        if let MultiTurnStreamItem::StreamAssistantItem(StreamedAssistantContent::Text(delta)) =
            delta.context(StreamingSnafu)?
        {
            if total.len() >= 3900 {
                break;
            }
            total += delta.text();
            if time_since_last_edit.elapsed() >= Duration::from_secs(1) {
                time_since_last_edit = Instant::now();
                history.set_choices((character.clone(), total.clone(), now.elapsed()));
                let edit = history
                    .to_response(&character, response_message.id, db, false)
                    .await
                    .to_prefix_edit(EditMessage::new());

                response_message
                    .edit(ctx, edit)
                    .await
                    .context(EditMessageSnafu)?;
            }
        }
    }

    history.set_choices((character.clone(), total, now.elapsed()));

    let edit = history
        .to_response(&character, response_message.id, db, true)
        .await
        .to_prefix_edit(EditMessage::new());

    response_message
        .edit(ctx, edit)
        .await
        .context(EditMessageSnafu)?;

    history.set_id(response_message);

    db.insert_history(history).await?;
    Ok(())
}

trait HistoryFromReply {
    async fn reply_history(&self, db: &Database) -> Result<Option<History>, Report>;
}

pub trait HistoryCharacter {
    fn history_character(
        &self,
        db: &Database,
    ) -> impl Future<Output = Result<Option<(History, Character)>, Report>>;
}

impl HistoryFromReply for Message {
    async fn reply_history(&self, db: &Database) -> Result<Option<History>, Report> {
        let Some(reply) = self.referenced_message.as_deref() else {
            return Ok(None);
        };
        Ok(db.history(reply.id).await?)
    }
}

impl HistoryCharacter for &Message {
    async fn history_character(
        &self,
        db: &Database,
    ) -> Result<Option<(History, Character)>, Report> {
        let Some(history) = self.reply_history(db).await? else {
            return Ok(None);
        };
        let Some(character) = db.character(history.character()).await? else {
            return Ok(None);
        };
        Ok(Some((history, character)))
    }
}
impl HistoryCharacter for MessageId {
    async fn history_character(
        &self,
        db: &Database,
    ) -> Result<Option<(History, Character)>, Report> {
        let Some(history) = db.history(self).await? else {
            return Ok(None);
        };
        let Some(character) = db.character(history.character()).await? else {
            return Ok(None);
        };
        Ok(Some((history, character)))
    }
}
