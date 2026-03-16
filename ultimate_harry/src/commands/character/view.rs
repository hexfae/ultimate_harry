use std::borrow::Cow;

use crate::{
    Context, DeferEphemeralOrBroadcast, FIVE_SECONDS, Result, RetrieveMessageSnafu,
    SendMessageSnafu,
    traits::{DeleteSelfAndInvokingMessageIfPrefix, SayWith},
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
use tokio::time::sleep;
use ultimate_character::{Character, CharacterPages};
use ultimate_database::DB;
use ultimate_phrases::{NO_CHARACTER_PHRASES, sample};
use ultimate_statistics::STATISTICS;

const PREV: &str = "⬅️";
const NEXT: &str = "➡️";

#[poise::command(slash_command, rename = "visa")]
pub async fn view(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: Option<String>,
) -> Result<(), Report> {
    ctx.defer_ephemeral_or_broadcast().await?;
    let characters = match name {
        Some(name) => DB.characters_by_similarity(name).await,
        None => DB.characters_by_usage().await,
    }?;

    let Some(first) = characters.first() else {
        let response = sample(NO_CHARACTER_PHRASES);
        let msg = ctx.say_with(response).await?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    };

    let footer_text = {
        let pages = characters.len();
        let conversations_had = first.conversations_had();
        let similarity = first.similarity();
        format!("1/{pages} | {conversations_had} konversationer{similarity}")
    };

    let id = send_message(ctx, first, footer_text, characters.len()).await?;

    STATISTICS.character_viewed_by(ctx.author());

    let character_pages = CharacterPages::builder()
        .id(id)
        .characters(&characters)
        .build();

    DB.insert_character_pages(character_pages).await?;

    Ok(())
}

async fn send_message<'a>(
    ctx: Context<'a>,
    character: &'a Character,
    footer_text: String,
    total_pages: usize,
) -> Result<MessageId> {
    let id = ctx.id();
    let embed = character.clone().into_embed_with_footer_text(footer_text);
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
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(PREV))),
            CreateButton::new(next)
                .disabled(disabled)
                .style(ButtonStyle::Secondary)
                .emoji(ReactionType::Unicode(FixedString::from_static_trunc(NEXT))),
        ]),
    ))])
}

// impl TryFrom<&ComponentInteraction> for InteractionType {
//     type Error = Error;

//     fn try_from(input: &ComponentInteraction) -> Result<Self, Self::Error> {
//         match input.data.custom_id.as_str() {
//             i if i.ends_with("prev") => Ok(Self::Prev),
//             i if i.ends_with("next") => Ok(Self::Next),
//             i => Err(Error::UnknownInteraction {
//                 found: i.to_string(),
//             }),
//         }
//     }
// }

// async fn send_initial_embed(
//     ctx: Context<'_>,
//     character: &Character,
//     footer_text: String,
// ) -> Result<(), Report> {
// let id = ctx.id();
// let prev = format!("{id}prev");
// let next = format!("{id}next");
// let mut current_page: usize = 0;

// index into 0 is safe because we checked `is_empty()` earlier
// let msg = send_initial_embed(ctx, characters_and_footer_text[0].clone()).await?;
// while let Some(interaction) = ComponentInteractionCollector::new(ctx)
//     .custom_ids(vec![prev.clone(), next.clone()])
//     .timeout(TEN_MINUTES)
//     .await
// {
//     let interaction_type = InteractionType::try_from(&interaction)?;
//     match interaction_type {
//         InteractionType::Prev => {
//             current_page = (current_page + pages - 1) % pages;
//         }
//         InteractionType::Next => {
//             current_page = (current_page + 1) % pages;
//         }
//     }

//     let (character, footer_text) = characters_and_footer_text[current_page].clone();
//     let embed = character.to_embed_with_footer_text(footer_text);

//     interaction
//         .create_response(
//             ctx,
//             CreateInteractionResponse::UpdateMessage(
//                 CreateInteractionResponseMessage::new().embed(embed),
//             ),
//         )
//         .await
//         .context(SendResponseSnafu)?;
// }

// msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
// Ok(())
// }
