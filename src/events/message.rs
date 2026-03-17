use miette::Report;
use poise::serenity_prelude::{Context, EditMessage, Message};
use snafu::ResultExt;
use std::time::Instant;

use crate::{
    EditMessageSnafu, ReactSnafu, SendMessageSnafu,
    db::Database,
    models::{character::Character, history::History, message::Message as UltimateMessage},
    requester::Requester,
};

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

    let requester = Requester::new(
        character
            .model_settings()
            .unwrap_or(db.model_settings().await),
    );

    let now = Instant::now();
    let response = requester.request(history.clone()).await?;

    history.set_choices(UltimateMessage::try_from((
        character.clone(),
        response.clone(),
        now.elapsed(),
    ))?);

    let edit = history
        .to_response(&character, response_message.id, db)
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

trait HistoryCharacter {
    async fn history_character(
        &self,
        db: &Database,
    ) -> Result<Option<(History, Character)>, Report>;
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
