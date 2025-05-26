use miette::Report;
use poise::serenity_prelude::{Context, EditMessage, Message};
use snafu::ResultExt;
use std::time::Instant;
use ultimate_character::Character;
use ultimate_config::CONFIG;
use ultimate_database::DB;
use ultimate_history::History;
use ultimate_message::Message as UltimateMessage;
use ultimate_requester::Requester;

use crate::{EditMessageSnafu, SendMessageSnafu};

pub async fn message(ctx: &Context, new_message: &Message) -> Result<(), Report> {
    let Some((mut history, character)) = new_message.history_character().await? else {
        return Ok(());
    };

    history.push(history.chosen_choice_message().to_owned());
    history.push((new_message.clone(), DB.name(&new_message.author).await?));
    history.reset_choices();

    let placeholder = history
        .to_placeholder(&character)
        .to_prefix(new_message.into());

    let mut response_message = new_message
        .channel_id
        .send_message(ctx, placeholder)
        .await
        .context(SendMessageSnafu)?;

    let requester = Requester::new(
        character
            .model_settings()
            .unwrap_or_else(|| CONFIG.read().model_settings()),
    );

    let now = Instant::now();
    let response = requester.request(history.clone()).await?;

    history.set_choices(UltimateMessage::try_from((
        character.clone(),
        response.clone(),
        now.elapsed(),
    ))?);

    let edit = history
        .to_response(&character, response_message.id)
        .to_prefix_edit(EditMessage::new());

    response_message
        .edit(ctx, edit)
        .await
        .context(EditMessageSnafu)?;

    history.set_id(response_message);

    DB.insert_history(history).await?;
    Ok(())
}

trait HistoryFromReply {
    async fn reply_history(&self) -> Result<Option<History>, Report>;
}

trait HistoryCharacter {
    async fn history_character(&self) -> Result<Option<(History, Character)>, Report>;
}

impl HistoryFromReply for Message {
    async fn reply_history(&self) -> Result<Option<History>, Report> {
        let Some(reply) = self.referenced_message.as_deref() else {
            return Ok(None);
        };
        Ok(DB.history(reply.id).await?)
    }
}

impl HistoryCharacter for &Message {
    async fn history_character(&self) -> Result<Option<(History, Character)>, Report> {
        let Some(history) = self.reply_history().await? else {
            return Ok(None);
        };
        let Some(character) = DB.character(history.character()).await? else {
            return Ok(None);
        };
        Ok(Some((history, character)))
    }
}
