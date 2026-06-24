//! The different interaction events that can happen on a chat message.

mod character;
mod edit;
mod next;
mod pin;
mod speak;
mod stop;
mod swipe;
mod tts;
mod voice;

use crate::{
    AppResult,
    cancellation::Cancellations,
    database::Database,
    error::{AppError, SendResponseSnafu},
    error_display::{error_followup, error_response},
    events::lookup::history_and_character_of,
    models::history::History,
    util::report_error,
};
use core::fmt::{self, Display, Formatter};
use miette::{Diagnostic, Result, SourceSpan};
use poise::serenity_prelude::{ComponentInteraction, Context, MessageId};
use snafu::{ResultExt as _, Snafu};
use tracing::warn;

use character::character as character_fn;
use edit::edit;
use next::next;
use pin::pin;
use stop::stop;
use swipe::swipe;
use tts::tts;
use voice::voice;

/// An interaction that happened on Ultimate Harry.
#[derive(Debug)]
pub struct Interaction {
    ///  The ID of the relevant message.
    pub id: MessageId,
    /// The kind of interaction that happened.
    pub kind: InteractionKind,
}

/// The different kinds of possible interactions.
#[derive(Debug, PartialEq, Eq)]
#[expect(
    clippy::module_name_repetitions,
    reason = "this is the public name for an interaction's variants, so the module prefix disambiguates it at use sites"
)]
pub enum InteractionKind {
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
    /// Speak this reply aloud as a TTS audio attachment.
    Tts,
    /// Stop this reply mid-stream, keeping whatever has streamed so far.
    Stop,
    /// Send a new reply to this reply as the given character.
    Character,
    /// Speak this reply aloud with a chosen palette voice, or auto-assigned voices.
    Voice,
    /// Confirm performing the desired operation.
    Confirm,
    /// Cancel performing the desired operation.
    Cancel,
    /// Show an older version of the character being viewed.
    OlderVersion,
    /// Show a newer version of the character being viewed.
    NewerVersion,
    /// Roll the character back to the older version being viewed.
    Rollback,
}

/// An unknown interaction happened.
#[derive(Debug, Snafu, Diagnostic)]
#[snafu(display("Okänd interaktion: {custom_id}"))]
#[diagnostic(
    help("Knappen kan vara för gammal, försök igen."),
    code(events::interaction::unknown)
)]
#[expect(
    clippy::module_name_repetitions,
    reason = "this is the public error type for an unparseable interaction, so the module prefix disambiguates it at use sites"
)]
pub struct UnknownInteraction {
    /// The ID of the interaction.
    #[source_code]
    custom_id: String,
    /// The part that was wrong (in practice, the entire ID is selected).
    #[label]
    span: SourceSpan,
}

/// Handle the interaction based on which button was pressed, surfacing any
/// failure to the user as an ephemeral error notice before returning it to be
/// logged, so a button press never silently does nothing.
pub async fn component(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    cancellations: &Cancellations,
) -> AppResult {
    let result = dispatch(ctx, interaction, db, cancellations).await;
    if let Err(ref why) = result {
        report_failure(ctx, interaction, why).await;
    }
    result
}

/// Surfaces a failed interaction to the user as an ephemeral error notice.
///
/// An unacknowledged interaction (the common case: a failure before any
/// response) needs its initial response within Discord's 3-second window, so that
/// is tried first; an already-acknowledged interaction (a failure after a
/// placeholder was posted) rejects a second response, so it falls back to a
/// followup. Logs if neither can be sent.
async fn report_failure(ctx: &Context, interaction: &ComponentInteraction, why: &AppError) {
    let message = why.user_message();
    if interaction
        .create_response(&ctx.http, error_response(message.clone()))
        .await
        .is_ok()
    {
        return;
    }
    if let Err(report_why) = interaction
        .create_followup(&ctx.http, error_followup(message))
        .await
        .context(SendResponseSnafu)
    {
        warn!(
            custom_id = %interaction.data.custom_id,
            "failed to show the error notice"
        );
        report_error(report_why);
    }
}

/// Parses the pressed component and dispatches to the matching button handler.
async fn dispatch(
    ctx: &Context,
    interaction: &ComponentInteraction,
    db: &Database,
    cancellations: &Cancellations,
) -> AppResult {
    let ultimate_interaction = TryInto::<Interaction>::try_into(interaction)?;

    let (id, kind) = (ultimate_interaction.id, ultimate_interaction.kind);

    if matches!(kind, InteractionKind::Confirm | InteractionKind::Cancel) {
        return Ok(());
    }

    // stopping needs no history load: it just cancels the in-flight stream, which
    // re-renders the frozen reply itself.
    if kind == InteractionKind::Stop {
        return stop(ctx, interaction, id, cancellations).await;
    }

    let Some((history, character)) = history_and_character_of(id, db).await? else {
        return Ok(());
    };

    match kind {
        InteractionKind::Previous => {
            swipe(ctx, interaction, id, db, history, character, History::previous).await?;
        }
        InteractionKind::Next => {
            next(ctx, interaction, id, db, history, character, cancellations).await?;
        }
        InteractionKind::Edit => edit(ctx, interaction, id, db, history, character).await?,
        InteractionKind::Undo => {
            swipe(ctx, interaction, id, db, history, character, History::undo).await?;
        }
        InteractionKind::Redo => {
            swipe(ctx, interaction, id, db, history, character, History::redo).await?;
        }
        InteractionKind::Pin => pin(ctx, interaction, db, history, character).await?,
        InteractionKind::Tts => tts(ctx, interaction, db, history, character).await?,
        InteractionKind::Voice => voice(ctx, interaction, db, history, character).await?,
        InteractionKind::Character => {
            character_fn(ctx, interaction, db, history, cancellations).await?;
        }
        // stop is handled above, before the history load; the rest only ever fire
        // on the ephemeral /gubbe visa paginator, collected by that command's own
        // collector, never on a chat message's history
        InteractionKind::Stop
        | InteractionKind::Confirm
        | InteractionKind::Cancel
        | InteractionKind::OlderVersion
        | InteractionKind::NewerVersion
        | InteractionKind::Rollback => {}
    }
    Ok(())
}

