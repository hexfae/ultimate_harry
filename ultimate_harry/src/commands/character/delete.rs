use std::borrow::Cow;

use crate::{
    Context, DeferEphemeralOrBroadcast, DeleteInvokingMessageIfPrefix, Error, FIVE_SECONDS,
    ONE_MINUTE, RespondToWith, Result, SendMessageSnafu, SendResponseSnafu, TEN_MINUTES,
    traits::{DeleteResponse, DeleteSelfAndInvokingMessageIfPrefix, SayWith},
};
use miette::Report;
use poise::{
    CreateReply, ReplyHandle,
    serenity_prelude::{
        ButtonStyle, ComponentInteraction, ComponentInteractionCollector, CreateActionRow,
        CreateButton, CreateComponent, CreateInteractionResponse, CreateInteractionResponseMessage,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use snafu::ResultExt;
use surrealdb::RecordId;
use tokio::time::sleep;
use ultimate_character::Character;
use ultimate_database::DB;
use ultimate_phrases::{
    ASK_DELETE_PHRASES, CANCELLED_PHRASES, DELETED_PHRASES, NO_CHARACTER_PHRASES, sample,
};
use ultimate_statistics::STATISTICS;

enum InteractionType {
    Prev,
    Next,
    Confirm,
    Cancel,
}

#[poise::command(slash_command, prefix_command, rename = "döda")]
pub async fn delete(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<(), Report> {
    ctx.defer_ephemeral_or_broadcast().await?;

    let characters = DB.characters_by_similarity(&name).await?;

    if characters.is_empty() {
        let response = sample(NO_CHARACTER_PHRASES);
        let msg = ctx.say_with(response).await?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    }

    let pages = characters.len();
    let characters_and_footer_text = characters
        .into_iter()
        .enumerate()
        .map(|(index, character)| {
            let index = index + 1;
            let similarity = character.similarity();
            let conversations_had = character.conversations_had();
            let footer_text =
                format!("{index}/{pages} | {conversations_had} konversationer{similarity}");
            (character, footer_text)
        })
        .collect();

    display_pagination(ctx, characters_and_footer_text).await?;
    Ok(())
}

async fn display_pagination(
    ctx: Context<'_>,
    characters_and_footer_text: Vec<(Character, String)>,
) -> Result<(), Report> {
    let id = ctx.id();
    let custom_ids = FixedArray::from_vec_trunc(
        ["confirm", "cancel", "prev", "next"]
            .map(|s| FixedString::from_string_trunc(format!("{id}{s}")))
            .to_vec(),
    );

    let mut current_page: usize = 0;
    let pages = characters_and_footer_text.len();

    // index into 0 is safe because we checked `is_empty()` earlier
    let msg = send_initial_embed(ctx, characters_and_footer_text[0].clone()).await?;

    while let Some(interaction) = create_collector(ctx, custom_ids.clone()).await {
        let interaction_type = InteractionType::try_from(&interaction)?;
        match interaction_type {
            InteractionType::Prev => {
                current_page = (current_page + pages - 1) % pages;
            }
            InteractionType::Next => {
                current_page = (current_page + 1) % pages;
            }
            InteractionType::Confirm => {
                let (character, _footer_text) = characters_and_footer_text[current_page].clone();

                let id = ctx.id();
                let confirm_id = format!("{id}confirm");

                let Some(response) =
                    ask_for_confirmation(ctx, interaction, character.clone()).await?
                else {
                    continue;
                };
                let returned_id = response.data.custom_id.clone();

                if returned_id == confirm_id {
                    delete_confirmed(ctx, response, character.id()).await?;
                } else {
                    delete_cancelled(ctx, response).await?;
                }
                return Ok(());
            }
            InteractionType::Cancel => {
                delete_cancelled(ctx, interaction).await?;
                return Ok(());
            }
        }

        let (character, footer_text) = characters_and_footer_text[current_page].clone();
        let embed = character.into_embed_with_footer_text(footer_text);

        interaction
            .create_response(
                ctx.http(),
                CreateInteractionResponse::UpdateMessage(
                    CreateInteractionResponseMessage::new().embed(embed),
                ),
            )
            .await
            .context(SendResponseSnafu)?;
    }

    msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
    Ok(())
}

#[must_use]
async fn create_collector(
    ctx: Context<'_>,
    custom_ids: FixedArray<FixedString>,
) -> Option<ComponentInteraction> {
    ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(custom_ids)
        .timeout(TEN_MINUTES)
        .await
}

async fn send_initial_embed(
    ctx: Context<'_>,
    (character, footer_text): (Character, String),
) -> Result<ReplyHandle<'_>> {
    let id = ctx.id();
    let embed = character.into_embed_with_footer_text(footer_text);
    let buttons = create_buttons(id);
    ctx.send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)
}

#[must_use]
pub fn create_buttons(id: u64) -> Cow<'static, [CreateComponent<'static>]> {
    let confirm = format!("{id}confirm");
    let cancel = format!("{id}cancel");
    let prev = format!("{id}prev");
    let next = format!("{id}next");
    Cow::Owned(vec![CreateComponent::ActionRow(CreateActionRow::Buttons(
        Cow::Owned(vec![
            CreateButton::new(confirm)
                .emoji('🗑')
                .style(ButtonStyle::Secondary),
            CreateButton::new(cancel)
                .emoji('❌')
                .style(ButtonStyle::Secondary),
            CreateButton::new(prev)
                .emoji('◀')
                .style(ButtonStyle::Secondary),
            CreateButton::new(next)
                .emoji('▶')
                .style(ButtonStyle::Secondary),
        ]),
    ))])
}

