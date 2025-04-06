use poise::{
    CreateReply, ReplyHandle,
    serenity_prelude::{
        ComponentInteractionCollector, CreateActionRow, CreateButton, CreateInteractionResponse,
        CreateInteractionResponseMessage,
    },
};
use ultimate_character::{CHARACTERS, Character};
use ultimate_harry::{
    Context, DeferEphemeralOrBroadcast, FIVE_SECONDS, Result, TEN_MINUTES,
    delete_invoking_message_if_prefix,
};
use ultimate_phrases::{NO_CHARACTER_PHRASES, sample};
use ultimate_statistics::STATISTICS;

#[poise::command(slash_command, prefix_command, rename = "visa")]
pub async fn view(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: Option<String>,
) -> Result<()> {
    ctx.defer_ephemeral_or_broadcast().await?;
    match name {
        Some(name) => sorted_by_similarity(ctx, name).await?,
        None => sorted_by_usage(ctx).await?,
    }
    Ok(())
}

async fn sorted_by_similarity(ctx: Context<'_>, name: impl AsRef<str>) -> Result<()> {
    let characters = CHARACTERS.read().get_all_sorted_by_similarity(name);

    if characters.is_empty() {
        let msg = ctx.say(sample(NO_CHARACTER_PHRASES)).await?;
        std::thread::sleep(FIVE_SECONDS);
        msg.delete(ctx).await?;
        delete_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    }

    let characters_and_footer_text = characters
        .into_iter()
        .map(|(similarity, character)| {
            let similarity = format!("{:.0}", similarity * 100.0);
            let conversations_had = character.conversations_had();
            let footer_text =
                format!("{conversations_had} konversationer | {similarity}% namnlikhet");
            (character, footer_text)
        })
        .collect();
    display_pagination(ctx, characters_and_footer_text).await?;
    Ok(())
}

async fn sorted_by_usage(ctx: Context<'_>) -> Result<()> {
    let characters = CHARACTERS.read().get_all_sorted_by_usage();

    if characters.is_empty() {
        let msg = ctx.say(sample(NO_CHARACTER_PHRASES)).await?;
        std::thread::sleep(FIVE_SECONDS);
        msg.delete(ctx).await?;
        delete_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    }

    let characters_and_footer_text = characters
        .into_iter()
        .map(|character| {
            let footer_text = format!("{} konversationer", character.conversations_had());
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
    STATISTICS.write().character_viewed_by(ctx.author());

    // index into 0 is safe because we checked `is_empty()` earlier
    let msg = send_initial_embed(ctx, characters_and_footer_text[0].clone()).await?;

    while let Some(interaction) = ComponentInteractionCollector::new(ctx)
        .custom_ids(vec![prev.clone(), next.clone()])
        .timeout(TEN_MINUTES)
        .await
    {
        let interaction_id = interaction.data.custom_id.clone();
        if interaction_id == prev {
            current_page = current_page.checked_sub(1).unwrap_or(pages - 1);
        } else if interaction_id == next {
            current_page += 1;
            if current_page >= pages {
                current_page = 0;
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
            .await?;
    }

    delete_invoking_message_if_prefix(ctx).await?;
    msg.delete(ctx).await?;

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
        .map_err(Into::into)
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
