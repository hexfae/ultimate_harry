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
    CreateContainerComponent, CreateInteractionResponse, CreateInteractionResponseMessage,
    CreateMessage, CreateSection, CreateSectionAccessory, CreateSectionComponent, CreateSelectMenu,
    CreateSelectMenuKind, CreateSelectMenuOption, CreateTextDisplay, CreateThumbnail,
    CreateUnfurledMediaItem, EditInteractionResponse, EditMessage, Message as DiscordMessage,
    MessageFlags, MessageId,
};

use crate::{
    components::{emoji_button, reaction_from, unicode_emoji},
    constants::{
        AUTO_VOICE, CHARACTER_LIMIT, CONTINUE, EDIT, NEXT, PIN, PREVIOUS, REDO,
        SELECT_OPTION_LIMIT, SPEAK, STOP, UNDO,
    },
    database::Database,
    error_display::error_container,
    events::interaction::InteractionKind,
    models::character::{Character, CharacterOption},
    read_aloud,
    tts::{VoiceEntry, is_speakable},
    util::ellipsize,
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
    pub fn to_placeholder_interaction<'a, M: Into<MessageId>>(
        &self,
        character: &'a Character,
        id: M,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> CreateInteractionResponse<'a> {
        as_update_message(self.to_placeholder(character, id, Duration::ZERO, true, options, voices))
    }

    /// Converts the history to a placeholder interaction response edit.
    pub fn to_placeholder_interaction_edit<'a, M: Into<MessageId>>(
        &self,
        character: &'a Character,
        id: M,
        elapsed: Duration,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> EditInteractionResponse<'a> {
        as_edit_interaction(self.to_placeholder(character, id, elapsed, true, options, voices))
    }

    /// Converts the history to a placeholder message.
    ///
    /// The reply has no Discord ID yet (it is about to be sent), so the Stop
    /// button is rendered disabled (`stoppable = false`); the first streaming
    /// tick re-renders it via
    /// [`to_placeholder_message_edit`](Self::to_placeholder_message_edit) with
    /// the real ID and enables it. The ID passed here is therefore unused.
    pub fn to_placeholder_message<'a>(
        &self,
        character: &'a Character,
        replying_to: &DiscordMessage,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> CreateMessage<'a> {
        self.to_placeholder(
            character,
            MessageId::new(1),
            Duration::ZERO,
            false,
            options,
            voices,
        )
        .to_prefix(replying_to.into())
        .reference_message(replying_to)
        .allowed_mentions(CreateAllowedMentions::new())
    }

    /// Converts the history to a placeholder message edit.
    pub fn to_placeholder_message_edit<'a, M: Into<MessageId>>(
        &self,
        character: &'a Character,
        id: M,
        elapsed: Duration,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> EditMessage<'a> {
        as_message_edit(self.to_placeholder(character, id, elapsed, true, options, voices))
    }

    /// Converts the history to a placeholder, keying its buttons (the live Stop
    /// button in particular) on the reply's message `id`.
    ///
    /// Nothing has streamed in yet, so there is nothing to speak; the speak
    /// button and voice dropdown are disabled anyway while unfinished, but the
    /// dropdown is still rendered so it is present throughout the stream like
    /// the hand-off menu, rather than popping in only when the reply finishes.
    fn to_placeholder<'a, M: Into<MessageId>>(
        &self,
        character: &'a Character,
        id: M,
        elapsed: Duration,
        stoppable: bool,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> CreateReply<'a> {
        let container = self.placeholder_components(
            character,
            id.into().into(),
            elapsed,
            stoppable,
            voices,
            options,
        );
        CreateReply::default()
            .flags(MessageFlags::IS_COMPONENTS_V2)
            .components(container)
    }

    /// Builds the placeholder container, keyed on the reply's message `id`, given
    /// the already-resolved voice palette so this stays synchronous. Carries the
    /// character's accent colour so the placeholder matches the finished reply.
    fn placeholder_components<'a>(
        &self,
        character: &'a Character,
        id: u64,
        elapsed: Duration,
        stoppable: bool,
        voices: &[VoiceEntry],
        options: &[CharacterOption],
    ) -> Vec<CreateComponent<'a>> {
        let (has_previous, has_edit) = (false, false);

        let footer = text_component(format!(
            "-# {}/{} | tar {:.1}s | 0/{CHARACTER_LIMIT}",
            self.current.saturating_add(1),
            self.choices.len(),
            elapsed.as_secs_f32(),
        ));

        let title = vec![character_title_section(character, "…")].into();

        let components = create_buttons(
            id,
            self.has_finished,
            has_previous,
            has_edit,
            false,
            stoppable,
            false,
            options,
            voices,
        );

        vec![card(title, components, footer, character)]
    }

    /// Converts the history into a Components V2 reply with the chosen message
    /// content, for posting in the pin channel; `link` jumps back to the original
    /// message.
    pub async fn into_bare_response<'a>(
        self,
        character: &'a Character,
        link: String,
        db: &Database,
    ) -> CreateReply<'a> {
        let content = self
            .chosen_message()
            .chosen_revision()
            .head()
            .content()
            .to_owned();
        let editor = match self.chosen_message().current_editor() {
            Some(user_id) => db.substitute_name(user_id).await,
            None => String::new(),
        };
        let footer = format!("-# {}/{CHARACTER_LIMIT}{editor}", content.chars().count());

        // discord rejects a whitespace-only text display
        let text = if content.trim().is_empty() {
            "(ingen hälsning)".to_owned()
        } else {
            content
        };
        let mut card = title_and_body_lines(character, &text);
        card.push(CreateContainerComponent::TextDisplay(
            CreateTextDisplay::new(footer),
        ));

        let mut card_container = CreateContainer::new(card);
        if let Some(colour) = character.color() {
            card_container = card_container.accent_colour(colour);
        }

        let jump = CreateComponent::TextDisplay(CreateTextDisplay::new(link));
        let container = CreateComponent::Container(card_container);

        CreateReply::default()
            .flags(MessageFlags::IS_COMPONENTS_V2)
            .components(vec![jump, container])
    }

    /// Converts the history to an interaction response.
    pub async fn to_interaction<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> CreateInteractionResponse<'a> {
        as_update_message(self.to_response(character, id, db, options, voices).await)
    }

    /// Converts the history to an edit interaction response.
    pub async fn to_edit_interaction<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> EditInteractionResponse<'a> {
        as_edit_interaction(self.to_response(character, id, db, options, voices).await)
    }

    /// Converts the history to an edit message response.
    pub async fn to_edit_response<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> EditMessage<'a> {
        as_message_edit(self.to_response(character, id, db, options, voices).await)
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
        let pages = format!(
            "{}/{}",
            self.current.saturating_add(1),
            self.choices_count()
        );

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

        let editor = editor_name.map_or_else(String::new, |name| format!(" | redigerad av {name}"));

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

    /// The body text shown for the chosen reply: its own content, or a
    /// placeholder when the content is blank. A blank choice with no prior
    /// turns is the seeded skip-greeting option (shown with a hint); once a turn
    /// is under way a blank choice is a reply stopped before its first token
    /// (shown as the "…" placeholder).
    fn body_text<'a>(&self, content: &'a str) -> &'a str {
        if !content.trim().is_empty() {
            content
        } else if self.previous_messages().is_empty() {
            "(ingen hälsning, ditt meddelande blir det första)"
        } else {
            "…"
        }
    }

    /// Converts the history to a full response with buttons and choices.
    pub async fn to_response<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> CreateReply<'a> {
        self.to_response_with(character, id.into().into(), true, db, options, voices)
            .await
    }

    /// Converts the history to a full response whose controls are all rendered
    /// disabled.
    ///
    /// The reply has no Discord ID yet (it is about to be sent), so every button
    /// and dropdown would be keyed on a placeholder ID that resolves to no
    /// conversation; the sender re-renders with [`to_response`](Self::to_response)
    /// once the real ID is known, which enables them.
    pub async fn to_pending_response<'a>(
        &'a self,
        character: &'a Character,
        db: &Database,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> CreateReply<'a> {
        self.to_response_with(character, 1, false, db, options, voices)
            .await
    }

    /// Builds the full response keyed on the reply's message `id`, with every
    /// control disabled when the message is not yet `live`.
    async fn to_response_with<'a>(
        &'a self,
        character: &'a Character,
        id: u64,
        live: bool,
        db: &Database,
        options: &[CharacterOption],
        voices: &[VoiceEntry],
    ) -> CreateReply<'a> {
        let editor_name = match self.chosen_message().current_editor() {
            Some(user_id) => Some(db.substitute_name(user_id).await),
            None => None,
        };
        let container = self.render_components(
            character,
            id,
            editor_name.as_deref(),
            live,
            voices,
            options,
        );
        CreateReply::default()
            .flags(MessageFlags::IS_COMPONENTS_V2)
            .components(container)
    }

    /// Renders the chosen reply into the final Components V2 container, given the
    /// already-resolved editor name and voice palette so this stays synchronous.
    /// `live` is false while the message's ID is still unknown, which renders
    /// every control disabled.
    ///
    /// A failed generation is drawn as a distinct red error container rather than
    /// as the character speaking, while keeping the buttons so the user can swipe
    /// to retry without resending their message.
    fn render_components<'a>(
        &'a self,
        character: &'a Character,
        id: u64,
        editor_name: Option<&str>,
        live: bool,
        voices: &[VoiceEntry],
        options: &[CharacterOption],
    ) -> Vec<CreateComponent<'a>> {
        let chosen = self.chosen_message();
        let content = chosen.chosen_revision().head().content();
        let footer =
            text_component(self.footer_text(character, content.chars().count(), editor_name));

        let components = create_buttons(
            id,
            live && self.has_finished,
            self.has_multiple_choices(),
            live && self.chosen_has_edit(),
            is_speakable(content),
            live && !self.has_finished,
            self.show_continue && self.continue_available(),
            options,
            voices,
        );

        if chosen.is_error() {
            let body = text_component(content.to_owned());
            return vec![error_container([body, components, footer].concat())];
        }

        // discord rejects a whitespace-only text display, so an empty choice
        // renders a placeholder: the skip-greeting hint or the stalled "…"
        let text = self.body_text(content);
        vec![card(
            title_and_body_lines(character, text).into(),
            components,
            footer,
            character,
        )]
    }
}

