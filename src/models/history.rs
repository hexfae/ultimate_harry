//! The history model: a conversation between a user and a character.
//!
//! A new [`History`] is saved per bot reply, keyed by that reply's Discord message ID, so a button
//! interaction or a user reply can find the conversation from the message it acted on.
//!
//! ## Normalized storage
//!
//! History is stored in two pieces so the same message is never written twice and the derivable
//! scaffolding is never written at all:
//!
//! - Each [`Message`] lives once in its own `native_db` table, keyed by its ID.
//! - [`StoredHistory`] (the persisted form) holds only ordered ID lists: `previous` (the chat
//!   context) and `choices` (the swipeable replies for this turn).
//! - The system-prompt scaffolding (the Swedish roleplay framing built by [`scaffolding`]) is not
//!   stored; it is rebuilt from the character at request time. Characters are versioned and a
//!   history references a specific version, so the rebuilt scaffolding always matches.
//!
//! This replaced an older design where every history stored the whole conversation (plus
//! scaffolding) inline, which duplicated messages across the many histories of one conversation and
//! grew quadratically.
//!
//! ## In-memory vs stored
//!
//! [`History`] (this in-memory form, used by the chat loop, interaction handlers, and rendering)
//! differs from [`StoredHistory`]:
//!
//! - `choices` are hydrated into full [`Message`]s, because the swipe/edit/undo/redo/regenerate
//!   handlers operate on them directly. Rendering also only needs `choices` + the character.
//! - `previous` stays as IDs; the full messages are resolved (and the scaffolding prepended) only
//!   when building the LLM context, via [`Database::build_context`](crate::database::Database::build_context).
//! - `pending` buffers messages pushed this turn so they can be written to the message table on the
//!   next save.
//!
//! [`History::into_stored`] (on save) and [`History::hydrate`] (on load) bridge the two forms;
//! `Database::history` / `Database::upsert_history` perform the message-table resolution.

use alloc::borrow::Cow;
use bon::Builder;
use core::time::Duration;
use native_db::{ToKey as _, native_db};
use native_model::{Model as _, native_model};
use nonempty::NonEmpty;
use poise::CreateReply;
use serde::{Deserialize, Serialize};
use serenity::all::{
    CreateActionRow, CreateAllowedMentions, CreateComponent, CreateContainer,
    CreateContainerComponent, CreateEmbed, CreateEmbedFooter, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateMessage, CreateSection, CreateSectionAccessory,
    CreateSectionComponent, CreateSelectMenu, CreateSelectMenuKind, CreateSelectMenuOption,
    CreateTextDisplay, CreateThumbnail, CreateUnfurledMediaItem, EditInteractionResponse,
    EditMessage, Message as DiscordMessage, MessageFlags, MessageId, UserId,
};

use crate::{
    components::emoji_button,
    constants::{CHARACTER_LIMIT, EDIT, NEXT, PIN, PREVIOUS, REDO, UNDO},
    database::Database,
    events::interaction::InteractionKind,
    models::{
        character::{Character, CharacterOption},
        message::Message,
    },
};

/// The system message that precedes every conversation.
const SYSTEM_MESSAGE: &str = "Du kommer nu att gå in i ett rollspel med en användare. Under inga omständigheter får du bryta rollspelet, gå ur karaktär, eller prata åt användaren.";

/// The system message that precedes the bot's personality.
const BEGIN_PERSONALITY: &str = "Beskriv nu karaktären du ska rollspela som.";

/// The system message that precedes the prompt.
const BEGIN_PROMPT: &str = "Detta är dina instruktioner som du ska följa under hela rollspelet: ";

/// The system message that precedes the scenario.
const BEGIN_SCENARIO: &str = "Detta är scenen du och användaren finner er själva i: ";

/// The system message that precedes the example messages.
const BEGIN_EXAMPLE_MESSAGES: &str = "Det följande är exempel på hur du ska prata med användaren.";

/// The system message that gets placed between every example message.
const EXAMPLE_MESSAGE_SEPARATOR: &str = "Nytt exempelmeddelande.";

