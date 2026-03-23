use std::borrow::Cow;

use crate::{
    Context, DeferSnafu, Result, RetrieveMessageSnafu, SendMessageSnafu,
    constants::{NEXT, NO_CHARACTER_PHRASES, PREVIOUS, sample},
    models::character::{Character, CharacterPages},
    traits::SayWith,
};
use miette::Report;
use poise::{
    CreateReply,
    serenity_prelude::{
        ButtonStyle, CreateActionRow, CreateButton, CreateComponent, MessageId, ReactionType,
        small_fixed_array::FixedString,
    },
};
use snafu::ResultExt;

#[poise::command(slash_command, rename = "visa")]
pub async fn view(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: Option<String>,
) -> Result<(), Report> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;
    let characters: Vec<Character> = match name {
        Some(name) => ctx.data().db.characters_by_similarity(name).await,
        None => ctx.data().db.characters_by_usage().await,
    }?;

    let Some(first) = characters.first() else {
        let response = sample(NO_CHARACTER_PHRASES);
        ctx.say_with(response).await?;
        return Ok(());
    };

    let footer_text = {
        let pages = characters.len();
        let conversations_had = first.conversations_had();
        let similarity = first.similarity();
        format!("1/{pages} | {conversations_had} konversationer{similarity}")
    };

    let id = send_message(ctx, first, footer_text, characters.len()).await?;

    ctx.data().stats.character_viewed_by(ctx.author());

    let character_pages = CharacterPages::builder()
        .id(id)
        .characters(&characters)
        .build();

    ctx.data()
        .db
        .insert_character_pages(character_pages)
        .await?;

    Ok(())
}

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
    ctx.send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)?
        .message()
        .await
        .context(RetrieveMessageSnafu)
        .map(|m| m.id)
}

pub fn create_buttons(id: u64, total_pages: usize) -> Cow<'static, [CreateComponent<'static>]> {
    let prev = format!("{id}prev");
    let next = format!("{id}next");
    let disabled = total_pages < 2;
    Cow::Owned(vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        Cow::Owned(vec![
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
        ]),
    ))])
}