/// Wraps a built [`CreateReply`] as an `UpdateMessage` interaction response.
fn as_update_message(reply: CreateReply<'_>) -> CreateInteractionResponse<'_> {
    CreateInteractionResponse::UpdateMessage(
        reply.to_slash_initial_response(CreateInteractionResponseMessage::new()),
    )
}

/// Wraps a built [`CreateReply`] as an interaction-response edit.
fn as_edit_interaction(reply: CreateReply<'_>) -> EditInteractionResponse<'_> {
    reply.to_slash_initial_response_edit(EditInteractionResponse::new())
}

/// Wraps a built [`CreateReply`] as a message edit that clears allowed mentions.
fn as_message_edit(reply: CreateReply<'_>) -> EditMessage<'_> {
    reply
        .to_prefix_edit(EditMessage::new())
        .allowed_mentions(CreateAllowedMentions::new())
}

/// Wraps `text` as a single text-display container component list, ready to
/// `concat` into a container (the footer, error heading, and error body all share
/// this one-line shape).
fn text_component<'a, T: Into<Cow<'a, str>>>(text: T) -> Cow<'a, [CreateContainerComponent<'a>]> {
    vec![CreateContainerComponent::TextDisplay(
        CreateTextDisplay::new(text),
    )]
    .into()
}

/// Assembles the success-path Components V2 card from its `title`, button `components`,
/// and `footer` sections, tinted with the character's accent colour.
fn card<'a>(
    title: Cow<'a, [CreateContainerComponent<'a>]>,
    components: Cow<'a, [CreateContainerComponent<'a>]>,
    footer: Cow<'a, [CreateContainerComponent<'a>]>,
    character: &Character,
) -> CreateComponent<'a> {
    CreateComponent::Container(with_accent(
        CreateContainer::new([title, components, footer].concat()),
        character,
    ))
}