/// The system that precedes the conversation actually beginning.
const BEGIN_MESSAGE: &str = "Rollspelet börjar nu. Efter denna punkt får du inte längra avbryta rollspelet, gå ur karaktär, eller skriva åt användaren.";

/// An empty avatar, a transparent 1x1 PNG.
///
/// Used because Discord sections require an accessory, but a character may not have an avatar set.
const EMPTY_AVATAR: &str = "https://upload.wikimedia.org/wikipedia/commons/c/ca/1x1.png";

/// A log of messages between the user and a character.
///
/// This is the in-memory form used by the chat loop, interaction handlers, and rendering.
/// It is persisted as a [`StoredHistory`] (which holds message IDs, not inline messages); the
/// system-prompt scaffolding is never stored, but rebuilt from the character via [`scaffolding`].
#[derive(Debug, Clone, Builder)]
pub struct History {
    /// The Discord Message ID of this history.
    #[builder(with = |id: MessageId| id.to_string())]
    id: String,
    /// The ulid ID of the currently responding character.
    #[builder(with = |id: &str| id.to_owned())]
    character: String,
    /// The current responses the user can pick between by "swiping" (pressing next/previous).
    choices: NonEmpty<Message>,
    /// The index of the current response the user has chosen.
    #[builder(default)]
    current: usize,
    /// If the streaming has finished for this history. Transient, never stored.
    ///
    /// This is used to create embeds while streaming a response.
    #[builder(default = true)]
    has_finished: bool,
    /// The IDs of the previous messages (the chat context), resolved from the message table on
    /// demand. Contains no scaffolding.
    #[builder(default)]
    previous: Vec<String>,
    /// Messages introduced this turn that must be persisted on save. Not part of the stored shape.
    #[builder(default)]
    pending: Vec<Message>,
}

/// The persisted form of a [`History`]: only message IDs, no inline messages and no scaffolding.
#[derive(Debug, Serialize, Deserialize)]
#[native_model(id = 2, version = 1, with = crate::codec::Json)]
#[native_db]
#[expect(
    clippy::module_name_repetitions,
    reason = "StoredHistory is the persisted counterpart of History and belongs in this module"
)]
pub struct StoredHistory {
    /// The Discord Message ID of this history, used as the primary key.
    #[primary_key]
    pub id: String,
    /// The ulid ID of the currently responding character.
    pub character: String,
    /// The IDs of the previous messages (the chat context), in order.
    pub previous: Vec<String>,
    /// The IDs of the current swipeable responses.
    pub choices: Vec<String>,
    /// The index of the current chosen response.
    pub current: usize,
}

/// Builds the system-prompt scaffolding messages for a character.
///
/// These frame the roleplay (system message, personality, prompt, scenario, example messages,
/// system prompt) and are rebuilt from the character at request time rather than stored. The
/// generated message IDs are throwaway, since scaffolding is never persisted.
pub fn scaffolding(character: &Character) -> Vec<Message> {
    let mut messages = vec![Message::new_system(SYSTEM_MESSAGE)];

    if let Some(personality) = character.personality() {
        messages.push(Message::new_system(BEGIN_PERSONALITY));
        messages.push(Message::new_assistant(personality, character));
    }

    if let Some(prompt) = character.prompt() {
        let mut begin_prompt = BEGIN_PROMPT.to_owned();
        begin_prompt.push_str(prompt);
        messages.push(Message::new_system(begin_prompt));
    }

    if let Some(scenario) = character.scenario() {
        let mut begin_scenario = BEGIN_SCENARIO.to_owned();
        begin_scenario.push_str(scenario);
        messages.push(Message::new_system(begin_scenario));
    }

    if !character.example_messages().is_empty() {
        messages.push(Message::new_system(BEGIN_EXAMPLE_MESSAGES));
        for (user_message, assistant_message) in character.example_messages() {
            if let Some(content) = user_message {
                messages.push(Message::new_user("Användaren", content));
            }
            messages.push(Message::new_assistant(assistant_message, character));
            messages.push(Message::new_system(EXAMPLE_MESSAGE_SEPARATOR));
        }
    }

    if let Some(system_prompt) = character.system_prompt() {
        messages.push(Message::new_system(system_prompt));
    }

    messages.push(Message::new_system(BEGIN_MESSAGE));
    messages
}

