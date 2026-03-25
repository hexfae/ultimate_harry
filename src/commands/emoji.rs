use crate::Context;
use miette::{Diagnostic, Report};
use poise::serenity_prelude::ReactionType;
use snafu::{ResultExt, Snafu};

#[derive(Debug, Snafu, Diagnostic)]
enum SetEmojiError {
    #[snafu(display("Kunde inte skjuta upp svaret: {source}"))]
    #[diagnostic(
        help("Detta kan bero på nätverksproblem eller Discord-tjänsten är otillgänglig"),
        code(commands::emoji::set_emoji::defer)
    )]
    Defer { source: serenity::Error },
    #[snafu(display("Kunde inte spara emojit: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att emojit är giltigt"),
        code(commands::emoji::set_emoji::save_emoji)
    )]
    SaveEmoji { source: crate::db::DatabaseError },
    #[snafu(display("Kunde inte skicka meddelandet: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(commands::emoji::set_emoji::send_message)
    )]
    SendMessage { source: serenity::Error },
}

#[poise::command(slash_command)]
pub async fn emoji(ctx: Context<'_>, emoji: String) -> Result<(), Report> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;
    let Ok(emoji) = ReactionType::try_from(emoji) else {
        ctx.say("Det där var ingen emoji… (eller så gick någonting fel!)")
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };
    ctx.data()
        .db
        .upsert_user_emoji(ctx.author(), emoji.clone())
        .await
        .context(SaveEmojiSnafu)?;
    ctx.say(emoji.to_string()).await.context(SendMessageSnafu)?;
    Ok(())
}
