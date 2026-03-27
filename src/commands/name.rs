//! The bot's Discord slash command for setting users' names.

use crate::{Context, database::DatabaseError, traits::SayEphemeral as _};
use miette::{Diagnostic, Result};
use poise::serenity_prelude::UserId;
use snafu::{ResultExt as _, Snafu};

/// The bot's Discord slash command for setting a user's name.
#[poise::command(slash_command)]
pub async fn name(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Namnet att använda"]
    name: String,
    #[rename = "användare"]
    #[description = "Användaren vars namn ska ändras"]
    user: Option<UserId>,
) -> Result<()> {
    let user_id = user.unwrap_or_else(|| ctx.author().id);
    ctx.data()
        .db
        .upsert_user_name(user_id, name)
        .await
        .context(SaveNameSnafu)?;
    ctx.say_ephemeral("done").await.context(SendMessageSnafu)?;
    Ok(())
}

/// All errors that can happen when setting a user's name.
#[derive(Debug, Snafu, Diagnostic)]
enum ChangeNameError {
    /// Sending a message failed.
    #[snafu(display("Kunde inte skicka meddelande: {source}"))]
    #[diagnostic(
        help("Det kan hända att meddelandet är för långt eller att kanalen är full"),
        code(commands::name::send_message)
    )]
    SendMessage {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Saving the name to the database failed.
    #[snafu(display("Kunde inte spara namnet: {source}"))]
    #[diagnostic(transparent, code(commands::name::save_name))]
    SaveName {
        /// The source of the error.
        source: DatabaseError,
    },
}