/// A log of messages between the user and a character.
impl History {
    /// Edits the content of the current choice message.
    pub fn edit_content<A, C, E>(&mut self, author: A, content: C, editor: Option<E>)
    where
        A: Into<String>,
        C: Into<String>,
        E: Into<UserId>,
    {
        if let Some(choice) = self.choices.get_mut(self.current) {
            choice.edit(author, content, editor);
        }
    }

    /// Sets whether the history has finished generating.
    pub const fn set_finished(&mut self, has_finished: bool) {
        self.has_finished = has_finished;
    }

    /// Shows the previous choice by cycling the current index backward.
    pub fn previous(&mut self) {
        self.current = self
            .current
            .saturating_add(self.choices.len())
            .saturating_sub(1)
            .strict_rem(self.choices.len());
    }

    /// Shows the next choice by cycling the current index forward.
    ///
    /// Callers must ensure this is not invoked on the last choice; advancing off
    /// the end is handled separately as a swipe-to-generate.
    pub const fn next(&mut self) {
        self.current = self.current.saturating_add(1);
    }

    /// Returns the current choice index.
    #[must_use]
    pub const fn current_choice(&self) -> usize {
        self.current
    }

    /// Returns the number of choices.
    #[must_use]
    pub fn choices_count(&self) -> usize {
        self.choices.len()
    }

    /// Returns true if there are multiple choices of this history.
    fn has_multiple_choices(&self) -> bool {
        self.choices.len() > 1
    }

    /// Returns true if the currently chosen message has one or more edits.
    fn chosen_has_edit(&self) -> bool {
        self.chosen_message().revisions_count() > 0
    }

    /// Returns whether the user is on the last choice.
    #[must_use]
    pub fn is_on_last_choice(&self) -> bool {
        self.current_choice().saturating_add(1) >= self.choices_count()
    }

    /// Undoes the current message edit by showing the previous revision.
    pub fn undo(&mut self) {
        if let Some(message) = self.choices.get_mut(self.current) {
            message.undo();
        }
    }

    /// Redoes the current message edit by showing the next revision.
    pub fn redo(&mut self) {
        if let Some(message) = self.choices.get_mut(self.current) {
            message.redo();
        }
    }

    /// Returns the currently chosen message.
    #[must_use]
    pub fn chosen_message(&self) -> &Message {
        self.choices
            .get(self.current)
            .unwrap_or_else(|| self.choices.first())
    }

    /// Returns the Discord message ID of this history.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Sets the Discord message ID of this history.
    pub fn set_id<M: Into<MessageId>>(&mut self, id: M) {
        self.id = id.into().to_string();
    }

    /// Returns the character ID of the currently responding character.
    #[must_use]
    pub fn character(&self) -> &str {
        &self.character
    }

    /// Sets the character ID of the currently responding character.
    pub fn set_character(&mut self, character: String) {
        self.character = character;
    }

    /// Appends a message to the chat context and queues it for persistence on the next save.
    pub fn push<M: Into<Message>>(&mut self, value: M) {
        let message = value.into();
        self.previous.push(message.id().to_owned());
        self.pending.push(message);
    }

    /// Resets the choices, removing all but the first choice, and resets the current index to 0.
    pub fn reset_choices(&mut self) {
        self.choices.tail.clear();
        self.current = 0;
    }

    /// Sets the choices and resets the current index to 0.
    pub fn set_choices<M: Into<Message>>(&mut self, choices: M) {
        self.choices = NonEmpty::new(choices.into());
        self.current = 0;
    }

    /// Pushes a new choice and sets the current index to that choice.
    pub fn push_choice<M: Into<Message>>(&mut self, choice: M) {
        self.choices.push(choice.into());
        self.current = self.choices_count().saturating_sub(1);
    }

