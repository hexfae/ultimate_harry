use miette::{Diagnostic, Report};
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::Snafu;

use character::character;
use edit::edit;
use next::next;
use pin::pin;
use previous::previous;
use redo::redo;
use undo::undo;

use crate::db::Database;

mod character;
mod edit;
mod next;
mod pin;
mod previous;
mod redo;
mod undo;

pub async fn interaction_create(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
) -> Result<(), Report> {
    let ultimate_interaction = TryInto::<UltimateInteraction>::try_into(interaction)?;

    let (id, kind) = (ultimate_interaction.id, ultimate_interaction.kind);

    match kind {
        InteractionKind::Prev => previous(ctx, interaction, id, db).await?,
        InteractionKind::Next => next(ctx, interaction, id, db).await?,
        InteractionKind::Edit => edit(ctx, interaction, id, db).await?,
        InteractionKind::Undo => undo(ctx, interaction, id, db).await?,
        InteractionKind::Redo => redo(ctx, interaction, id, db).await?,
        InteractionKind::Pin => pin(ctx, interaction, id, db).await?,
        InteractionKind::Char => character(ctx, interaction, id, db).await?,
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
    Pin,
    Char,
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
            "pinn" => Ok(Self::Pin),
            "char" => Ok(Self::Char),
            _ => Err(UnknownInteraction),
        }
    }
}

impl TryFrom<&ComponentInteraction> for UltimateInteraction {
    type Error = UnknownInteraction;

    fn try_from(interaction: &ComponentInteraction) -> Result<Self, Self::Error> {
        let id = &interaction.data.custom_id;
        let (id, kind) = id
            .split_at_checked((id.len() - 4) as usize)
            .ok_or(UnknownInteraction)?;
        let id = MessageId::from(id.parse::<u64>().map_err(|_| UnknownInteraction)?);
        let kind = kind.try_into()?;
        Ok(Self { id, kind })
    }
}
