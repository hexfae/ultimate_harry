use crate::{
    Context, DeferSnafu, EditMessageSnafu, FIVE_SECONDS, Result, RetrieveMessageSnafu,
    SendMessageSnafu,
    commands::autocomplete,
    traits::{DeleteSelfAndInvokingMessageIfPrefix, SayWith},
};
use miette::Report;
use poise::serenity_prelude::MessageId;
use snafu::ResultExt;
use tokio::time::sleep;
use ultimate_database::DB;
use ultimate_history::History;
use ultimate_phrases::{NO_CHARACTER_PHRASES, sample};
use ultimate_statistics::STATISTICS;

#[poise::command(slash_command, prefix_command, rename = "prata")]
pub async fn chat(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: Option<String>,
) -> Result<(), Report> {
    ctx.defer_or_broadcast().await.context(DeferSnafu)?;

    let characters = if let Some(name) = name {
        DB.characters_by_similarity(name).await
    } else {
        DB.random_characters().await
    }?;

    let Some(character) = characters.first() else {
        let response = sample(NO_CHARACTER_PHRASES);
        let msg = ctx.say_with(response).await?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    };

    // create a temporary message id, this will be edited to
    // be the actual message id after like half a second
    let id = MessageId::new(1);
    let mut history = History::from((character, id, ctx.author().id));
    let msg = ctx
        .send(history.to_response(character, id))
        .await
        .context(SendMessageSnafu)?;
    let id = msg.message().await.context(RetrieveMessageSnafu)?.id;

    // TODO: instead of a standard "response," create some sort of specific pagination
    // for creating a new conversation where you paginate between characters
    let edit = history.to_response(character, id);
    msg.edit(ctx, edit).await.context(EditMessageSnafu)?;
    history.set_id(id);
    DB.insert_history(history).await?;

    STATISTICS.conversation_started_by(ctx.author());

    Ok(())
}