    /// Updates the current choice in place.
    pub fn update_current_choice<M: Into<Message>>(&mut self, choice: M) {
        if let Some(slot) = self.choices.get_mut(self.current) {
            *slot = choice.into();
        }
    }

    /// Begins a new reply turn: the chosen reply and the user message join the
    /// context, the old swipeable choices are cleared, and the reply is marked
    /// as not yet finished.
    ///
    /// An empty chosen reply (the "skip the greeting" choice) is omitted from the context so the
    /// user message becomes the first turn, rather than injecting a blank turn.
    pub fn begin_new_turn<M: Into<Message>>(&mut self, user_message: M) {
        if !self
            .chosen_message()
            .chosen_revision()
            .head()
            .content()
            .is_empty()
        {
            self.push(self.chosen_message().to_owned());
        }
        self.push(user_message);
        self.reset_choices();
        self.set_finished(false);
    }

    /// Hands the conversation to another character: the chosen reply joins the
    /// context, the old choices are cleared, the responding character switches,
    /// the hand-off system message joins the context, and the reply is marked as
    /// not yet finished.
    pub fn begin_handoff<M: Into<Message>>(&mut self, character: String, system_message: M) {
        self.push(self.chosen_message().to_owned());
        self.reset_choices();
        self.set_character(character);
        self.push(system_message);
        self.set_finished(false);
    }

    /// Returns the IDs of the previous messages (the chat context), in order.
    #[must_use]
    pub fn previous_ids(&self) -> &[String] {
        &self.previous
    }

    /// Returns the messages queued for persistence (those pushed this turn, not yet in the table).
    #[must_use]
    pub fn pending(&self) -> &[Message] {
        &self.pending
    }

    /// Converts the in-memory history into its persisted form plus the messages that must be
    /// written to the message table (the queued `pending` messages and the current choices).
    #[must_use]
    pub fn into_stored(self) -> (StoredHistory, Vec<Message>) {
        let choices = self
            .choices
            .iter()
            .map(|message| message.id().to_owned())
            .collect();
        let mut messages = self.pending;
        messages.extend(self.choices);
        let stored = StoredHistory {
            id: self.id,
            character: self.character,
            previous: self.previous,
            choices,
            current: self.current,
        };
        (stored, messages)
    }

    /// Rebuilds an in-memory history from its persisted form and its resolved choice messages.
    #[must_use]
    pub fn hydrate(stored: StoredHistory, choices: NonEmpty<Message>) -> Self {
        // choices can be shorter than the stored list if some message records failed to resolve,
        // so clamp the chosen index to what is actually available rather than dangling past the end
        let current = stored.current.min(choices.len().saturating_sub(1));
        Self {
            id: stored.id,
            character: stored.character,
            choices,
            current,
            has_finished: true,
            previous: stored.previous,
            pending: Vec::new(),
        }
    }

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
    #[must_use]
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

        let components = create_buttons(
            id.into().into(),
            self.has_finished,
            self.has_multiple_choices(),
            self.chosen_has_edit(),
            options,
        );

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

/// Creates a new [`History`] from a character and message ID.
///
/// The conversation starts empty (the scaffolding is derived at request time). Two swipe choices
/// are seeded: the greeting (shown by default) and an empty "skip the greeting" choice, so the user
/// can swipe to start the conversation with their own message first, leaving the character unprimed.
impl From<(&Character, MessageId)> for History {
    fn from((character, id): (&Character, MessageId)) -> Self {
        let greeting = Message::new_assistant(character.greeting(), character);
        let skip_greeting = Message::new_assistant("", character);
        let choices = NonEmpty {
            head: greeting,
            tail: vec![skip_greeting],
        };

        Self::builder()
            .choices(choices)
            .character(character.id())
            .id(id)
            .build()
    }
}

/// Tests for in-memory history navigation.
#[cfg(test)]
mod tests {
    use super::{
        BEGIN_EXAMPLE_MESSAGES, BEGIN_MESSAGE, History, SYSTEM_MESSAGE, StoredHistory, scaffolding,
    };
    use crate::models::{character::Character, message::Message};
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

