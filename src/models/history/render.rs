//! Discord Components V2 rendering for [`History`]: the placeholder and full
//! response builders, the footer line, and the title/button section helpers.
//!
//! Split out from the model in `history.rs`; as a child module this can still
//! reach `History`'s private fields and helper methods.

use alloc::borrow::Cow;
use core::time::Duration;
use poise::CreateReply;
use serenity::all::{
    CreateActionRow, CreateAllowedMentions, CreateComponent, CreateContainer,
    CreateContainerComponent, CreateEmbed, CreateEmbedFooter, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateMessage, CreateSection, CreateSectionAccessory,
    CreateSectionComponent, CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption,
    CreateTextDisplay, CreateThumbnail, CreateUnfurledMediaItem, EditInteractionResponse,
    EditMessage, Message as DiscordMessage, MessageFlags, MessageId,
};

use crate::{
    components::emoji_button,
    constants::{CHARACTER_LIMIT, EDIT, ERROR_COLOUR, ERROR_HEADING, NEXT, PIN, PREVIOUS, REDO, UNDO},
    database::Database,
    events::interaction::InteractionKind,
    models::character::{Character, CharacterOption},
};

use super::History;

/// An empty avatar, a transparent 1x1 PNG.
///
/// Used because Discord sections require an accessory, but a character may not have an avatar set.
const EMPTY_AVATAR: &str = "https://upload.wikimedia.org/wikipedia/commons/c/ca/1x1.png";

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the rendering methods are split into this child module to separate the History model from Discord rendering"
)]
impl History {
    /// Converts the history to a placeholder interaction response.
    pub fn to_placeholder_interaction<'a>(
        &self,
        character: &'a Character,
        options: &[CharacterOption],
    ) -> CreateInteractionResponse<'a> {
        CreateInteractionResponse::UpdateMessage(
            self.to_placeholder(character, Duration::ZERO, options)
                .to_slash_initial_response(CreateInteractionResponseMessage::new()),
        )
    }

    /// Converts the history to a placeholder interaction response edit.
    pub fn to_placeholder_interaction_edit<'a>(
        &self,
        character: &'a Character,
        elapsed: Duration,
        options: &[CharacterOption],
    ) -> EditInteractionResponse<'a> {
        self.to_placeholder(character, elapsed, options)
            .to_slash_initial_response_edit(EditInteractionResponse::new())
    }

    /// Converts the history to a placeholder message.
    pub fn to_placeholder_message<'a>(
        &self,
        character: &'a Character,
        replying_to: &DiscordMessage,
        options: &[CharacterOption],
    ) -> CreateMessage<'a> {
        self.to_placeholder(character, Duration::ZERO, options)
            .to_prefix(replying_to.into())
            .reference_message(replying_to)
            .allowed_mentions(CreateAllowedMentions::new())
    }

    /// Converts the history to a placeholder message edit.
    pub fn to_placeholder_message_edit<'a>(
        &self,
        character: &'a Character,
        elapsed: Duration,
        options: &[CharacterOption],
    ) -> EditMessage<'a> {
        self.to_placeholder(character, elapsed, options)
            .to_prefix_edit(EditMessage::new())
            .allowed_mentions(CreateAllowedMentions::new())
    }

    /// Converts the history to a placeholder.
    fn to_placeholder<'a>(
        &self,
        character: &'a Character,
        elapsed: Duration,
        options: &[CharacterOption],
    ) -> CreateReply<'a> {
        let (has_previous, has_edit) = (false, false);

        let footer = {
            let pages = if self.choices.is_empty() {
                String::new()
            } else {
                format!(
                    "-# {}/{} | tar {:.1}s | 0/{CHARACTER_LIMIT}",
                    self.current.saturating_add(1),
                    self.choices.len(),
                    elapsed.as_secs_f32(),
                )
            };
            vec![CreateContainerComponent::TextDisplay(
                CreateTextDisplay::new(pages),
            )]
            .into()
        };

        let title = vec![character_title_section(character, "…")].into();

        let components = create_buttons(1, self.has_finished, has_previous, has_edit, options);

        let container = vec![CreateComponent::Container(CreateContainer::new(
            [title, components, footer].concat(),
        ))];

        CreateReply::default()
            .flags(MessageFlags::IS_COMPONENTS_V2)
            .components(container)
    }

    /// Converts the history into a bare response reply with the chosen message content.
    pub async fn into_bare_response(
        self,
        character: &Character,
        link: String,
        db: &Database,
    ) -> CreateReply<'static> {
        let content = self
            .chosen_message()
            .chosen_revision()
            .head()
            .content()
            .to_owned();
        let footer = {
            let editor = match self.chosen_message().current_editor() {
                Some(user_id) => db.substitute_name(user_id).await,
                None => String::new(),
            };
            let len = format!("{}/{CHARACTER_LIMIT}", content.len());

            let footer = format!("{len}{editor}");
            CreateEmbedFooter::new(footer)
        };
        let mut embed = CreateEmbed::new()
            .title(character.name().to_owned())
            .description(content)
            .footer(footer);

        if let Some(avatar) = character.avatar() {
            embed = embed.thumbnail(avatar.to_owned());
        }
        if let Some(color) = character.color() {
            embed = embed.color(color);
        }
        CreateReply::default().content(link).embed(embed)
    }

    /// Converts the history to an interaction response.
    pub async fn to_interaction<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
        options: &[CharacterOption],
    ) -> CreateInteractionResponse<'a> {
        CreateInteractionResponse::UpdateMessage(
            self.to_response(character, id, db, options)
                .await
                .to_slash_initial_response(CreateInteractionResponseMessage::new()),
        )
    }

    /// Converts the history to an edit interaction response.
    pub async fn to_edit_interaction<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
        options: &[CharacterOption],
    ) -> EditInteractionResponse<'a> {
        self.to_response(character, id, db, options)
            .await
            .to_slash_initial_response_edit(EditInteractionResponse::new())
    }

    /// Converts the history to an edit message response.
    pub async fn to_edit_response<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
        options: &[CharacterOption],
    ) -> EditMessage<'a> {
        self.to_response(character, id, db, options)
            .await
            .to_prefix_edit(EditMessage::new())
            .allowed_mentions(CreateAllowedMentions::new())
    }

    /// Builds the footer line of a full response: the page counter, name
    /// similarity, timing (or nothing when the reply was edited), reply length,
    /// and edit/revision info. `editor_name` is the resolved display name of the
    /// chosen reply's editor, present exactly when the reply was edited.
    fn footer_text(
        &self,
        character: &Character,
        content_len: usize,
        editor_name: Option<&str>,
    ) -> String {
        let chosen = self.chosen_message();
        let pages = format!("{}/{}", self.current.saturating_add(1), self.choices_count());

        let elapsed = if editor_name.is_some() {
            String::new()
        } else {
            chosen.time_taken().map_or_else(String::new, |elapsed| {
                if self.has_finished {
                    format!(" | tog {:.1}s", elapsed.as_secs_f64())
                } else {
                    format!(" | tar {:.1}s", elapsed.as_secs_f64())
                }
            })
        };

        let similarity = character.similarity();

        let editor =
            editor_name.map_or_else(String::new, |name| format!(" | redigerad av {name}"));

        let edit_pages = if self.chosen_has_edit() {
            format!(
                " | {}/{}{}",
                chosen.revision().saturating_add(1),
                chosen.revisions_count().saturating_add(1),
                editor
            )
        } else {
            String::new()
        };

        let len = format!(" | {content_len}/{CHARACTER_LIMIT}");

        format!("-# {pages}{similarity}{elapsed}{len}{edit_pages}")
    }

    /// Converts the history to a full response with buttons and choices.
    pub async fn to_response<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
        options: &[CharacterOption],
    ) -> CreateReply<'a> {
        let chosen = self.chosen_message();
        let content = chosen.chosen_revision().head().content();
        let editor_name = match chosen.current_editor() {
            Some(user_id) => Some(db.substitute_name(user_id).await),
            None => None,
        };
        let footer = {
            let footer = self.footer_text(character, content.len(), editor_name.as_deref());
            vec![CreateContainerComponent::TextDisplay(
                CreateTextDisplay::new(footer),
            )]
            .into()
        };

        let components = create_buttons(
            id.into().into(),
            self.has_finished,
            self.has_multiple_choices(),
            self.chosen_has_edit(),
            options,
        );

        // a failed generation renders as a distinct red error container rather than
        // as the character speaking, while keeping the buttons so the user can swipe
        // to retry without resending their message
        if chosen.is_error() {
            let heading = vec![CreateContainerComponent::TextDisplay(CreateTextDisplay::new(
                ERROR_HEADING,
            ))]
            .into();
            let body = vec![CreateContainerComponent::TextDisplay(CreateTextDisplay::new(
                content.to_owned(),
            ))]
            .into();
            let container = vec![CreateComponent::Container(
                CreateContainer::new([heading, body, components, footer].concat())
                    .accent_colour(ERROR_COLOUR),
            )];
            return CreateReply::default()
                .flags(MessageFlags::IS_COMPONENTS_V2)
                .components(container);
        }

        // discord rejects a whitespace-only text display; an empty finished choice is the
        // "skip the greeting" swipe option, so show a hint explaining what selecting it does
        let text = if content.is_empty() {
            "(ingen hälsning, ditt meddelande blir det första)"
        } else {
            content
        };
        let (first, second) = match text.split_once('\n') {
            Some((first, second)) => (first, Some(second)),
            None => (text, None),
        };

        let title = vec![character_title_section(character, first)].into();

        let container = vec![CreateComponent::Container(CreateContainer::new(
            [
                title,
                second
                    .map_or_else(Vec::new, |rest| {
                        rest.split('\n')
                            .filter(|line| !line.is_empty())
                            .map(|part| {
                                CreateContainerComponent::TextDisplay(CreateTextDisplay::new(part))
                            })
                            .collect()
                    })
                    .into(),
                components,
                footer,
            ]
            .concat(),
        ))];

        CreateReply::default()
            .flags(MessageFlags::IS_COMPONENTS_V2)
            .components(container)
    }
}