/// Tints `container` with the character's accent colour, or leaves it untinted
/// when the character has no colour set.
fn with_accent<'a>(container: CreateContainer<'a>, character: &Character) -> CreateContainer<'a> {
    match character.color() {
        Some(colour) => container.accent_colour(colour),
        None => container,
    }
}

/// The character title section followed by one text display per remaining
/// non-blank line of `body`: its first non-blank line becomes the title's
/// leading text and each subsequent non-blank line its own text display. Blank
/// lines are dropped because Discord rejects a whitespace-only text display. The
/// components own their text, so the result outlives the borrowed `body`.
fn title_and_body_lines<'a>(
    character: &'a Character,
    body: &str,
) -> Vec<CreateContainerComponent<'a>> {
    let mut lines = body.lines().filter(|line| !line.trim().is_empty());
    let first = lines.next().unwrap_or_default();
    let mut card = vec![character_title_section(character, first.to_owned())];
    card.extend(lines.map(|part| {
        CreateContainerComponent::TextDisplay(CreateTextDisplay::new(part.to_owned()))
    }));
    card
}

/// Builds the title section of a history message: the character's name as a
/// heading, `body` as the leading text, and the character's avatar (or a
/// transparent placeholder) as the section thumbnail.
fn character_title_section<'a, B>(character: &'a Character, body: B) -> CreateContainerComponent<'a>
where
    B: Into<Cow<'a, str>>,
{
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
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "each bool is the independent enabled state of one of the message's buttons"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is an independent piece of the rendered button row's state"
)]
fn create_buttons<'a>(
    id: u64,
    finished: bool,
    previous: bool,
    edit: bool,
    speakable: bool,
    stoppable: bool,
    continuable: bool,
    options: &[CharacterOption],
    voices: &[VoiceEntry],
) -> Cow<'a, [CreateContainerComponent<'a>]> {
    let prev_msg_id = InteractionKind::Previous.custom_id(id);
    let next_msg_id = InteractionKind::Next.custom_id(id);
    let edit_msg_id = InteractionKind::Edit.custom_id(id);
    let undo_id = InteractionKind::Undo.custom_id(id);
    let redo_id = InteractionKind::Redo.custom_id(id);
    let pin_id = InteractionKind::Pin.custom_id(id);
    let tts_id = InteractionKind::Tts.custom_id(id);
    let stop_id = InteractionKind::Stop.custom_id(id);
    let continue_id = InteractionKind::Continue.custom_id(id);
    let char_id = InteractionKind::Character.custom_id(id);
    let voice_id = InteractionKind::Voice.custom_id(id);

    // one slot holds the live Stop while the reply streams, then becomes the
    // Continue button once it finishes: enabled when the reply can be extended,
    // disabled (e.g. at the character limit) otherwise
    let stop_or_continue = if finished {
        emoji_button(continue_id, CONTINUE).disabled(!continuable)
    } else {
        emoji_button(stop_id, STOP).disabled(!stoppable)
    };

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
                emoji_button(tts_id, SPEAK).disabled(!finished || !speakable),
                stop_or_continue,
            ]
            .into(),
        )),
    ];
    if !options.is_empty() {
        components.push(character_select_menu(char_id, options, finished));
    }
    if !voices.is_empty() {
        components.push(voice_select_menu(voice_id, voices, finished, speakable));
    }
    components.into()
}