    /// Returns the content of a scaffolding message's first part.
    fn first_part_content(message: &Message) -> &str {
        message.chosen_revision().head().content()
    }

    /// Builds a history with `count` swipeable choices.
    fn history_with_choices(count: usize) -> History {
        let mut choices = NonEmpty::new(Message::new_system("choice"));
        for _ in 1..count {
            choices.push(Message::new_system("choice"));
        }
        History::builder()
            .id(MessageId::new(1))
            .character("character-id")
            .choices(choices)
            .build()
    }

    /// `previous` steps the current index backward and wraps past the first choice.
    #[test]
    fn previous_cycles_backward_and_wraps() {
        let mut history = history_with_choices(3);
        assert_eq!(
            history.current_choice(),
            0,
            "a fresh history starts on the first choice"
        );
        history.previous();
        assert_eq!(
            history.current_choice(),
            2,
            "previous on the first choice wraps to the last"
        );
        history.previous();
        assert_eq!(
            history.current_choice(),
            1,
            "previous steps backward by one"
        );
        history.previous();
        assert_eq!(
            history.current_choice(),
            0,
            "previous returns to the first choice"
        );
    }

    /// `is_on_last_choice` is true only when the current index is the final choice.
    #[test]
    fn last_choice_detected_at_the_end() {
        let mut history = history_with_choices(3);
        assert!(
            !history.is_on_last_choice(),
            "the first of three choices is not the last"
        );
        history.push_choice(Message::new_system("pushed"));
        assert!(
            history.is_on_last_choice(),
            "a pushed choice becomes the current and last choice"
        );
        history.previous();
        assert!(
            !history.is_on_last_choice(),
            "stepping back from the last choice is no longer last"
        );
    }

    /// A single-choice history is always on its last (and only) choice.
    #[test]
    fn single_choice_is_always_last() {
        let history = history_with_choices(1);
        assert!(
            history.is_on_last_choice(),
            "the only choice is also the last choice"
        );
    }

    /// `into_stored` emits ID lists plus pending-then-choice messages, and `hydrate` rebuilds
    /// the in-memory history from them unchanged.
    #[test]
    fn into_stored_then_hydrate_preserves_history() {
        let choice_one = Message::new_system("choice one");
        let choice_two = Message::new_system("choice two");
        let pending_one = Message::new_user("Alice", "hello");
        let choice_ids = vec![choice_one.id().to_owned(), choice_two.id().to_owned()];
        let choice_two_id = choice_two.id().to_owned();
        let pending_id = pending_one.id().to_owned();
        let previous_ids = vec!["prev-a".to_owned(), "prev-b".to_owned()];

        let mut choices = NonEmpty::new(choice_one.clone());
        choices.push(choice_two.clone());
        let mut hydrate_choices = NonEmpty::new(choice_one);
        hydrate_choices.push(choice_two);

        let history = History::builder()
            .id(MessageId::new(42))
            .character("character-id")
            .choices(choices)
            .current(1_usize)
            .previous(previous_ids.clone())
            .pending(vec![pending_one])
            .build();

        let (stored, messages) = history.into_stored();
        assert_eq!(stored.id, "42", "the message ID is preserved as the key");
        assert_eq!(
            stored.character, "character-id",
            "the character ID is preserved"
        );
        assert_eq!(
            stored.choices, choice_ids,
            "stored choices are the choice IDs in order"
        );
        assert_eq!(
            stored.previous, previous_ids,
            "stored previous are the context IDs in order"
        );
        assert_eq!(stored.current, 1, "the chosen index is preserved");

        let message_ids = messages
            .iter()
            .map(|message| message.id().to_owned())
            .collect::<Vec<String>>();
        let mut expected_ids = vec![pending_id];
        expected_ids.extend(choice_ids.iter().cloned());
        assert_eq!(
            message_ids, expected_ids,
            "into_stored writes pending messages first, then the choices"
        );

        let hydrated = History::hydrate(stored, hydrate_choices);
        assert_eq!(hydrated.id(), "42", "hydrate restores the message ID");
        assert_eq!(
            hydrated.character(),
            "character-id",
            "hydrate restores the character ID"
        );
        assert_eq!(
            hydrated.current_choice(),
            1,
            "hydrate restores the chosen index"
        );
        assert_eq!(
            hydrated.previous_ids(),
            previous_ids.as_slice(),
            "hydrate restores the context IDs"
        );
        assert!(
            hydrated.pending().is_empty(),
            "a freshly hydrated history has nothing pending"
        );
        assert_eq!(
            hydrated.chosen_message().id(),
            choice_two_id.as_str(),
            "the chosen index points at the second choice"
        );
    }

