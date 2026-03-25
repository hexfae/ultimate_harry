use miette::{Diagnostic, SourceSpan};
use serenity::all::ComponentInteraction;
use snafu::Snafu;

pub enum InteractionType {
    Prev,
    Next,
    Confirm,
    Cancel,
}

impl TryFrom<&ComponentInteraction> for InteractionType {
    type Error = InteractionError;

    fn try_from(input: &ComponentInteraction) -> Result<Self, Self::Error> {
        match input.data.custom_id.as_str() {
            i if i.ends_with("aprev") => Ok(Self::Prev),
            i if i.ends_with("anext") => Ok(Self::Next),
            i if i.ends_with("aconfirm") => Ok(Self::Confirm),
            i if i.ends_with("acancel") => Ok(Self::Cancel),
            i => Err(InteractionError {
                found: i.to_owned(),
                span: (0, i.len()).into(),
            }),
        }
    }
}

#[derive(Debug, Snafu, Diagnostic)]
#[snafu(display("Okänd interaktion: '{}' hittades", found))]
#[snafu(visibility(pub))]
#[diagnostic(
    code(commands::character::interaction),
    severity(Warning),
    help("Valid suffixes are: 'prev', 'next', 'confirm', 'cancel'")
)]
pub struct InteractionError {
    #[source_code]
    pub found: String,
    #[label("här")]
    pub span: SourceSpan,
}
