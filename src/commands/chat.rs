use crate::{
    Context,
    commands::autocomplete,
    constants::{NO_CHARACTER_PHRASES, sample},
    models::{character::Character, history::History},
    traits::{DeleteSelfAndInvokingMessageIfPrefix, SayWith},
};
use miette::{IntoDiagnostic, Report};
use poise::serenity_prelude::MessageId;
use std::time::Duration;
use tokio::time::sleep;

#[poise::command(slash_command, rename = "prata")]
pub async fn chat(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: Option<String>,
) -> Result<(), Report> {
    ctx.defer_or_broadcast().await.into_diagnostic()?;

    let db = &ctx.data().db;
    let stats = &ctx.data().stats;

    let characters: Vec<Character> = if let Some(name) = name {
        db.characters_by_similarity(name).await?
    } else {
        db.random_characters().await?
    };

    let Some(character) = characters.first() else {
        let response = sample(NO_CHARACTER_PHRASES);
        let msg = ctx.say_with(response).await.into_diagnostic()?;
        sleep(Duration::from_secs(5)).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx)
            .await
            .into_diagnostic()?;
        return Ok(());
    };

    let id = MessageId::new(1);
    let mut history = History::from((character, id, ctx.author().id));

    let msg = ctx
        .send(
            history
                .to_response(character, id, &ctx.data().db, true)
                .await,
        )
        .await
        .into_diagnostic()?;
    let actual_id = msg.message().await.into_diagnostic()?.id;

    history.set_id(actual_id);
    db.insert_history(history).await.into_diagnostic()?;

    stats.conversation_started_by(ctx.author().id);

    Ok(())
}
