use crate::{
    Context, DeferSnafu, EditMessageSnafu, Error, FIVE_SECONDS, ONE_HOUR, Result,
    RetrieveMessageSnafu, SendMessageSnafu, ShowModalSnafu, TEN_MINUTES,
    traits::{AcknowledgeResponse, DeleteSelfAndInvokingMessageIfPrefix, SayWith},
};
use miette::Report;
use poise::{
    ReplyHandle, execute_modal_on_component_interaction,
    serenity_prelude::{ComponentInteraction, ComponentInteractionCollector, MessageId, UserId},
};
use snafu::ResultExt;
use tokio::time::sleep;
use ultimate_character::{CHARACTERS, Character, HasFinished};
use ultimate_history::{HISTORIES, History};
use ultimate_modals::EditMessageModal;
use ultimate_phrases::{NO_CHARACTER_PHRASES, sample};
use ultimate_statistics::STATISTICS;

enum InteractionType {
    Prev,
    Next,
    Edit,
    Undo,
    Redo,
}

#[poise::command(slash_command, prefix_command, rename = "prata")]
pub async fn chat(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<(), Report> {
    ctx.defer_or_broadcast().await.context(DeferSnafu)?;

    let characters = CHARACTERS.get_all_sorted_by_similarity(name);

    if characters.is_empty() {
        let response = sample(NO_CHARACTER_PHRASES);
        let msg = ctx.say_with(response).await?;
        sleep(FIVE_SECONDS).await;
        msg.delete_self_and_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    }

    STATISTICS.conversation_started_by(ctx.author());

    let pages = characters.len();
    let mut current_page: usize = 0;
    let mut revisions_per_page = vec![0; pages];
    let id = ctx.id();
    let custom_ids = ["prev", "next", "edit", "undo", "redo"]
        .map(|s| format!("{id}{s}"))
        .to_vec();

    let (msg, msg_id) = send_initial_message(ctx, characters.clone()).await?;

    let mut similarities_characters_histories =
        to_similarities_characters_histories(characters, msg_id, ctx.author());

    while let Some(interaction) = ComponentInteractionCollector::new(ctx)
        .custom_ids(custom_ids.clone())
        .timeout(ONE_HOUR)
        .await
    {
        let interaction_type = InteractionType::try_from(&interaction)?;
        let mut history = similarities_characters_histories[current_page].2.clone();

        let last = history.last();

        match interaction_type {
            InteractionType::Prev => {
                ctx.acknowledge(interaction).await?;
                current_page = (current_page + pages - 1) % pages;
            }
            InteractionType::Next => {
                ctx.acknowledge(interaction).await?;
                current_page = (current_page + 1) % pages;
            }
            InteractionType::Edit => {
                // no acknowledge because showing a modal is a response
                let response = display_edit_modal(ctx, interaction.clone()).await?;
                if let Some(response) = response {
                    history.edit_content(response, ctx.author());
                    similarities_characters_histories[current_page].2 = history.clone();
                    revisions_per_page[current_page] = history.last().revisions_len();
                }
            }
            InteractionType::Undo => {
                ctx.acknowledge(interaction).await?;
                revisions_per_page[current_page] = revisions_per_page[current_page]
                    .checked_sub(1)
                    .unwrap_or(revisions_per_page[current_page] - 1);
                history.set_revision(revisions_per_page[current_page]);
            }
            InteractionType::Redo => {
                ctx.acknowledge(interaction).await?;
                revisions_per_page[current_page] += 1;
                if revisions_per_page[current_page] > last.revisions_len() {
                    revisions_per_page[current_page] = 0;
                }
                history.set_revision(revisions_per_page[current_page]);
            }
        }

        handle_post_interaction(
            ctx,
            similarities_characters_histories.clone(),
            current_page,
            pages,
            revisions_per_page.clone(),
            &msg,
        )
        .await?;
    }

    msg.delete_self_and_invoking_message_if_prefix(ctx).await?;

    Ok(())
}

async fn handle_post_interaction(
    ctx: Context<'_>,
    similarities_characters_histories: Vec<(f64, Character, History)>,
    current_page: usize,
    pages: usize,
    revisions_per_page: Vec<usize>,
    msg: &ReplyHandle<'_>,
) -> Result<()> {
    let (similarity, character, history) = similarities_characters_histories[current_page].clone();

    HISTORIES.insert(history.clone());

    let current_revision = revisions_per_page[current_page];

    let last = history.last();

    // indexing is fine because we have checked that characters is not empty
    let content_pages = (current_page, pages);
    let edit_pages_and_editor = (
        current_revision,
        last.revisions_len(),
        last.editor_of_version(current_revision),
    );
    let content = last.get_version_content(current_revision);
    let has_finished = HasFinished::Yes;

    msg.edit(
        ctx,
        character.master_reply(
            ctx.id(),
            content_pages,
            edit_pages_and_editor,
            Some(similarity),
            content,
            None,
            has_finished,
        ),
    )
    .await
    .context(EditMessageSnafu)?;
    Ok(())
}

async fn send_initial_message(
    ctx: Context<'_>,
    characters: Vec<(f64, Character)>,
) -> Result<(ReplyHandle<'_>, MessageId)> {
    let (msg, msg_id) = {
        let (msg, character) = {
            let (similarity, character) = characters[0].clone();

            let msg = ctx
                .send(character.master_reply(
                    ctx.id(),
                    (0, characters.len()),
                    (0, 0, None::<UserId>),
                    Some(similarity),
                    character.greeting(),
                    None,
                    HasFinished::Yes,
                ))
                .await
                .context(SendMessageSnafu)?;
            (msg, character)
        };
        let msg_id = msg.message().await.context(RetrieveMessageSnafu)?.id;
        let history = History::from((character, msg_id, ctx.author().id));
        HISTORIES.insert(history);
        (msg, msg_id)
    };
    Ok((msg, msg_id))
}

fn to_similarities_characters_histories(
    characters: Vec<(f64, Character)>,
    msg_id: MessageId,
    user_id: &(impl Into<UserId> + Clone),
) -> Vec<(f64, Character, History)> {
    characters
        .into_iter()
        .map(|(similarity, character)| {
            (
                similarity,
                character.clone(),
                History::from((character, msg_id, user_id.clone().into())),
            )
        })
        .collect()
}

async fn display_edit_modal(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
) -> Result<Option<String>> {
    execute_modal_on_component_interaction::<EditMessageModal>(
        ctx,
        interaction,
        None,
        Some(TEN_MINUTES),
    )
    .await
    .context(ShowModalSnafu)
    .map(|m| m.map(|m| m.content))
}

impl TryFrom<&ComponentInteraction> for InteractionType {
    type Error = Error;

    fn try_from(input: &ComponentInteraction) -> Result<Self, Self::Error> {
        match input.data.custom_id.as_str() {
            i if i.ends_with("prev") => Ok(Self::Prev),
            i if i.ends_with("next") => Ok(Self::Next),
            i if i.ends_with("edit") => Ok(Self::Edit),
            i if i.ends_with("undo") => Ok(Self::Undo),
            i if i.ends_with("redo") => Ok(Self::Redo),
            i => Err(Error::UnknownInteraction {
                found: i.to_string(),
            }),
        }
    }
}
