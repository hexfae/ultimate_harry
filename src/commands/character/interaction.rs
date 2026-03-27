//! The interaction type used in commands that manipulate existing characters.

use miette::{Diagnostic, SourceSpan};
use serenity::all::ComponentInteraction;
use snafu::Snafu;

/// The types of buttons that can be interacted with by users.
pub enum InteractionType {
    /// Show the previous character.
    Previous,
    /// Show the next character.
    Next,
    /// Perform the associated action.
    Confirm,
    /// Cancel the operation.
    Cancel,
}

impl TryFrom<&ComponentInteraction> for InteractionType {
    type Error = InteractionError;

    fn try_from(input: &ComponentInteraction) -> Result<Self, Self::Error> {
        match input.data.custom_id.as_str() {
            interaction if interaction.ends_with("prev") => Ok(Self::Previous),
            interaction if interaction.ends_with("next") => Ok(Self::Next),
            interaction if interaction.ends_with("confirm") => Ok(Self::Confirm),
            interaction if interaction.ends_with("cancel") => Ok(Self::Cancel),
            interaction => Err(InteractionError {
                found: interaction.to_owned(),
                span: (0..interaction.len()).into(),
            }),
        }
    }
}

/// An unknown interaction occured.
#[derive(Debug, Snafu, Diagnostic)]
#[snafu(display("Okänd interaktion: '{}' hittades", found))]
#[diagnostic(
    code(commands::character::interaction),
    severity(Warning),
    help("Valid suffixes are: 'prev', 'next', 'confirm', 'cancel'")
)]
pub struct InteractionError {
    /// The entire interaction ID that was found.
    #[source_code]
    found: String,
    /// The part that was wrong (in practice, the entire ID is selected).
    #[label]
    span: SourceSpan,
}
