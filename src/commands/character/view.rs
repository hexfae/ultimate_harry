//! The bot's Discord slash command for viewing characters.

use crate::{
    Context, Result,
    constants::{NEXT, PREVIOUS},
    models::character::{Character, ViewCharacterPages},
    phrases::no_character,
    traits::SayEphemeral as _,
};
use alloc::borrow::Cow;
use miette::Diagnostic;
use poise::{
    CreateReply,
    serenity_prelude::{
        ButtonStyle, CreateActionRow, CreateButton, CreateComponent, MessageId, ReactionType,
        small_fixed_array::FixedString,
    },
};
use snafu::{ResultExt as _, Snafu};

#[poise::command(slash_command, rename = "visa")]
pub async fn view(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: Option<String>,
) -> Result<()> {
    let characters: Vec<Character> = match name {
        Some(character_name) => ctx.data().db.characters_by_similarity(character_name).await,
        None => ctx.data().db.characters_by_usage().await,
    }?;

    let Some(first) = characters.first() else {
        ctx.say_ephemeral(no_character())
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };

    let footer_text = {
        let pages = characters.len();
        let conversations_had = first.conversations_had();
        let similarity = first.similarity();
        format!("1/{pages} | {conversations_had} konversationer{similarity}")
    };

    let id = send_message(ctx, first, footer_text, characters.len()).await?;

    let character_pages = ViewCharacterPages::builder()
        .id(id)
        .characters(&characters)
        .build();

    ctx.data()
        .db
        .insert_character_pages(character_pages)
        .await?;

    Ok(())
}

/// Send the first message, containing the embed of a character and buttons for viewing others.
async fn send_message<'a>(
    ctx: Context<'a>,
    character: &'a Character,
    footer_text: String,
    total_pages: usize,
) -> Result<MessageId> {
    let id = ctx.id();
    let embed = character
        .clone()
        .into_embed_with_footer_text(footer_text, &ctx.data().db)
        .await;
    let buttons = create_buttons(id, total_pages);
    let msg = ctx
        .send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)?;
    Ok(msg.message().await.context(RetrieveMessageSnafu)?.id)
}

/// Returns a component action row containing 2 buttons for viewing others.
#[must_use]
pub fn create_buttons(id: u64, total_pages: usize) -> Cow<'static, [CreateComponent<'static>]> {
    let prev = format!("{id}prev");
    let next = format!("{id}next");
    let disabled = total_pages < 2;
    vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        vec![
            CreateButton::new(prev)
                .disabled(disabled)
                .style(ButtonStyle::Secondary)
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(
                    PREVIOUS,
                ))),
            CreateButton::new(next)
                .disabled(disabled)
                .style(ButtonStyle::Secondary)
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(NEXT))),
        ]
        .into(),
    ))]
    .into()
}

/// All errors that can happen when viewing characters.
#[derive(Debug, Snafu, Diagnostic)]
enum ViewCharacterError {
    /// Sending a message failed.
    #[snafu(display("Kunde inte skicka meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen om en stund"),
        code(commands::character::view::send_message)
    )]
    SendMessage {
        /// The source of the error.
        source: serenity::Error,
    },
    /// Retrieving a message failed.
    #[snafu(display("Kunde inte hämta meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen eller kontrollera att meddelandet fortfarande finns."),
        code(commands::character::view::retrieve_message)
    )]
    RetrieveMessage {
        /// The source of the error.
        source: serenity::Error,
    },
}