/// Builds the "reply as another character" hand-off select menu keyed on `char_id`.
fn character_select_menu<'a>(
    char_id: String,
    options: &[CharacterOption],
    finished: bool,
) -> CreateContainerComponent<'a> {
    CreateContainerComponent::ActionRow(CreateActionRow::SelectMenu(
        CreateSelectMenu::new(
            char_id,
            CreateSelectMenuKind::String {
                options: options
                    .iter()
                    .map(|option| {
                        let reaction = option.emoji().and_then(reaction_from);
                        // a real emoji goes in the native slot; free-text in the
                        // "emoji" field is prepended to the label instead, since
                        // Discord rejects non-emoji in the emoji slot.
                        let label = match (option.emoji(), &reaction) {
                            (Some(text), None) => {
                                ellipsize(&format!("{text} {}", option.name()), SELECT_OPTION_LIMIT)
                            }
                            _ => option.name().to_owned(),
                        };
                        let mut menu_option =
                            CreateSelectMenuOption::new(label, option.id().to_owned())
                                .description(option.description().to_owned());
                        if let Some(emoji) = reaction {
                            menu_option = menu_option.emoji(emoji);
                        }
                        menu_option
                    })
                    .collect(),
            },
        )
        .placeholder("Svara som…")
        .disabled(!finished),
    ))
}