    /// When some stored choices fail to resolve, the rehydrated choice list is shorter than the
    /// stored `current` index, so hydrate must clamp it to the last available choice rather than
    /// leaving it dangling past the end.
    #[test]
    fn hydrate_clamps_current_past_the_available_choices() {
        let stored = StoredHistory {
            id: "42".to_owned(),
            character: "character-id".to_owned(),
            previous: Vec::new(),
            choices: vec!["a".to_owned(), "b".to_owned(), "c".to_owned()],
            current: 2,
        };
        let mut choices = NonEmpty::new(Message::new_system("only choice"));
        choices.push(Message::new_system("second choice"));

        let hydrated = History::hydrate(stored, choices);
        assert_eq!(
            hydrated.current_choice(),
            1,
            "current is clamped to the last resolvable choice, not the stored index"
        );
    }

    /// A character with no optional fields scaffolds to just the framing system message and the
    /// begin-message marker.
    #[test]
    fn scaffolding_is_minimal_for_a_bare_character() {
        let character = Character::builder()
            .id("character-id".to_owned())
            .name("Harry")
            .greeting("hello")
            .creator(UserId::new(1))
            .build();
        let messages = scaffolding(&character);
        assert_eq!(
            messages.len(),
            2,
            "a bare character scaffolds to two system messages"
        );
        assert_eq!(
            messages.first().map(first_part_content),
            Some(SYSTEM_MESSAGE),
            "the first scaffolding message frames the roleplay"
        );
        assert_eq!(
            messages.last().map(first_part_content),
            Some(BEGIN_MESSAGE),
            "the last scaffolding message marks the start of the conversation"
        );
    }

    /// Every optional field plus an example pair contributes its scaffolding messages in order.
    #[test]
    fn scaffolding_expands_with_optional_fields() {
        let character = Character::builder()
            .id("character-id".to_owned())
            .name("Harry")
            .greeting("hello")
            .creator(UserId::new(1))
            .personality("personality".to_owned())
            .prompt("prompt".to_owned())
            .scenario("scenario".to_owned())
            .system_prompt("system prompt".to_owned())
            .example_messages(vec![(Some("hi".to_owned()), "hello".to_owned())])
            .build();
        let messages = scaffolding(&character);
        assert_eq!(
            messages.len(),
            11,
            "the framing, personality pair, prompt, scenario, example pair, system prompt and \
             begin-message marker total eleven messages"
        );
        assert_eq!(
            messages.first().map(first_part_content),
            Some(SYSTEM_MESSAGE),
            "the framing system message stays first"
        );
        assert_eq!(
            messages.last().map(first_part_content),
            Some(BEGIN_MESSAGE),
            "the begin-message marker stays last"
        );
        assert!(
            messages
                .iter()
                .any(|message| first_part_content(message) == BEGIN_EXAMPLE_MESSAGES),
            "the example messages are introduced by their marker"
        );
    }

