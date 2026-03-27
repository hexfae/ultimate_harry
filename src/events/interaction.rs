//! The different interaction events that can happen on a chat message.

mod character;
mod edit;
mod next;
mod pin;
mod previous;
mod redo;
mod undo;

use crate::database::Database;
use miette::{Diagnostic, Result, SourceSpan};
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::Snafu;

use character::character;
use edit::edit;
use next::next;
use pin::pin;
use previous::previous;
use redo::redo;
use undo::undo;

/// An interaction that happened on Ultimate Harry.
#[derive(Debug)]
struct UltimateInteraction {
    ///  The ID of the relevant message.
    id: MessageId,
    /// The kind of interaction that happened.
    kind: InteractionKind,
}

/// The different kinds of possible interactions.
#[derive(Debug)]
enum InteractionKind {
    /// Show the previous reply.
    Previous,
    /// Show the next reply or generate a new one.
    Next,
    /// Edit the contents of this reply.
    Edit,
    /// Show the previous revision of this reply.
    Undo,
    /// Show the next revision of this reply.
    Redo,
    /// Send this reply in the pin channel.
    Pin,
    /// Send a new reply to this reply as the given character.
    Character,
}

/// An unknown interaction happened.
#[derive(Debug, Snafu, Diagnostic)]
#[snafu(display("Okänd interaktion: {custom_id}"))]
struct UnknownInteraction {
    /// The ID of the interaction.
    #[source_code]
    custom_id: String,
    /// The part that was wrong (in practice, the entire ID is selected).
    #[label]
    span: SourceSpan,
}

/// Handle the interaction based on which button was pressed.
pub async fn component(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
) -> Result<()> {
    let ultimate_interaction = TryInto::<UltimateInteraction>::try_into(interaction)?;

    let (id, kind) = (ultimate_interaction.id, ultimate_interaction.kind);

    match kind {
        InteractionKind::Previous => previous(ctx, interaction, id, db).await?,
        InteractionKind::Next => next(ctx, interaction, id, db).await?,
        InteractionKind::Edit => edit(ctx, interaction, id, db).await?,
        InteractionKind::Undo => undo(ctx, interaction, id, db).await?,
        InteractionKind::Redo => redo(ctx, interaction, id, db).await?,
        InteractionKind::Pin => pin(ctx, interaction, id, db).await?,
        InteractionKind::Character => character(ctx, interaction, id, db).await?,
    }
    Ok(())
}

impl TryFrom<&str> for InteractionKind {
    type Error = UnknownInteraction;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "prev" => Ok(Self::Previous),
            "next" => Ok(Self::Next),
            "edit" => Ok(Self::Edit),
            "undo" => Ok(Self::Undo),
            "redo" => Ok(Self::Redo),
            "pinn" => Ok(Self::Pin),
            "char" => Ok(Self::Character),
            _ => Err(UnknownInteraction {
                custom_id: value.to_owned(),
                span: (0..value.len()).into(),
            }),
        }
    }
}

impl TryFrom<&ComponentInteraction> for UltimateInteraction {
    type Error = UnknownInteraction;

    fn try_from(interaction: &ComponentInteraction) -> Result<Self, Self::Error> {
        let custom_id = interaction.data.custom_id.to_string();
        let (str_id, str_kind) = custom_id
            .split_at_checked(custom_id.len().saturating_sub(4))
            .ok_or_else(|| UnknownInteraction {
                custom_id: custom_id.clone(),
                span: (0..custom_id.len()).into(),
            })?;
        let id = MessageId::from(str_id.parse::<u64>().map_err(|_why| UnknownInteraction {
            custom_id: str_id.to_owned(),
            span: (0..str_id.to_owned().len()).into(),
        })?);
        let kind = str_kind.try_into()?;
        Ok(Self { id, kind })
    }
}
