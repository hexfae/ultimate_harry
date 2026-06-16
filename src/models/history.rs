//! The history model for storing chat histories between users and characters.

use alloc::borrow::Cow;
use bon::Builder;
use core::time::Duration;
use native_db::{ToKey as _, native_db};
use native_model::{Model as _, native_model};
use nonempty::NonEmpty;
use poise::CreateReply;
use serde::{Deserialize, Serialize};
use serenity::{
    all::{
        ButtonStyle, CreateActionRow, CreateAllowedMentions, CreateButton, CreateComponent,
        CreateContainer, CreateContainerComponent, CreateEmbed, CreateEmbedFooter,
        CreateInteractionResponse, CreateInteractionResponseMessage, CreateMessage, CreateSection,
        CreateSectionAccessory, CreateSectionComponent, CreateSelectMenu, CreateSelectMenuKind,
        CreateSelectMenuOption, CreateTextDisplay, CreateThumbnail, CreateUnfurledMediaItem,
        EditInteractionResponse, EditMessage, Message as DiscordMessage, MessageFlags, MessageId,
        ReactionType, UserId,
    },
    small_fixed_array::FixedString,
};

use crate::{
    constants::{CHARACTER_LIMIT, EDIT, NEXT, PIN, PREVIOUS, REDO, UNDO},
    database::Database,
    models::{character::Character, message::Message},
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
pub fn scaffolding(character: &Character, user_id: UserId) -> Vec<Message> {
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
                messages.push(Message::new_user("Användaren", content, user_id));
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
            .checked_rem(self.choices.len())
            .unwrap_or_default();
    }

    /// Shows the next choice by cycling the current index forward.
    pub fn next(&mut self) {
        if !self.is_on_last_choice() {
            self.current = self.current.saturating_add(1);
        }
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
        self.choices.tail.truncate(0);
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
        Self {
            id: stored.id,
            character: stored.character,
            choices,
            current: stored.current,
            has_finished: true,
            previous: stored.previous,
            pending: Vec::new(),
        }
    }

    /// Converts the history to a placeholder interaction response.
    pub async fn to_placeholder_interaction<'a>(
        &self,
        character: &'a Character,
        db: &Database,
    ) -> CreateInteractionResponse<'a> {
        CreateInteractionResponse::UpdateMessage(
            self.to_placeholder(character, Duration::ZERO, db)
                .await
                .to_slash_initial_response(CreateInteractionResponseMessage::new()),
        )
    }

    /// Converts the history to a placeholder interaction response edit.
    pub async fn to_placeholder_interaction_edit<'a>(
        &self,
        character: &'a Character,
        elapsed: Duration,
        db: &Database,
    ) -> EditInteractionResponse<'a> {
        self.to_placeholder(character, elapsed, db)
            .await
            .to_slash_initial_response_edit(EditInteractionResponse::new())
    }

    /// Converts the history to a placeholder message.
    pub async fn to_placeholder_message<'a>(
        &self,
        character: &'a Character,
        replying_to: &DiscordMessage,
        db: &Database,
    ) -> CreateMessage<'a> {
        self.to_placeholder(character, Duration::ZERO, db)
            .await
            .to_prefix(replying_to.into())
            .reference_message(replying_to)
            .allowed_mentions(CreateAllowedMentions::new())
    }

    /// Converts the history to a placeholder message edit.
    pub async fn to_placeholder_message_edit<'a>(
        &self,
        character: &'a Character,
        elapsed: Duration,
        db: &Database,
    ) -> EditMessage<'a> {
        self.to_placeholder(character, elapsed, db)
            .await
            .to_prefix_edit(EditMessage::new())
            .allowed_mentions(CreateAllowedMentions::new())
    }

    /// Converts the history to a placeholder.
    async fn to_placeholder<'a>(
        &self,
        character: &'a Character,
        elapsed: Duration,
        db: &Database,
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

        let title = vec![CreateContainerComponent::Section(CreateSection::new(
            vec![
                CreateSectionComponent::TextDisplay(CreateTextDisplay::new(format!(
                    "## {character}"
                ))),
                CreateSectionComponent::TextDisplay(CreateTextDisplay::new("…")),
            ],
            CreateSectionAccessory::Thumbnail(CreateThumbnail::new(CreateUnfurledMediaItem::new(
                character.avatar().unwrap_or(EMPTY_AVATAR),
            ))),
        ))]
        .into();

        let components = create_buttons(
            1,
            self.has_finished,
            has_previous,
            has_edit,
            &db.characters_by_usage().await.unwrap_or_default(),
        );

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
    ) -> CreateInteractionResponse<'a> {
        CreateInteractionResponse::UpdateMessage(
            self.to_response(character, id, db)
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
    ) -> EditInteractionResponse<'a> {
        self.to_response(character, id, db)
            .await
            .to_slash_initial_response_edit(EditInteractionResponse::new())
    }

    /// Converts the history to an edit message response.
    pub async fn to_edit_response<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
    ) -> EditMessage<'a> {
        self.to_response(character, id, db)
            .await
            .to_prefix_edit(EditMessage::new())
            .allowed_mentions(CreateAllowedMentions::new())
    }

    /// Converts the history to a full response with buttons and choices.
    pub async fn to_response<'a, M: Into<MessageId>>(
        &'a self,
        character: &'a Character,
        id: M,
        db: &Database,
    ) -> CreateReply<'a> {
        let chosen = self.chosen_message();
        let content = chosen.chosen_revision().head().content();
        let footer = {
            let pages = format!(
                "{}/{}",
                self.current.saturating_add(1),
                self.choices_count()
            );

            let elapsed = if chosen.current_editor().is_some() {
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

            let editor = match chosen.current_editor() {
                Some(user_id) => format!(" | redigerad av {}", db.substitute_name(user_id).await),
                None => String::new(),
            };

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

            let len = format!(" | {}/{CHARACTER_LIMIT}", content.len());

            let footer = format!("-# {pages}{similarity}{elapsed}{len}{edit_pages}");
            vec![CreateContainerComponent::TextDisplay(
                CreateTextDisplay::new(footer),
            )]
            .into()
        };

        // discord rejects a whitespace-only text display, so fall back to a
        // visible placeholder when there is no content
        let text = if content.is_empty() { "…" } else { content };
        let (first, second) = match text.split_once('\n') {
            Some((first, second)) => (first, Some(second)),
            None => (text, None),
        };

        let title = vec![CreateContainerComponent::Section(CreateSection::new(
            vec![
                CreateSectionComponent::TextDisplay(CreateTextDisplay::new(format!(
                    "## {character}"
                ))),
                CreateSectionComponent::TextDisplay(CreateTextDisplay::new(first)),
            ],
            CreateSectionAccessory::Thumbnail(CreateThumbnail::new(CreateUnfurledMediaItem::new(
                character
                    .avatar()
                    .unwrap_or("https://upload.wikimedia.org/wikipedia/commons/c/ca/1x1.png"),
            ))),
        ))]
        .into();

        let components = create_buttons(
            id.into().into(),
            self.has_finished,
            self.has_multiple_choices(),
            self.chosen_has_edit(),
            &db.characters_by_usage().await.unwrap_or_default(),
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

/// Creates button components for the history message.
fn create_buttons<'a>(
    id: u64,
    finished: bool,
    previous: bool,
    edit: bool,
    characters: &[Character],
) -> Cow<'a, [CreateContainerComponent<'a>]> {
    let prev_msg_id = format!("{id}prev");
    let next_msg_id = format!("{id}next");
    let edit_msg_id = format!("{id}edit");
    let undo_id = format!("{id}undo");
    let redo_id = format!("{id}redo");
    let pin_id = format!("{id}pinn");
    let char_id = format!("{id}char");

    let mut components = vec![
        CreateContainerComponent::ActionRow(CreateActionRow::Buttons(
            vec![
                create_button(prev_msg_id, PREVIOUS, !finished || !previous),
                create_button(next_msg_id, NEXT, !finished),
                create_button(edit_msg_id, EDIT, !finished),
                create_button(undo_id, UNDO, !edit),
                create_button(redo_id, REDO, !edit),
            ]
            .into(),
        )),
        CreateContainerComponent::ActionRow(CreateActionRow::Buttons(
            vec![create_button(pin_id, PIN, !finished)].into(),
        )),
    ];
    if !characters.is_empty() {
        components.push(CreateContainerComponent::ActionRow(
            CreateActionRow::SelectMenu(CreateSelectMenu::new(
                char_id,
                CreateSelectMenuKind::String {
                    options: characters
                        .iter()
                        .map(|char| {
                            CreateSelectMenuOption::new(char.to_string(), char.id().to_owned())
                        })
                        .collect(),
                },
            )),
        ));
    }
    components.into()
}

/// Creates a single button component.
fn create_button(custom_id: String, emoji: &str, disabled: bool) -> CreateButton<'_> {
    CreateButton::new(custom_id)
        .disabled(disabled)
        .style(ButtonStyle::Secondary)
        .emoji(ReactionType::Unicode(FixedString::from_str_trunc(emoji)))
}

/// Creates a new [`History`] from a character, message ID, and user ID.
///
/// The conversation starts empty (the scaffolding is derived at request time); the greeting is
/// the single initial choice.
impl From<(&Character, MessageId, UserId)> for History {
    fn from((character, id, _user_id): (&Character, MessageId, UserId)) -> Self {
        let mut message = Message::new_system("");
        message.edit(character.name(), character.greeting(), None::<u64>);
        let choices = NonEmpty::new(message);

        Self::builder()
            .choices(choices)
            .character(character.id())
            .id(id)
            .build()
    }
}
