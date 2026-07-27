//! The persistent button-id codec: the interaction-kind vocabulary and the
//! `<message_id><4-char tag>` `custom_id` encode/decode shared app-wide by the
//! renderers, the paginators, and the chat-event dispatcher.

use core::fmt::{self, Display, Formatter};
use miette::{Diagnostic, SourceSpan};
use poise::serenity_prelude::{ComponentInteraction, MessageId};
use snafu::Snafu;

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
    /// Continue this finished reply, streaming more text appended in place.
    Continue,
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
    help("Knappen hör till ett gammalt meddelande och fungerar inte längre."),
    code(events::interaction::unknown)
)]
pub struct UnknownInteraction {
    /// The ID of the interaction.
    #[source_code]
    custom_id: String,
    /// The part that was wrong (in practice, the entire ID is selected).
    #[label]
    span: SourceSpan,
}

impl UnknownInteraction {
    /// The stale-button error for `custom_id`, labelling the whole id.
    ///
    /// Besides an unparsable id, this also covers a button whose message or
    /// character no longer resolves: to the user both mean the same thing, a
    /// button on a message that is no longer backed by anything.
    #[must_use]
    pub fn stale(custom_id: &str) -> Self {
        Self {
            custom_id: custom_id.to_owned(),
            span: (0..custom_id.len()).into(),
        }
    }
}

impl InteractionKind {
    /// Every interaction kind, the basis for tag round-tripping and the round-trip test.
    const ALL: [Self; 16] = [
        Self::Previous,
        Self::Next,
        Self::Edit,
        Self::Undo,
        Self::Redo,
        Self::Pin,
        Self::Tts,
        Self::Stop,
        Self::Continue,
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
            Self::Continue => "cont",
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
            .ok_or_else(|| UnknownInteraction::stale(value))
    }
}

impl Interaction {
    /// Parses a component `custom_id` (`<message_id><4-char tag>`) into an interaction, the
    /// inverse of [`InteractionKind::custom_id`].
    pub(super) fn parse(custom_id: &str) -> Result<Self, UnknownInteraction> {
        let (str_id, str_kind) = custom_id
            .split_at_checked(custom_id.len().saturating_sub(4))
            .ok_or_else(|| UnknownInteraction::stale(custom_id))?;
        let id = MessageId::from(
            str_id
                .parse::<u64>()
                .map_err(|_why| UnknownInteraction::stale(str_id))?,
        );
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

    /// The Continue button carries the `cont` tag, so its persistent `custom_id`
    /// stays stable across restarts.
    #[test]
    fn continue_tag_is_cont() {
        assert_eq!(
            InteractionKind::Continue.as_tag(),
            "cont",
            "the Continue kind encodes as the cont tag"
        );
        assert_eq!(
            InteractionKind::try_from("cont").ok(),
            Some(InteractionKind::Continue),
            "the cont tag decodes back to Continue"
        );
    }

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

    /// Every kind survives a full encode/decode through the production
    /// `custom_id` builder, so a button's tag always parses back to its kind.
    #[test]
    fn custom_id_round_trips_for_every_kind() {
        for kind in InteractionKind::ALL {
            let encoded = kind.custom_id(123_456_u64);
            let parsed = Interaction::parse(&encoded);
            assert!(
                parsed.is_ok(),
                "the encoded custom_id {encoded:?} should parse"
            );
            let Ok(interaction) = parsed else { continue };
            assert_eq!(
                interaction.id,
                MessageId::new(123_456),
                "the leading digits of {encoded:?} decode to the message ID"
            );
            assert_eq!(
                interaction.kind, kind,
                "the encoded custom_id {encoded:?} did not round-trip to its kind"
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
