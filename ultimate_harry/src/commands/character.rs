use std::time::Duration;

use poise::serenity_prelude::{
    ComponentInteraction, ComponentInteractionCollector, CreateInteractionResponse,
    CreateInteractionResponseMessage,
};
use ulid::Ulid;
use ultimate_character::{CHARACTERS, Character};
use ultimate_harry::{Context, Result};
use ultimate_phrases::{
    CANCELLED_PHRASES, CREATED_PHRASES, DELETED_PHRASES, NO_CHARACTER_PHRASES, sample,
};
use ultimate_statistics::STATISTICS;

const FIVE_SECONDS: Duration = Duration::from_secs(5);
const ONE_MINUTE: Duration = Duration::from_secs(60);

#[poise::command(
    slash_command,
    prefix_command,
    subcommands("create", "edit", "view", "delete"),
    subcommand_required,
    rename = "gubbe"
)]
pub async fn character(_: Context<'_>) -> Result<()> {
    Ok(())
}

#[poise::command(slash_command, prefix_command, rename = "skapa")]
async fn create(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
    #[rename = "hälsning"]
    #[description = "Gubbens hälsning"]
    greeting: String,
) -> Result<()> {
    ctx.defer_ephemeral().await?;
    STATISTICS.write().character_created_by(ctx.author());

    CHARACTERS.write().insert(
        Character::builder()
            .name(&name)
            .greeting(&greeting)
            .creator(ctx.author())
            .build(),
    );
    ctx.say(sample(CREATED_PHRASES).replace("{character}", &name))
        .await?;
    Ok(())
}

#[poise::command(slash_command, prefix_command, rename = "ändra")]
async fn edit(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    _name: String,
) -> Result<()> {
    ctx.defer_ephemeral().await?;
    STATISTICS.write().character_edited_by(ctx.author());

    ctx.say("TODO").await?; // TODO:
    Ok(())
}

#[poise::command(slash_command, prefix_command, rename = "visa")]
async fn view(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<()> {
    ctx.defer_ephemeral().await?;
    STATISTICS.write().character_viewed_by(ctx.author());

    let character = CHARACTERS.read().get_closest(&name);

    if let Some(character) = character {
        ctx.send(character.to_embed_reply()).await?;
    } else {
        ctx.say(sample(NO_CHARACTER_PHRASES)).await?;
    }
    Ok(())
}

#[poise::command(slash_command, prefix_command, rename = "döda")]
async fn delete(
    ctx: Context<'_>,
    #[rest]
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    name: String,
) -> Result<()> {
    ctx.defer_ephemeral().await?;

    let Some(character) = CHARACTERS.read().get_closest(&name) else {
        ctx.say(sample(NO_CHARACTER_PHRASES)).await?;
        return Ok(());
    };

    ctx.send(character.to_confirm_reply(ctx.id())).await?;
    let id = ctx.id();
    let confirm_id = format!("{id}confirm");
    let cancel_id = format!("{id}cancel");

    while let Some(interaction) = ComponentInteractionCollector::new(ctx)
        .filter(move |interaction| {
            interaction
                .data
                .custom_id
                .as_str()
                .starts_with(&id.to_string())
        })
        .author_id(ctx.author().id)
        .timeout(ONE_MINUTE)
        .await
    {
        if interaction.data.custom_id == cancel_id {
            cancel(ctx, interaction).await?;
            break;
        } else if interaction.data.custom_id == confirm_id {
            confirm(ctx, interaction, character.id()).await?;
            break;
        }
    }
    Ok(())
}

async fn cancel(ctx: Context<'_>, interaction: ComponentInteraction) -> Result<()> {
    interaction
        .respond_with(ctx, sample(CANCELLED_PHRASES))
        .await?;
    std::thread::sleep(FIVE_SECONDS);
    interaction.delete_response(ctx).await?;
    Ok(())
}

async fn confirm(
    ctx: Context<'_>,
    interaction: ComponentInteraction,
    id: impl Into<Ulid>,
) -> Result<()> {
    CHARACTERS.write().delete_by_id(id.into(), ctx.author());
    STATISTICS.write().character_deleted_by(ctx.author());

    interaction
        .respond_with(ctx, sample(DELETED_PHRASES))
        .await?;
    std::thread::sleep(FIVE_SECONDS);
    interaction.delete_response(ctx).await?;
    Ok(())
}

trait RespondWith {
    async fn respond_with(&self, ctx: Context<'_>, message: impl AsRef<str>) -> Result<()>;
}

impl RespondWith for ComponentInteraction {
    async fn respond_with(&self, ctx: Context<'_>, message: impl AsRef<str>) -> Result<()> {
        self.create_response(
            ctx,
            CreateInteractionResponse::UpdateMessage(
                CreateInteractionResponseMessage::new()
                    .content(message.as_ref())
                    .embeds(vec![])
                    .components(vec![]),
            ),
        )
        .await?;
        Ok(())
    }
}