impl InteractionKind {
    /// Every interaction kind, the basis for tag round-tripping and the round-trip test.
    const ALL: [Self; 15] = [
        Self::Previous,
        Self::Next,
        Self::Edit,
        Self::Undo,
        Self::Redo,
        Self::Pin,
        Self::Tts,
        Self::Stop,
        Self::Character,
        Self::Voice,
        Self::Confirm,
        Self::Cancel,
        Self::OlderVersion,
        Self::NewerVersion,
        Self::Rollback,
    ];

    /// The 4-character tag that encodes this kind in a component's `custom_id`.
    #[must_use]
    pub const fn as_tag(&self) -> &'static str {
        match *self {
            Self::Previous => "prev",
            Self::Next => "next",
            Self::Edit => "edit",
            Self::Undo => "undo",
            Self::Redo => "redo",
            Self::Pin => "pinn",
            Self::Tts => "tala",
            Self::Stop => "stop",
            Self::Character => "char",
            Self::Voice => "voic",
            Self::Confirm => "conf",
            Self::Cancel => "canc",
            Self::OlderVersion => "ovrs",
            Self::NewerVersion => "nvrs",
            Self::Rollback => "roll",
        }
    }

    /// Builds the component `custom_id` (`<message_id><4-char tag>`) for this
    /// kind on the message with `into_id`, the inverse of the decode in
    /// [`Interaction::try_from`].
    #[must_use]
    pub fn custom_id<T: Into<u64>>(&self, into_id: T) -> String {
        let id: u64 = into_id.into();
        format!("{id}{self}")
    }
}

impl Display for InteractionKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_tag())
    }
}

impl TryFrom<&str> for InteractionKind {
    type Error = UnknownInteraction;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_tag() == value)
            .ok_or_else(|| UnknownInteraction {
                custom_id: value.to_owned(),
                span: (0..value.len()).into(),
            })
    }
}

impl Interaction {
    /// Parses a component `custom_id` (`<message_id><4-char tag>`) into an interaction, the
    /// inverse of [`InteractionKind::custom_id`].
    fn parse(custom_id: &str) -> Result<Self, UnknownInteraction> {
        let (str_id, str_kind) = custom_id
            .split_at_checked(custom_id.len().saturating_sub(4))
            .ok_or_else(|| UnknownInteraction {
                custom_id: custom_id.to_owned(),
                span: (0..custom_id.len()).into(),
            })?;
        let id = MessageId::from(str_id.parse::<u64>().map_err(|_why| UnknownInteraction {
            custom_id: str_id.to_owned(),
            span: (0..str_id.len()).into(),
        })?);
        let kind = str_kind.try_into()?;
        Ok(Self { id, kind })
    }
}

impl TryFrom<&ComponentInteraction> for Interaction {
    type Error = UnknownInteraction;

    fn try_from(interaction: &ComponentInteraction) -> Result<Self, Self::Error> {
        Self::parse(&interaction.data.custom_id)
    }
}

/// Tests for the tag encoding shared by every component `custom_id`.
#[cfg(test)]
mod tests {
    use super::{Interaction, InteractionKind};
    use serenity::all::MessageId;

    /// Each kind's tag must be 4 characters and parse back to the same kind.
    #[test]
    fn tags_round_trip() {
        for kind in InteractionKind::ALL {
            let tag = kind.as_tag();
            assert_eq!(tag.len(), 4, "tag {tag:?} must be exactly 4 characters");
            let reparsed = InteractionKind::try_from(tag).ok();
            assert_eq!(
                reparsed,
                Some(kind),
                "tag {tag:?} did not round-trip to its kind"
            );
        }
    }

    /// A well-formed `custom_id` splits into the leading message ID and the trailing kind tag.
    #[test]
    fn custom_id_splits_into_message_id_and_kind() {
        let previous = Interaction::parse("123456prev");
        assert!(previous.is_ok(), "a well-formed custom_id should parse");
        if let Ok(interaction) = previous {
            assert_eq!(
                interaction.id,
                MessageId::new(123_456),
                "the leading digits decode to the message ID"
            );
            assert_eq!(
                interaction.kind,
                InteractionKind::Previous,
                "the trailing tag decodes to its kind"
            );
        }

        let character = Interaction::parse("789char");
        assert!(character.is_ok(), "a shorter message ID should still parse");
        if let Ok(interaction) = character {
            assert_eq!(
                interaction.id,
                MessageId::new(789),
                "a shorter ID still decodes correctly"
            );
            assert_eq!(
                interaction.kind,
                InteractionKind::Character,
                "the char tag decodes to the character kind"
            );
        }
    }

    /// Malformed `custom_id`s (empty ID, non-numeric ID, unknown tag) are rejected.
    #[test]
    fn malformed_custom_ids_are_rejected() {
        assert!(
            Interaction::parse("prev").is_err(),
            "a custom_id with no message ID is rejected"
        );
        assert!(
            Interaction::parse("abcedit").is_err(),
            "a non-numeric message ID is rejected"
        );
        assert!(
            Interaction::parse("123456zzzz").is_err(),
            "an unknown trailing tag is rejected"
        );
    }
}
