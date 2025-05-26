use crate::{
    Context, DeferSnafu, EditMessageSnafu, FIVE_SECONDS, Result, RetrieveMessageSnafu,
    SendMessageSnafu,
    traits::{DeleteSelfAndInvokingMessageIfPrefix, SayWith},
};
use miette::Report;
use poise::{ReplyHandle, serenity_prelude::UserId};
use snafu::ResultExt;
use tokio::time::sleep;
use ultimate_character::{Character, HasFinished};
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
    name: Option<String>,
) -> Result<(), Report> {
    ctx.defer_or_broadcast().await.context(DeferSnafu)?;

    let characters = if let Some(name) = name {
        DB.characters_by_similarity(name).await
    } else {
        DB.random_characters().await
    }?;

    if characters.is_empty() {
        let response = sample(NO_CHARACTER_PHRASES);
        let msg = ctx.say_with(response).await?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    }

    STATISTICS.conversation_started_by(ctx.author());

    let msg = send_initial_message(ctx, characters.clone()).await?;

    edit_initial_message(ctx, characters, msg).await?;

    Ok(())
}

async fn send_initial_message(
    ctx: Context<'_>,
    characters: Vec<Character>,
) -> Result<ReplyHandle<'_>, Report> {
    let character = characters[0].clone();

    Ok(ctx
        .send(character.to_response(
            ctx.id(),
            (0, characters.len()),
            (0, 0, None::<UserId>),
            None::<&str>,
            None,
            HasFinished::No,
        ))
        .await
        .context(SendMessageSnafu)?)
}

async fn edit_initial_message(
    ctx: Context<'_>,
    characters: Vec<Character>,
    msg: ReplyHandle<'_>,
) -> Result<(), Report> {
    let character = characters[0].clone();

    let msg_id = msg.message().await.context(RetrieveMessageSnafu)?.id;
    let edit = character.to_response(
        msg_id,
        (0, characters.len()),
        (0, 0, None::<UserId>),
        None::<&str>,
        None,
        HasFinished::Yes,
    );
    msg.edit(ctx, edit).await.context(EditMessageSnafu)?;
    let history = History::from((character, msg_id, ctx.author().id));
    DB.insert_history(history).await?;
    Ok(())
}