/// Builds the read-aloud voice select menu keyed on `voice_id`.
fn voice_select_menu<'a>(
    voice_id: String,
    voices: &[VoiceEntry],
    finished: bool,
    speakable: bool,
) -> CreateContainerComponent<'a> {
    CreateContainerComponent::ActionRow(CreateActionRow::SelectMenu(
        CreateSelectMenu::new(
            voice_id,
            CreateSelectMenuKind::String {
                options: voice_options(voices).into(),
            },
        )
        .placeholder("Läs upp som…")
        .disabled(!finished || !speakable),
    ))
}

/// Builds the voice dropdown's options: a leading "automatic" entry followed by
/// one entry per palette voice, each carrying its emoji and description.
fn voice_options<'a>(voices: &[VoiceEntry]) -> Vec<CreateSelectMenuOption<'a>> {
    let mut options = vec![
        CreateSelectMenuOption::new(read_aloud::AUTO_LABEL, read_aloud::AUTO_VALUE)
            .description("Välj röst(er) automatiskt utifrån innehållet")
            .emoji(unicode_emoji(AUTO_VOICE)),
    ];
    options.extend(voices.iter().map(|voice| {
        let mut option = CreateSelectMenuOption::new(voice.name.clone(), voice.voice_id.clone())
            .description(ellipsize(&voice.description, SELECT_OPTION_LIMIT));
        if let Some(emoji) = reaction_from(&voice.emoji) {
            option = option.emoji(emoji);
        }
        option
    }));
    options
}

/// Tests for the response rendering: footer, body, and the error container.
#[cfg(test)]
mod tests {
    use crate::constants::{CHARACTER_LIMIT, ERROR_COLOUR, ERROR_HEADING};
    use crate::models::{character::Character, history::History, message::Message};
    use core::time::Duration;
    use nonempty::NonEmpty;
    use serenity::all::{Color, MessageId, UserId};

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
            character,
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

