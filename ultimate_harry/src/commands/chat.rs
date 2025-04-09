use poise::{
    execute_modal_on_component_interaction,
    serenity_prelude::{
        ComponentInteraction, ComponentInteractionCollector, CreateInteractionResponse, UserId,
    },
};
use snafu::Snafu;
use ultimate_character::{CHARACTERS, HasFinished, HasPrevious, HasRedo, HasUndo};
use ultimate_harry::{
    Context, FIVE_SECONDS, ONE_HOUR, Result, TEN_MINUTES, delete_invoking_message_if_prefix,
};
use ultimate_history::{HISTORIES, History};
use ultimate_modals::EditMessageModal;
use ultimate_phrases::{NO_CHARACTER_PHRASES, sample};
use ultimate_statistics::STATISTICS;

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Fick ingen modal tillbaka"))]
    NoModalReturned,
}

#[poise::command(slash_command, prefix_command, rename = "prata")]
pub async fn chat(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<()> {
    ctx.defer_or_broadcast().await?;

    let characters = CHARACTERS.read().get_all_sorted_by_similarity(name);

    if characters.is_empty() {
        let msg = ctx.say(sample(NO_CHARACTER_PHRASES)).await?;
        std::thread::sleep(FIVE_SECONDS);
        msg.delete(ctx).await?;
        delete_invoking_message_if_prefix(ctx).await?;
        return Ok(());
    };

    STATISTICS.write().conversation_started_by(ctx.author());

    let pages = characters.len();
    let mut current_page: usize = 0;
    let mut current_revision_by_page_index = vec![0; pages];
    let id = ctx.id();
    let prev = format!("{id}prev");
    let next = format!("{id}next");
    let edit = format!("{id}edit");
    let undo = format!("{id}undo");
    let redo = format!("{id}redo");

    let (msg, msg_id) = {
        let (msg, character) = {
            let (similarity, character) = characters[0].clone();

            let msg = ctx
                .send(character.master_reply(
                    id,
                    (0, pages),
                    (0, 0, None::<UserId>),
                    Some(similarity),
                    character.greeting(),
                    None,
                    HasFinished::Yes,
                    if pages == 1 {
                        HasPrevious::No
                    } else {
                        HasPrevious::Yes
                    },
                    HasUndo::No,
                    HasRedo::No,
                ))
                .await?;
            (msg, character)
        };
        let msg_id = msg.message().await?.id;
        let history = History::from((character, msg_id, ctx.author().id));
        HISTORIES.write().insert(history.clone());
        (msg, msg_id)
    };

    let mut similarities_characters_histories = characters
        .clone()
        .into_iter()
        .map(|(similarity, character)| {
            (
                similarity,
                character.clone(),
                History::from((character, msg_id, ctx.author().id)),
            )
        })
        .collect::<Vec<_>>();

    while let Some(interaction) = ComponentInteractionCollector::new(ctx)
        .custom_ids(vec![
            prev.clone(),
            next.clone(),
            edit.clone(),
            undo.clone(),
            redo.clone(),
        ])
        .timeout(ONE_HOUR)
        .await
    {
        let interaction_id = interaction.data.custom_id.clone();

        let mut history = similarities_characters_histories[current_page].2.clone();

        let last = history.last();

        if interaction_id == prev {
            ctx.acknowledge(interaction).await?;
            current_page = current_page.checked_sub(1).unwrap_or(pages - 1);
        } else if interaction_id == next {
            ctx.acknowledge(interaction).await?;
            current_page += 1;
            if current_page >= pages {
                current_page = 0;
            }
        } else if interaction_id == edit {
            let response = display_edit_modal(ctx, interaction.clone()).await?;
            history.edit_content(response, ctx.author());
            similarities_characters_histories[current_page].2 = history.clone();
            current_revision_by_page_index[current_page] = history.last().revisions_len();
            // no acknowledge response because showing a modal is a response
        } else if interaction_id == undo {
            ctx.acknowledge(interaction).await?;
            current_revision_by_page_index[current_page] = current_revision_by_page_index
                [current_page]
                .checked_sub(1)
                .unwrap_or(current_revision_by_page_index[current_page] - 1);
            history.set_revision(current_revision_by_page_index[current_page]);
        } else if interaction_id == redo {
            ctx.acknowledge(interaction).await?;
            current_revision_by_page_index[current_page] += 1;
            if current_revision_by_page_index[current_page] > last.revisions_len() {
                current_revision_by_page_index[current_page] = 0;
            }
            history.set_revision(current_revision_by_page_index[current_page]);
        }

        let (similarity, character, history) =
            similarities_characters_histories[current_page].clone();

        HISTORIES.write().insert(history.clone());

        let current_revision = current_revision_by_page_index[current_page];

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
        let has_previous = HasPrevious::Yes;
        let has_undo = if last.revisions_len() > 0 {
            HasUndo::Yes
        } else {
            HasUndo::No
        };
        let has_redo = if current_revision < last.revisions_len() {
            HasRedo::Yes
        } else {
            HasRedo::No
        };

        msg.edit(
            ctx,
            character.master_reply(
                id,
                content_pages,
                edit_pages_and_editor,
                Some(similarity),
                content,
                None,
                has_finished,
                has_previous,
                has_undo,
                has_redo,
            ),
        )
        .await?;
    }

    Ok(())
}

async fn display_edit_modal(ctx: Context<'_>, interaction: ComponentInteraction) -> Result<String> {
    execute_modal_on_component_interaction::<EditMessageModal>(
        ctx,
        interaction,
        None,
        Some(TEN_MINUTES),
    )
    .await?
    .ok_or(Error::NoModalReturned.into())
    .map(|m| m.content)
}

trait AcknowledgeResponse {
    async fn acknowledge(&self, interaction: ComponentInteraction) -> Result<()>;
}

impl AcknowledgeResponse for Context<'_> {
    async fn acknowledge(&self, interaction: ComponentInteraction) -> Result<()> {
        interaction
            .create_response(self, CreateInteractionResponse::Acknowledge)
            .await?;
        Ok(())
    }
}