/// Builds the title section of a history message: the character's name as a
/// heading, `body` as the leading text, and the character's avatar (or a
/// transparent placeholder) as the section thumbnail.
fn character_title_section<'a>(
    character: &'a Character,
    body: &'a str,
) -> CreateContainerComponent<'a> {
    CreateContainerComponent::Section(CreateSection::new(
        vec![
            CreateSectionComponent::TextDisplay(CreateTextDisplay::new(format!("## {character}"))),
            CreateSectionComponent::TextDisplay(CreateTextDisplay::new(body)),
        ],
        CreateSectionAccessory::Thumbnail(CreateThumbnail::new(CreateUnfurledMediaItem::new(
            character.avatar().unwrap_or(EMPTY_AVATAR),
        ))),
    ))
}

/// Creates button components for the history message.
fn create_buttons<'a>(
    id: u64,
    finished: bool,
    previous: bool,
    edit: bool,
    options: &[CharacterOption],
) -> Cow<'a, [CreateContainerComponent<'a>]> {
    let prev_msg_id = InteractionKind::Previous.custom_id(id);
    let next_msg_id = InteractionKind::Next.custom_id(id);
    let edit_msg_id = InteractionKind::Edit.custom_id(id);
    let undo_id = InteractionKind::Undo.custom_id(id);
    let redo_id = InteractionKind::Redo.custom_id(id);
    let pin_id = InteractionKind::Pin.custom_id(id);
    let char_id = InteractionKind::Character.custom_id(id);

    let mut components = vec![
        CreateContainerComponent::ActionRow(CreateActionRow::Buttons(
            vec![
                emoji_button(prev_msg_id, PREVIOUS).disabled(!finished || !previous),
                emoji_button(next_msg_id, NEXT).disabled(!finished),
                emoji_button(undo_id, UNDO).disabled(!edit),
                emoji_button(redo_id, REDO).disabled(!edit),
            ]
            .into(),
        )),
        CreateContainerComponent::ActionRow(CreateActionRow::Buttons(
            vec![
                emoji_button(edit_msg_id, EDIT).disabled(!finished),
                emoji_button(pin_id, PIN).disabled(!finished),
            ]
            .into(),
        )),
    ];
    if !options.is_empty() {
        components.push(CreateContainerComponent::ActionRow(
            CreateActionRow::SelectMenu(CreateSelectMenu::new(
                char_id,
                CreateSelectMenuKind::String {
                    options: options
                        .iter()
                        .map(|option| {
                            CreateSelectMenuOption::new(
                                option.label().to_owned(),
                                option.id().to_owned(),
                            )
                        })
                        .collect(),
                },
            )),
        ));
    }
    components.into()
}