    /// Pins the turn-begin sequence run by the message handler: pushing the
    /// chosen reply and the user message into the context, clearing the old
    /// choices, and marking the reply unfinished.
    #[test]
    fn begin_new_turn_extends_context_and_resets_choices() {
        let mut history = history_with_choices(3);
        history.previous();
        assert_eq!(
            history.current_choice(),
            2,
            "the history starts on a non-first choice to prove the reset"
        );
        let chosen_id = history.chosen_message().id().to_owned();
        let user = Message::new_user("Alice", "hello");
        let user_id = user.id().to_owned();

        history.begin_new_turn(user);

        assert_eq!(
            history.choices_count(),
            1,
            "the old swipeable choices are cleared down to one"
        );
        assert_eq!(
            history.current_choice(),
            0,
            "the current index resets to the first choice"
        );
        assert_eq!(
            history.previous_ids(),
            [chosen_id, user_id].as_slice(),
            "the chosen reply then the user message are appended to the context"
        );
        assert_eq!(
            history.pending().len(),
            2,
            "both pushed messages are queued for persistence"
        );
        assert!(
            !history.has_finished,
            "the reply is marked as not yet finished"
        );
    }

    /// Pins the hand-off sequence run by the character-select handler: pushing
    /// the chosen reply, clearing choices, switching the responding character,
    /// pushing the system prompt, and marking the reply unfinished.
    #[test]
    fn handoff_switches_character_and_resets_choices() {
        let mut history = history_with_choices(3);
        history.previous();
        let chosen_id = history.chosen_message().id().to_owned();
        let system = Message::new_user("System", "Svara nu som X.");
        let system_id = system.id().to_owned();

        history.begin_handoff("new-character-id".to_owned(), system);

        assert_eq!(
            history.character(),
            "new-character-id",
            "the responding character switches to the new one"
        );
        assert_eq!(
            history.choices_count(),
            1,
            "the old swipeable choices are cleared down to one"
        );
        assert_eq!(
            history.current_choice(),
            0,
            "the current index resets to the first choice"
        );
        assert_eq!(
            history.previous_ids(),
            [chosen_id, system_id].as_slice(),
            "the chosen reply then the hand-off system prompt are appended to the context"
        );
        assert!(
            !history.has_finished,
            "the reply is marked as not yet finished"
        );
    }

    /// A fresh history seeds two swipe choices: the greeting (a clean,
    /// unedited original shown by default) and an empty "skip the greeting" choice.
    #[test]
    fn fresh_history_seeds_a_greeting_and_skip_choice() {
        let character = character();
        let history = History::from((&character, MessageId::new(1)));
        assert_eq!(
            history.choices_count(),
            2,
            "the greeting and the skip-greeting choice are both seeded"
        );
        assert_eq!(
            history.current_choice(),
            0,
            "the greeting is shown by default"
        );
        assert_eq!(
            history.chosen_message().revisions_count(),
            0,
            "the greeting is a clean original, not a fabricated edit"
        );
    }

    /// Keeping the greeting choice includes it in the LLM context.
    #[test]
    fn keeping_the_greeting_includes_it_in_context() {
        let character = character();
        let mut history = History::from((&character, MessageId::new(1)));
        let greeting_id = history.chosen_message().id().to_owned();
        let user = Message::new_user("Alice", "hello");
        let user_id = user.id().to_owned();

        history.begin_new_turn(user);

        assert_eq!(
            history.previous_ids(),
            [greeting_id, user_id].as_slice(),
            "the greeting then the user message form the context"
        );
        assert_eq!(
            history.pending().len(),
            2,
            "both the greeting and the user message are queued"
        );
    }

    /// Swiping to the empty skip choice omits the greeting from the LLM context,
    /// so the user message becomes the first turn.
    #[test]
    fn skipping_the_greeting_omits_it_from_context() {
        let character = character();
        let mut history = History::from((&character, MessageId::new(1)));
        let user = Message::new_user("Alice", "hello");
        let user_id = user.id().to_owned();

        history.previous();
        assert_eq!(
            history.current_choice(),
            1,
            "swiping back from the greeting lands on the skip choice"
        );

        history.begin_new_turn(user);

        assert_eq!(
            history.previous_ids(),
            [user_id].as_slice(),
            "the greeting is omitted; the user message is the first context turn"
        );
        assert_eq!(
            history.pending().len(),
            1,
            "only the user message is queued"
        );
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
