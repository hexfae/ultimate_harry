//! The history model for storing chat histories between users and characters.

use alloc::borrow::Cow;
use bon::Builder;
use core::time::Duration;
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
use surrealdb::RecordId;

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
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct History {
    /// The ulid ID of the currently responding character.
    #[builder(with = |id: &RecordId| id.to_owned())]
    character: RecordId,
    /// The current responses the user can pick between by "swiping" (pressing next/previous).
    choices: NonEmpty<Message>,
    /// The index of the current response the user has chosen.
    #[builder(default)]
    current: usize,
    #[builder(default)]
    /// If the streaming has finished for this history.
    ///
    /// This is used to create embeds while streaming a response.
    ///
    /// Defaults to true, since if a history is saved, it has finished.
    #[serde(skip_serializing, default = "default_true")]
    has_finished: bool,
    /// The Discord Message ID of this history.
    #[builder(with = |id: MessageId| RecordId::from(("history", id.to_string())))]
    id: RecordId,
    /// The previous messages, the history of the chat.
    previous: NonEmpty<Message>,
}

/// Always returns true.
///
/// Used for making `History.has_finished` true when deserializing.
const fn default_true() -> bool {
    true
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
    pub const fn has_finished(&mut self, has_finished: bool) {
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
    pub fn choices_len(&self) -> usize {
        self.choices.len()
    }

    /// Returns whether the user is on the last choice.
    #[must_use]
    pub fn is_on_last_choice(&self) -> bool {
        self.current_choice().saturating_add(1) >= self.choices_len()
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
    pub fn chosen_choice_message(&self) -> &Message {
        self.choices
            .get(self.current)
            .unwrap_or_else(|| self.choices.first())
    }

    /// Returns the Discord message ID of this history.
    #[must_use]
    pub const fn id(&self) -> &RecordId {
        &self.id
    }

    /// Sets the Discord message ID of this history.
    pub fn set_id<M: Into<MessageId>>(&mut self, id: M) {
        self.id = RecordId::from(("history", id.into().to_string()));
    }

    /// Returns the last message in the history.
    #[must_use]
    pub fn last(&self) -> &Message {
        self.previous.last()
    }

    /// Returns the character ID of the currently responding character.
    #[must_use]
    pub const fn character(&self) -> &RecordId {
        &self.character
    }

    /// Sets the character ID of the currently responding character.
    pub fn set_character(&mut self, character: RecordId) {
        self.character = character;
    }

    /// Rebuilds the character setup portion of history.
    ///
    /// This replaces personality, prompt, scenario, example messages, and system prompt
    /// with the new character's corresponding values.
    #[expect(
        clippy::expect_used,
        reason = "multiple messages are unconditionally added, making it impossible for the nonempty to return an error"
    )]
    pub fn replace_setup_with(&mut self, character: &Character, user_id: UserId) {
        let conversation_start = self
            .previous
            .iter()
            .position(|msg| {
                msg.chosen_revision()
                    .head()
                    .content()
                    .contains(BEGIN_MESSAGE)
            })
            .map_or(0, |i| i.saturating_add(1));

        let mut new_previous: Vec<Message> = Vec::new();
        new_previous.push(Message::new_system(SYSTEM_MESSAGE));

        if let Some(personality) = character.personality() {
            new_previous.push(Message::new_system(BEGIN_PERSONALITY));
            new_previous.push(Message::new_assistant(personality, character));
        }

        if let Some(prompt) = character.prompt() {
            let mut begin_prompt = BEGIN_PROMPT.to_owned();
            begin_prompt.push_str(prompt);
            new_previous.push(Message::new_system(begin_prompt));
        }

        if let Some(scenario) = character.scenario() {
            let mut begin_scenario = BEGIN_SCENARIO.to_owned();
            begin_scenario.push_str(scenario);
            new_previous.push(Message::new_system(begin_scenario));
        }

        if !character.example_messages().is_empty() {
            new_previous.push(Message::new_system(BEGIN_EXAMPLE_MESSAGES));

            for (user_message, assistant_message) in character.example_messages() {
                if let Some(content) = user_message {
                    new_previous.push(Message::new_user("Användaren", content, user_id));
                }
                new_previous.push(Message::new_assistant(assistant_message, character));
                new_previous.push(Message::new_system(EXAMPLE_MESSAGE_SEPARATOR));
            }
        }

        if let Some(system_prompt) = character.system_prompt() {
            new_previous.push(Message::new_system(system_prompt));
        }

        new_previous.push(Message::new_system(BEGIN_MESSAGE));

        let conversation: Vec<Message> = self
            .previous
            .iter()
            .skip(conversation_start)
            .cloned()
            .collect();

        new_previous.extend(conversation);

        self.previous = NonEmpty::from_vec(new_previous).expect("setup should not be empty");
    }

    /// Pushes a message to the history.
    pub fn push<M: Into<Message>>(&mut self, message: M) {
        self.previous.push(message.into());
    }

    /// Resets the choices, removing all but the first choice.
    pub fn reset_choices(&mut self) {
        self.choices.tail.truncate(0);
    }

    /// Sets the choices and resets the current index to 0.
    pub fn set_choices<M: Into<Message>>(&mut self, choices: M) {
        self.choices = NonEmpty::new(choices.into());
        self.current = 0;
    }

    /// Pushes a new choice and sets the current index to that choice.
    pub fn push_choice<M: Into<Message>>(&mut self, choice: M) {
        self.choices.push(choice.into());
        self.current = self.choices_len().saturating_sub(1);
    }

    /// Returns the previous messages in the history.
    #[must_use]
    pub const fn previous_messages(&self) -> &NonEmpty<Message> {
        &self.previous
    }

    /// Converts the history to a placeholder interaction response.
    pub fn to_placeholder_interaction<'a>(
        &self,
        character: &'a Character,
    ) -> CreateInteractionResponse<'a> {
        CreateInteractionResponse::UpdateMessage(
            self.to_placeholder(character, Duration::ZERO)
                .to_slash_initial_response(CreateInteractionResponseMessage::new()),
        )
    }

    /// Converts the history to a placeholder interaction response edit.
    pub fn to_placeholder_interaction_edit<'a>(
        &self,
        character: &'a Character,
        elapsed: Duration,
    ) -> EditInteractionResponse<'a> {
        self.to_placeholder(character, elapsed)
            .to_slash_initial_response_edit(EditInteractionResponse::new())
    }

    /// Converts the history to a placeholder message.
    pub fn to_placeholder_message<'a>(
        &self,
        character: &'a Character,
        replying_to: &DiscordMessage,
    ) -> CreateMessage<'a> {
        self.to_placeholder(character, Duration::ZERO)
            .to_prefix(replying_to.into())
            .reference_message(replying_to)
            .allowed_mentions(CreateAllowedMentions::new())
    }

    /// Converts the history to a placeholder message edit.
    pub fn to_placeholder_message_edit<'a>(
        &self,
        character: &'a Character,
        elapsed: Duration,
    ) -> EditMessage<'a> {
        self.to_placeholder(character, elapsed)
            .to_prefix_edit(EditMessage::new())
            .allowed_mentions(CreateAllowedMentions::new())
    }

    /// Converts the history to a placeholder.
    fn to_placeholder<'a>(&self, character: &'a Character, elapsed: Duration) -> CreateReply<'a> {
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

        let components = create_buttons(1, self.has_finished, has_previous, has_edit, &[]);

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
            .chosen_choice_message()
            .chosen_revision()
            .head()
            .content()
            .to_owned();
        let footer = {
            let editor = match self.chosen_choice_message().current_editor() {
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
        let chosen = self.chosen_choice_message();

        let has_previous = self.choices.len() > 1;
        let has_edit = chosen.revisions_len() > 0;
        let content = chosen.chosen_revision().head().content();
        let footer = {
            let pages = if self.choices.is_empty() {
                String::new()
            } else {
                format!("{}/{}", self.current.saturating_add(1), self.choices.len())
            };

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

            let edit_pages = if chosen.revisions_len() == 0 {
                String::new()
            } else {
                format!(
                    " | {}/{}{}",
                    chosen.revision().saturating_add(1),
                    chosen.revisions_len().saturating_add(1),
                    editor
                )
            };

            let len = format!(" | {}/{CHARACTER_LIMIT}", content.len());

            let footer = format!("-# {pages}{similarity}{elapsed}{len}{edit_pages}");
            vec![CreateContainerComponent::TextDisplay(
                CreateTextDisplay::new(footer),
            )]
            .into()
        };

        // content must contain at least 1 character, but we want it to remain visually empty
        let text = if content.is_empty() { " " } else { content };
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
            has_previous,
            has_edit,
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
                            CreateSelectMenuOption::new(char.to_string(), char.id().to_string())
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
impl From<(&Character, MessageId, UserId)> for History {
    fn from((character, id, user_id): (&Character, MessageId, UserId)) -> Self {
        let mut history = NonEmpty::new(Message::new_system(SYSTEM_MESSAGE));

        if let Some(personality) = character.personality() {
            history.push(Message::new_system(BEGIN_PERSONALITY));
            history.push(Message::new_assistant(personality, character));
        }

        if let Some(prompt) = character.prompt() {
            let mut begin_prompt = BEGIN_PROMPT.to_owned();
            begin_prompt.push_str(prompt);
            history.push(Message::new_system(begin_prompt));
        }

        if let Some(scenario) = character.scenario() {
            let mut begin_scenario = BEGIN_SCENARIO.to_owned();
            begin_scenario.push_str(scenario);

            history.push(Message::new_system(begin_scenario));
        }

        if !character.example_messages().is_empty() {
            history.push(Message::new_system(BEGIN_EXAMPLE_MESSAGES));

            for (user_message, assistant_message) in character.example_messages() {
                if let Some(content) = user_message {
                    history.push(Message::new_user("Användaren", content, user_id));
                }

                history.push(Message::new_assistant(assistant_message, character));

                history.push(Message::new_system(EXAMPLE_MESSAGE_SEPARATOR));
            }
        }

        if let Some(system_prompt) = character.system_prompt() {
            history.push(Message::new_system(system_prompt));
        }

        history.push(Message::new_system(BEGIN_MESSAGE));

        let mut message = Message::new_system("");
        message.edit(character.name(), character.greeting(), None::<u64>);
        let choices = NonEmpty::new(message);

        Self::builder()
            .previous(history)
            .choices(choices)
            .character(character.id())
            .id(id)
            .build()
    }
}