/// Tests for the response footer formatting.
#[cfg(test)]
mod tests {
    use crate::models::{character::Character, history::History, message::Message};
    use core::time::Duration;
    use nonempty::NonEmpty;
    use serenity::all::{MessageId, UserId};

    /// Builds a minimal character with no name-similarity score.
    fn character() -> Character {
        Character::builder()
            .id("id".to_owned())
            .name("Harry")
            .greeting("hi")
            .creator(UserId::new(1))
            .build()
    }

    /// Builds a reply that reports `seconds` of generation time.
    fn timed_choice(character: &Character, content: &str, seconds: f64) -> Message {
        Message::from((
            character.clone(),
            content.to_owned(),
            Duration::from_secs_f64(seconds),
        ))
    }

    /// A finished, timed reply shows the page counter, "tog" timing, and length.
    #[test]
    fn footer_shows_pages_finished_time_and_length() {
        let character = character();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(&character, "hello", 2.0)))
            .build();
        assert_eq!(
            history.footer_text(&character, 5, None),
            "-# 1/1 | tog 2.0s | 5/3900",
            "a finished timed reply shows the tog timing and the length"
        );
    }

    /// An unfinished reply shows "tar" timing instead of "tog".
    #[test]
    fn footer_shows_pending_time_when_unfinished() {
        let character = character();
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(&character, "hello", 2.0)))
            .build();
        history.set_finished(false);
        assert_eq!(
            history.footer_text(&character, 5, None),
            "-# 1/1 | tar 2.0s | 5/3900",
            "an unfinished reply shows the tar timing"
        );
    }

    /// An edited reply hides the timing and shows the editor and revision counter.
    #[test]
    fn footer_shows_editor_and_revision_for_an_edit() {
        let character = character();
        let mut choice = timed_choice(&character, "hello", 2.0);
        choice.edit("Bob", "hello there", Some(UserId::new(7)));
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(choice))
            .build();
        assert_eq!(
            history.footer_text(&character, 11, Some("Bob")),
            "-# 1/1 | 11/3900 | 2/2 | redigerad av Bob",
            "an edited reply hides timing and shows the editor and the revision pages"
        );
    }

    /// The page counter tracks which choice is currently chosen.
    #[test]
    fn footer_page_counter_tracks_the_chosen_choice() {
        let character = character();
        let mut choices = NonEmpty::new(timed_choice(&character, "a", 1.0));
        choices.push(timed_choice(&character, "b", 1.0));
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(choices)
            .current(1_usize)
            .build();
        assert_eq!(
            history.footer_text(&character, 1, None),
            "-# 2/2 | tog 1.0s | 1/3900",
            "the page counter shows the second of two choices"
        );
    }
}
