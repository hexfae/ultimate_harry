//! The bot's Discord slash command for setting users' emoji.

use crate::{Context, database::DatabaseError, traits::SayEphemeral as _};
use miette::{Diagnostic, Result};
use poise::serenity_prelude::ReactionType;
use snafu::{ResultExt as _, Snafu};

/// The bot's Discord slash command for setting a user's emoji.
#[poise::command(slash_command)]
pub async fn emoji(ctx: Context<'_>, emoji: String) -> Result<()> {
    let Ok(found_emoji) = ReactionType::try_from(emoji) else {
        ctx.say_ephemeral("Det där var ingen emoji… (eller så gick någonting fel!)")
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };
    ctx.data()
        .db
        .upsert_user_emoji(ctx.author(), found_emoji.clone())
        .await
        .context(SaveEmojiSnafu)?;
    ctx.say_ephemeral(found_emoji.to_string())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}

/// All errors that can happen when setting a user's emoji.
#[derive(Debug, Snafu, Diagnostic)]
enum SetEmojiError {
    /// Sending a message failed.
    #[snafu(display("Kunde inte skicka meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att kanalen är tillgänglig"),
        code(commands::emoji::send_message)
    )]
    SendMessage {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Saving the emoji to the database failed.
    #[snafu(display("Kunde inte spara emojin: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att emoji:n är giltig"),
        code(commands::emoji::save_emoji)
    )]
    SaveEmoji {
        /// The source of the error.
        source: DatabaseError,
    },
}