    /// The footer surfaces a ranked character's name-similarity between the page
    /// counter and the timing.
    #[test]
    fn footer_includes_the_similarity_when_ranked() {
        let mut ranked = Character::rank_by_similarity(vec![character()], "Harry");
        assert_eq!(
            ranked.len(),
            1,
            "ranking returns the single visible character"
        );
        let Some(character) = ranked.pop() else {
            return;
        };
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(&character, "hello", 2.0)))
            .build();
        assert_eq!(
            history.footer_text(&character, 5, None),
            "-# 1/1 | 100% namnlikhet | tog 2.0s | 5/3900",
            "an exact name match shows the similarity between the page counter and the timing"
        );
    }

    /// A reply with content renders that content as its body.
    #[test]
    fn body_text_uses_the_content_when_present() {
        let character = character();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(&character, "hello", 1.0)))
            .build();
        assert_eq!(
            history.body_text("hello"),
            "hello",
            "present content is shown as-is"
        );
    }

    /// An empty choice with no prior turns is the seeded skip-greeting option,
    /// shown with the hint explaining what selecting it does.
    #[test]
    fn body_text_shows_skip_greeting_hint_without_prior_turns() {
        let character = character();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(&character, "", 0.0)))
            .build();
        assert_eq!(
            history.body_text(""),
            "(ingen hälsning, ditt meddelande blir det första)",
            "an empty greeting choice explains the skip-greeting option"
        );
    }

    /// Whitespace-only content is treated as no content at all, so it renders the
    /// placeholder rather than a text display Discord rejects.
    #[test]
    fn body_text_treats_whitespace_as_empty() {
        let character = character();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(&character, " \n ", 0.0)))
            .previous(vec![Message::new_user("Bob", "hej")])
            .build();
        assert_eq!(
            history.body_text(" \n "),
            "…",
            "whitespace-only content falls back to the placeholder glyph"
        );
    }

    /// A body with leading and interior blank lines renders only the lines that
    /// carry text, since Discord rejects a whitespace-only text display.
    #[test]
    fn body_lines_skip_blank_lines() {
        let character = character();
        let card = super::title_and_body_lines(&character, "\n  \nhej\n \nsvejs\n");
        assert_eq!(
            card.len(),
            2,
            "only the title section and the second line of text are emitted"
        );
        let json = serde_json::to_string(&card).unwrap_or_default();
        assert!(
            json.contains("hej") && json.contains("svejs"),
            "both non-blank lines survive"
        );
    }

    /// A failed-generation choice renders as the red error container, keeping the
    /// buttons live, instead of as the character speaking.
    #[test]
    fn render_components_draws_a_failed_choice_as_an_error() {
        let character = character();
        let mut history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(&character, "trasigt svar", 1.0)))
            .build();
        history.set_current_choice_error();
        let container = history.render_components(&character, 1, None, true, &[], &[]);
        let json = serde_json::to_string(&container).unwrap_or_default();
        assert!(
            json.contains(ERROR_HEADING),
            "a failed choice shows the error heading"
        );
        assert!(
            json.contains(&ERROR_COLOUR.to_string()),
            "the error container carries the danger accent colour"
        );
        assert!(
            json.contains("1prev") && json.contains("1next"),
            "the navigation buttons stay live so the user can swipe to retry"
        );
        assert!(
            !json.contains("## Harry"),
            "an error is not rendered as the character speaking"
        );
    }

    /// A normal choice renders as the character card, not the error container.
    #[test]
    fn render_components_draws_a_normal_choice_as_the_character() {
        let character = character();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(&character, "hej", 1.0)))
            .build();
        let container = history.render_components(&character, 1, None, true, &[], &[]);
        let json = serde_json::to_string(&container).unwrap_or_default();
        assert!(
            json.contains("## Harry"),
            "a normal reply is rendered with the character title"
        );
        assert!(
            !json.contains(ERROR_HEADING),
            "a normal reply has no error heading"
        );
    }

    /// A normal reply carries the character's accent colour when one is set.
    #[test]
    fn render_components_applies_the_character_accent_colour() {
        let value: u32 = 0x00ab_cdef;
        let mut character = character();
        character.set_color(Color::new(value));
        let history = finished_reply(&character, "hej");
        let container = history.render_components(&character, 1, None, true, &[], &[]);
        let json = serde_json::to_string(&container).unwrap_or_default();
        assert!(
            json.contains(&value.to_string()),
            "a normal reply carries the character's accent colour"
        );
    }

    /// A reply for a character with no colour set carries no accent colour.
    #[test]
    fn render_components_omits_the_accent_colour_when_unset() {
        let character = character();
        let history = finished_reply(&character, "hej");
        let container = history.render_components(&character, 1, None, true, &[], &[]);
        let json = serde_json::to_string(&container).unwrap_or_default();
        assert!(
            !json.contains("accent_color"),
            "a colourless character renders without an accent colour"
        );
    }

    /// The streaming placeholder carries the character's accent colour when one
    /// is set, so it matches the finished reply instead of flickering.
    #[test]
    fn placeholder_applies_the_character_accent_colour() {
        let value: u32 = 0x00ab_cdef;
        let mut character = character();
        character.set_color(Color::new(value));
        let history = finished_reply(&character, "hej");
        let container =
            history.placeholder_components(&character, 1, Duration::ZERO, true, &[], &[]);
        let json = serde_json::to_string(&container).unwrap_or_default();
        assert!(
            json.contains(&value.to_string()),
            "the placeholder carries the character's accent colour"
        );
    }

    /// The placeholder omits the accent colour for a colourless character.
    #[test]
    fn placeholder_omits_the_accent_colour_when_unset() {
        let character = character();
        let history = finished_reply(&character, "hej");
        let container =
            history.placeholder_components(&character, 1, Duration::ZERO, false, &[], &[]);
        let json = serde_json::to_string(&container).unwrap_or_default();
        assert!(
            !json.contains("accent_color"),
            "a colourless placeholder has no accent colour"
        );
    }

    /// Builds a finished single-reply history with the given content.
    fn finished_reply(character: &Character, content: &str) -> History {
        History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(character, content, 1.0)))
            .build()
    }

    /// Reports whether the button keyed on `custom_id` is present in `history`'s
    /// rendered components and, if so, whether it is disabled.
    fn button_disabled(history: &History, character: &Character, custom_id: &str) -> Option<bool> {
        let value =
            serde_json::to_value(history.render_components(character, 1, None, true, &[], &[]))
                .unwrap_or_default();
        find_disabled(&value, custom_id)
    }

    /// Recursively searches `value` for a button object carrying `custom_id`,
    /// returning its `disabled` flag (absent meaning enabled), or `None` when no
    /// such button exists.
    fn find_disabled(value: &serde_json::Value, custom_id: &str) -> Option<bool> {
        if let Some(object) = value.as_object() {
            if object.get("custom_id").and_then(serde_json::Value::as_str) == Some(custom_id) {
                return Some(
                    object
                        .get("disabled")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                );
            }
            return object
                .values()
                .find_map(|nested| find_disabled(nested, custom_id));
        }
        value
            .as_array()?
            .iter()
            .find_map(|nested| find_disabled(nested, custom_id))
    }

    /// A finished, continuable reply renders an enabled Continue button in the
    /// Stop slot.
    #[test]
    fn continue_button_enabled_on_a_finished_reply() {
        let character = character();
        let history = finished_reply(&character, "hej");
        assert_eq!(
            button_disabled(&history, &character, "1cont"),
            Some(false),
            "a finished reply offers an enabled Continue button"
        );
        assert_eq!(
            button_disabled(&history, &character, "1stop"),
            None,
            "Continue takes over the Stop slot, so no Stop button is rendered"
        );
    }

    /// During the reveal delay (`show_continue` held false) the slot is already a
    /// Continue button, just disabled until it is revealed.
    #[test]
    fn continue_button_disabled_during_the_reveal_delay() {
        let character = character();
        let mut history = finished_reply(&character, "hej");
        history.set_show_continue(false);
        assert_eq!(
            button_disabled(&history, &character, "1cont"),
            Some(true),
            "the Continue button is present but disabled until it is revealed"
        );
        assert_eq!(
            button_disabled(&history, &character, "1stop"),
            None,
            "no Stop button is rendered during the reveal delay"
        );
    }

    /// A reply at the character limit shows a disabled Continue button: there is
    /// nothing left to extend, but the slot still reads as Continue.
    #[test]
    fn continue_button_disabled_at_the_character_limit() {
        let character = character();
        let history = finished_reply(&character, &"x".repeat(CHARACTER_LIMIT));
        assert_eq!(
            button_disabled(&history, &character, "1cont"),
            Some(true),
            "a reply at the limit shows Continue as disabled rather than hiding it"
        );
        assert_eq!(
            button_disabled(&history, &character, "1stop"),
            None,
            "the maxed-out reply keeps no Stop button in the slot"
        );
    }

    /// A reply still streaming shows the live Stop button, not Continue.
    #[test]
    fn stop_button_shown_while_streaming() {
        let character = character();
        let mut history = finished_reply(&character, "hej");
        history.set_finished(false);
        assert_eq!(
            button_disabled(&history, &character, "1stop"),
            Some(false),
            "a streaming reply shows the live Stop button"
        );
        assert_eq!(
            button_disabled(&history, &character, "1cont"),
            None,
            "Continue only appears once the reply has finished"
        );
    }

    /// A failed reply shows a disabled Continue button (it is not continuable),
    /// not a Stop button.
    #[test]
    fn continue_button_disabled_on_an_error_reply() {
        let character = character();
        let mut history = finished_reply(&character, "trasigt");
        history.set_current_choice_error();
        assert_eq!(
            button_disabled(&history, &character, "1cont"),
            Some(true),
            "an error reply is not continuable, so Continue is disabled"
        );
        assert_eq!(
            button_disabled(&history, &character, "1stop"),
            None,
            "an error reply renders no Stop button in the slot"
        );
    }

    /// Reports the disabled state of the select menu keyed on `custom_id` in a
    /// render that carries one hand-off option, or `None` when it is absent.
    fn select_disabled(history: &History, character: &Character, custom_id: &str) -> Option<bool> {
        let options = [character.to_menu_option()];
        let value = serde_json::to_value(history.render_components(
            character,
            1,
            None,
            true,
            &[],
            &options,
        ))
        .unwrap_or_default();
        find_disabled(&value, custom_id)
    }

    /// While the reply is still streaming, the "Svara som" hand-off dropdown is
    /// disabled just like the "Läs upp som" voice dropdown.
    #[test]
    fn handoff_dropdown_disabled_while_streaming() {
        let character = character();
        let mut history = finished_reply(&character, "hej");
        history.set_finished(false);
        assert_eq!(
            select_disabled(&history, &character, "1char"),
            Some(true),
            "the hand-off dropdown is disabled while the reply streams"
        );
    }

    /// Once the reply has finished, the "Svara som" hand-off dropdown is enabled.
    #[test]
    fn handoff_dropdown_enabled_when_finished() {
        let character = character();
        let history = finished_reply(&character, "hej");
        assert_eq!(
            select_disabled(&history, &character, "1char"),
            Some(false),
            "the hand-off dropdown is enabled on a finished reply"
        );
    }

    /// A response rendered before its message ID is known disables every
    /// control, so nothing can be pressed while the buttons are still keyed on
    /// the placeholder ID.
    #[test]
    fn pending_render_disables_every_control() {
        let character = character();
        let history = finished_reply(&character, "hej");
        let options = [character.to_menu_option()];
        let value = serde_json::to_value(history.render_components(
            &character,
            1,
            None,
            false,
            &[],
            &options,
        ))
        .unwrap_or_default();
        for custom_id in [
            "1prev", "1next", "1undo", "1redo", "1edit", "1pinn", "1tala", "1stop", "1char",
        ] {
            assert_eq!(
                find_disabled(&value, custom_id),
                Some(true),
                "{custom_id} is disabled until the real message ID is known"
            );
        }
    }

    /// An empty choice mid-conversation is a reply stopped before its first
    /// token, shown as the "…" placeholder rather than the skip-greeting hint.
    #[test]
    fn body_text_shows_ellipsis_for_a_stopped_reply() {
        let character = character();
        let history = History::builder()
            .id(MessageId::new(1))
            .character("id")
            .choices(NonEmpty::new(timed_choice(&character, "", 0.0)))
            .previous(vec![Message::new_user("Bob", "hej")])
            .build();
        assert_eq!(
            history.body_text(""),
            "…",
            "a reply stopped before its first token shows the placeholder glyph"
        );
    }
}
