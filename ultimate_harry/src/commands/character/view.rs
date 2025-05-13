use crate::{
    Context, DeferEphemeralOrBroadcast, Error, FIVE_SECONDS, Result, SendMessageSnafu,
    SendResponseSnafu, TEN_MINUTES,
    traits::{DeleteSelfAndInvokingMessageIfPrefix, SayWith},
};
use miette::Report;
use poise::{
    CreateReply, ReplyHandle,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, CreateActionRow, CreateButton,
        CreateInteractionResponse, CreateInteractionResponseMessage,
    },
};
use snafu::ResultExt;
use tokio::time::sleep;
use ultimate_character::Character;
use ultimate_database::DB;
use ultimate_phrases::{NO_CHARACTER_PHRASES, sample};
use ultimate_statistics::STATISTICS;

enum InteractionType {
    Prev,
    Next,
}

#[poise::command(slash_command, prefix_command, rename = "visa")]
pub async fn view(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: Option<String>,
) -> Result<(), Report> {
    ctx.defer_ephemeral_or_broadcast().await?;
    match name {
        Some(name) => sorted_by_similarity(ctx, name).await?,
        None => sorted_by_usage(ctx).await?,
    }
    Ok(())
}

async fn sorted_by_similarity(ctx: Context<'_>, name: String) -> Result<(), Report> {
    let characters = DB.characters_by_similarity(name).await?;

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
            let footer_text = format!(
                "{index}/{pages} | {conversations_had} konversationer | {similarity}% namnlikhet"
            );
            (character, footer_text)
        })
        .collect();
    display_pagination(ctx, characters_and_footer_text).await?;
    Ok(())
}

async fn sorted_by_usage(ctx: Context<'_>) -> Result<(), Report> {
    let characters = DB.characters_by_usage().await?;

    if characters.is_empty() {
        let msg = ctx
            .say(sample(NO_CHARACTER_PHRASES))
            .await
            .context(SendMessageSnafu)?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    }

    let pages = characters.len();
    let characters_and_footer_text = characters
        .into_iter()
        .enumerate()
        .map(|(index, character)| {
            let footer_text = format!(
                "{index}/{pages} | {} konversationer",
                character.conversations_had()
            );
            (character, footer_text)
        })
        .collect();
    display_pagination(ctx, characters_and_footer_text).await?;
    Ok(())
}

async fn display_pagination(
    ctx: Context<'_>,
    characters_and_footer_text: Vec<(Character, String)>,
) -> Result<()> {
    let id = ctx.id();
    let prev = format!("{id}prev");
    let next = format!("{id}next");
    let mut current_page: usize = 0;
    let pages = characters_and_footer_text.len();
    STATISTICS.character_viewed_by(ctx.author());

    // index into 0 is safe because we checked `is_empty()` earlier
    let msg = send_initial_embed(ctx, characters_and_footer_text[0].clone()).await?;

    while let Some(interaction) = ComponentInteractionCollector::new(ctx)
        .custom_ids(vec![prev.clone(), next.clone()])
        .timeout(TEN_MINUTES)
        .await
    {
        let interaction_type = InteractionType::try_from(&interaction)?;
        match interaction_type {
            InteractionType::Prev => {
                current_page = (current_page + pages - 1) % pages;
            }
            InteractionType::Next => {
                current_page = (current_page + 1) % pages;
            }
        }

        let (character, footer_text) = characters_and_footer_text[current_page].clone();
        let embed = character.to_embed_with_footer_text(footer_text);

        interaction
            .create_response(
                ctx,
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

async fn send_initial_embed(
    ctx: Context<'_>,
    (character, footer_text): (Character, String),
) -> Result<ReplyHandle<'_>> {
    let id = ctx.id();
    let embed = character.to_embed_with_footer_text(footer_text);
    let buttons = create_buttons(id);
    ctx.send(CreateReply::default().embed(embed).components(buttons))
        .await
        .context(SendMessageSnafu)
}

pub fn create_buttons(id: u64) -> Vec<CreateActionRow> {
    let prev = format!("{id}prev");
    let next = format!("{id}next");
    let components = CreateActionRow::Buttons(vec![
        CreateButton::new(&prev).emoji('◀'),
        CreateButton::new(&next).emoji('▶'),
    ]);
    vec![components]
}

impl TryFrom<&ComponentInteraction> for InteractionType {
    type Error = Error;

    fn try_from(input: &ComponentInteraction) -> Result<Self, Self::Error> {
        match input.data.custom_id.as_str() {
            i if i.ends_with("prev") => Ok(Self::Prev),
            i if i.ends_with("next") => Ok(Self::Next),
            i => Err(Error::UnknownInteraction {
                found: i.to_string(),
            }),
        }
    }
}