async fn delete_cancelled(ctx: Context<'_>, interaction: ComponentInteraction) -> Result<()> {
    let response = sample(CANCELLED_PHRASES);
    ctx.respond_to_with(&interaction, response).await?;
    sleep(FIVE_SECONDS).await;
    ctx.delete_response(interaction).await?;
    ctx.delete_invoking_message_if_prefix().await?;
    Ok(())
}

async fn ask_for_confirmation(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    character: Character,
) -> Result<Option<ComponentInteraction>> {
    let response = sample(ASK_DELETE_PHRASES);
    let reply = character.to_confirm_interaction_response(ctx.id(), response);
    interaction
        .create_response(ctx.http(), reply)
        .await
        .context(SendResponseSnafu)?;

    let id = ctx.id();
    let confirm_id = format!("{id}confirm");
    let cancel_id = format!("{id}cancel");

    Ok(ComponentInteractionCollector::new(ctx.serenity_context())
        .author_id(ctx.author().id)
        .custom_ids(FixedArray::from_vec_trunc(vec![
            FixedString::from_string_trunc(confirm_id.clone()),
            FixedString::from_string_trunc(cancel_id.clone()),
        ]))
        .timeout(ONE_MINUTE)
        .await)
}

async fn delete_confirmed(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    id: &RecordId,
) -> Result<(), Report> {
    DB.delete_character(id, ctx.author()).await?;
    STATISTICS.character_deleted_by(ctx.author());

    let response = sample(DELETED_PHRASES);
    ctx.respond_to_with(&interaction, response).await?;
    sleep(FIVE_SECONDS).await;
    ctx.delete_response(interaction).await?;
    ctx.delete_invoking_message_if_prefix().await?;
    Ok(())
}

impl TryFrom<&ComponentInteraction> for InteractionType {
    type Error = Error;

    fn try_from(input: &ComponentInteraction) -> Result<Self, Self::Error> {
        match input.data.custom_id.as_str() {
            i if i.ends_with("prev") => Ok(Self::Prev),
            i if i.ends_with("next") => Ok(Self::Next),
            i if i.ends_with("confirm") => Ok(Self::Confirm),
            i if i.ends_with("cancel") => Ok(Self::Cancel),
            i => Err(Error::UnknownInteraction {
                found: i.to_string(),
            }),
        }
    }
}
