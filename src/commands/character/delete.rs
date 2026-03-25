use std::{borrow::Cow, time::Duration};

use crate::{
    Context, Result,
    commands::character::interaction::InteractionType,
    constants::{
        ASK_DELETE_PHRASES, CANCEL, CANCELLED_PHRASES, DELETE, DELETED_PHRASES, NEXT,
        NO_CHARACTER_PHRASES, PREVIOUS, sample,
    },
    models::character::Character,
    traits::RespondToWith as _,
};
use miette::{Diagnostic, Report};
use poise::{
    CreateReply, ReplyHandle,
    serenity_prelude::{
        ButtonStyle, ComponentInteraction, ComponentInteractionCollector, CreateActionRow,
        CreateButton, CreateComponent, CreateInteractionResponse, CreateInteractionResponseMessage,
        small_fixed_array::{FixedArray, FixedString},
    },
};
use serenity::all::ReactionType;
use snafu::{ResultExt as _, Snafu};
use surrealdb::RecordId;
use tokio::time::sleep;

#[derive(Debug, Snafu, Diagnostic)]
pub enum DeleteCharacterError {
    #[snafu(display("Kunde inte skjuta upp svaret: {source}"))]
    #[diagnostic(
        help("Detta kan bero på nätverksproblem eller Discord-tjänsten är otillgänglig"),
        code(commands::character::delete::defer)
    )]
    Defer { source: serenity::Error },
    #[snafu(display("Kunde inte skicka meddelande: {source}"))]
    #[diagnostic(
        help("Försök igen om en stund"),
        code(commands::character::delete::send_message)
    )]
    SendMessage { source: serenity::Error },
    #[snafu(display("Kunde inte skicka interaktionssvar: {source}"))]
    #[diagnostic(
        help("Detta kan bero på att interaktionen har gått ut"),
        code(commands::character::delete::send_response)
    )]
    SendResponse { source: serenity::Error },
    #[snafu(display("Kunde inte ta bort svaret: {source}"))]
    #[diagnostic(
        help("Försök igen eller starta om interaktionen"),
        code(commands::character::delete::delete_response)
    )]
    DeleteResponse { source: serenity::Error },
}

#[poise::command(slash_command, rename = "döda")]
pub async fn delete(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<(), Report> {
    ctx.defer_ephemeral().await.context(DeferSnafu)?;

    let characters: Vec<Character> = ctx.data().db.characters_by_similarity(&name).await?;

    if characters.is_empty() {
        let response = sample(NO_CHARACTER_PHRASES);
        ctx.say(response).await.context(SendMessageSnafu)?;
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
    send_initial_embed(ctx, characters_and_footer_text[0].clone()).await?;

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

                let Some(response) = ask_for_confirmation(ctx, interaction).await? else {
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
        let embed = character
            .into_embed_with_footer_text(footer_text, &ctx.data().db)
            .await;

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

    Ok(())
}

#[must_use]
async fn create_collector(
    ctx: Context<'_>,
    custom_ids: FixedArray<FixedString>,
) -> Option<ComponentInteraction> {
    ComponentInteractionCollector::new(ctx.serenity_context())
        .custom_ids(custom_ids)
        .await
}

async fn send_initial_embed(
    ctx: Context<'_>,
    (character, footer_text): (Character, String),
) -> Result<ReplyHandle<'_>> {
    let id = ctx.id();
    let embed = character
        .into_embed_with_footer_text(footer_text, &ctx.data().db)
        .await;
    let buttons = create_buttons(id);
    Ok(ctx
        .send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)?)
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
                .emoji(DELETE.parse::<ReactionType>().expect("valid emoji"))
                .style(ButtonStyle::Secondary),
            CreateButton::new(cancel)
                .emoji(CANCEL.parse::<ReactionType>().expect("valid emoji"))
                .style(ButtonStyle::Secondary),
            CreateButton::new(prev)
                .emoji(PREVIOUS.parse::<ReactionType>().expect("valid emoji"))
                .style(ButtonStyle::Secondary),
            CreateButton::new(next)
                .emoji(NEXT.parse::<ReactionType>().expect("valid emoji"))
                .style(ButtonStyle::Secondary),
        ]),
    ))])
}

async fn delete_cancelled(ctx: Context<'_>, interaction: ComponentInteraction) -> Result<()> {
    let response = sample(CANCELLED_PHRASES);
    ctx.respond_to_with(&interaction, response)
        .await
        .context(SendMessageSnafu)?;
    sleep(Duration::from_secs(5)).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
}

async fn ask_for_confirmation(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> Result<Option<ComponentInteraction>> {
    let response = sample(ASK_DELETE_PHRASES);
    let reply = Character::to_confirm_interaction_response(ctx.id(), response);
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
        .await)
}

async fn delete_confirmed(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    id: &RecordId,
) -> Result<()> {
    ctx.data().db.delete_character(id, ctx.author()).await?;
    ctx.data().stats.character_deleted_by(ctx.author());

    let response = sample(DELETED_PHRASES);
    ctx.respond_to_with(&interaction, response)
        .await
        .context(SendMessageSnafu)?;
    sleep(Duration::from_secs(5)).await;
    interaction
        .delete_response(ctx.http())
        .await
        .context(DeleteResponseSnafu)?;
    Ok(())
}
