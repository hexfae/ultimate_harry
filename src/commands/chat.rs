use crate::{
    Context,
    commands::autocomplete,
    constants::{NO_CHARACTER_PHRASES, sample},
    models::{character::Character, history::History},
};
use miette::{Diagnostic, Report};
use poise::serenity_prelude::MessageId;
use snafu::{ResultExt, Snafu};

#[derive(Debug, Snafu, Diagnostic)]
enum ChatError {
    #[snafu(display("Kunde inte skjuta upp svaret: {source}"))]
    #[diagnostic(help("Försök igen om en liten stund"), code(commands::chat::defer))]
    Defer { source: serenity::Error },
    #[snafu(display("Kunde inte skicka meddelandet: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(commands::chat::send_message)
    )]
    SendMessage { source: serenity::Error },
    #[snafu(display("Kunde inte hämta meddelandet: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att meddelandet fortfarande finns"),
        code(commands::chat::retrieve_message)
    )]
    RetrieveMessage { source: serenity::Error },
}

#[poise::command(slash_command, rename = "prata")]
pub async fn chat(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: Option<String>,
) -> Result<(), Report> {
    ctx.defer_or_broadcast().await.context(DeferSnafu)?;

    let db = &ctx.data().db;
    let stats = &ctx.data().stats;

    let characters: Vec<Character> = if let Some(name) = name {
        db.characters_by_similarity(name).await?
    } else {
        db.random_characters().await?
    };

    let Some(character) = characters.first() else {
        let response = sample(NO_CHARACTER_PHRASES);
        ctx.say(response).await.context(SendMessageSnafu)?;
        return Ok(());
    };

    let id = MessageId::new(1);
    let mut history = History::from((character, id, ctx.author().id));
    history.has_finished(true);

    let msg = ctx
        .send(history.to_response(character, id, &ctx.data().db).await)
        .await
        .context(SendMessageSnafu)?;
    let actual_id = msg.message().await.context(RetrieveMessageSnafu)?.id;

    history.set_id(actual_id);
    db.insert_history(history).await?;

    stats.conversation_started_by(ctx.author().id);

    Ok(())
}
