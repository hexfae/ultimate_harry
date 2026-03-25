use crate::Context;
use miette::{Diagnostic, Report};
use poise::serenity_prelude::UserId;
use snafu::{ResultExt, Snafu};

#[derive(Debug, Snafu, Diagnostic)]
#[diagnostic(code(commands::name::change_name))]
enum ChangeNameError {
    #[snafu(display("Kunde inte skjuta upp svaret: {source}"))]
    #[diagnostic(help("Försök igen om en liten stund"))]
    Defer { source: serenity::Error },
    #[snafu(display("Kunde inte skicka meddelandet: {source}"))]
    #[diagnostic(help("Det kan hända att meddelandet är för långt eller att kanalen är full"))]
    SendMessage { source: serenity::Error },
}

#[poise::command(slash_command)]
pub async fn name(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Namnet att använda"]
    name: String,
    #[rename = "användare"]
    #[description = "Användaren vars namn ska ändras"]
    user: Option<UserId>,
) -> Result<(), Report> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;
    let user_id = user.unwrap_or_else(|| ctx.author().id);
    ctx.data().db.upsert_user_name(user_id, name).await?;
    ctx.say("done").await.context(SendMessageSnafu)?;
    Ok(())
}
