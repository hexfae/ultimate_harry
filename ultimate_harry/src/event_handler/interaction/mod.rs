use miette::{Diagnostic, Report};
use poise::serenity_prelude::{ComponentInteraction, Context, Interaction, MessageId};
use snafu::Snafu;

use edit::edit;
use next::next;
use previous::previous;
use redo::redo;
use undo::undo;

mod edit;
mod next;
mod previous;
mod redo;
mod undo;

pub async fn interaction_create(ctx: &Context, interaction: &Interaction) -> Result<(), Report> {
    let Interaction::Component(interaction) = interaction else {
        return Ok(());
    };

    let ultimate_interaction = TryInto::<UltimateInteraction>::try_into(interaction)?;

    let (id, kind) = (ultimate_interaction.id, ultimate_interaction.kind);

    #[allow(clippy::match_same_arms)]
    match kind {
        InteractionKind::Prev => previous(ctx, interaction, id).await?,
        InteractionKind::Next => next(ctx, interaction, id).await?,
        InteractionKind::Edit => edit(ctx, interaction, id).await?,
        InteractionKind::Undo => undo(ctx, interaction, id).await?,
        InteractionKind::Redo => redo(ctx, interaction, id).await?,
    }
    Ok(())
}

#[derive(Debug)]
struct UltimateInteraction {
    id: MessageId,
    kind: InteractionKind,
}

#[derive(Debug)]
enum InteractionKind {
    Prev,
    Next,
    Edit,
    Undo,
    Redo,
}

#[derive(Debug, Snafu, Diagnostic)]
struct UnknownInteraction;

impl TryFrom<&str> for InteractionKind {
    type Error = UnknownInteraction;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "prev" => Ok(Self::Prev),
            "next" => Ok(Self::Next),
            "edit" => Ok(Self::Edit),
            "undo" => Ok(Self::Undo),
            "redo" => Ok(Self::Redo),
            _ => Err(UnknownInteraction),
        }
    }
}

impl TryFrom<&ComponentInteraction> for UltimateInteraction {
    type Error = UnknownInteraction;

    fn try_from(interaction: &ComponentInteraction) -> Result<Self, Self::Error> {
        let id = &interaction.data.custom_id;
        let (id, kind) = id
            .split_at_checked(id.len() - 4)
            .ok_or(UnknownInteraction)?;
        let id = MessageId::from(id.parse::<u64>().map_err(|_| UnknownInteraction)?);
        let kind = kind.try_into()?;
        Ok(Self { id, kind })
    }
}
