use crate::Context;
use miette::{Diagnostic, Report};
use serenity::all::GuildChannel;
use snafu::{ResultExt as _, Snafu};

#[derive(Debug, Snafu, Diagnostic)]
#[diagnostic(code(commands::pin_channel::pin_channel))]
enum PinChannelError {
    #[snafu(display("Kunde inte skjuta upp svaret: {source}"))]
    #[diagnostic(help("Försök igen om en liten stund"))]
    Defer { source: serenity::Error },
    #[snafu(display("Kunde inte skicka meddelandet: {source}"))]
    #[diagnostic(help("Det kan hända att meddelandet är för långt eller att kanalen är full"))]
    SendMessage { source: serenity::Error },
}

#[poise::command(slash_command)]
pub async fn pin_channel(ctx: Context<'_>, channel: GuildChannel) -> Result<(), Report> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;
    ctx.data().db.upsert_pin_channel(channel.id).await?;
    ctx.say("done").await.context(SendMessageSnafu)?;
    Ok(())
}
