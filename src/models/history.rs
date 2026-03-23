use std::borrow::Cow;

use bon::Builder;
use nonempty::NonEmpty;
use poise::CreateReply;
use serde::{Deserialize, Serialize};
use serenity::{
    all::{
        ButtonStyle, CreateActionRow, CreateButton, CreateComponent, CreateContainer,
        CreateContainerComponent, CreateEmbed, CreateEmbedFooter, CreateSection,
        CreateSectionAccessory, CreateSectionComponent, CreateSelectMenu, CreateSelectMenuKind,
        CreateSelectMenuOption, CreateTextDisplay, CreateThumbnail, CreateUnfurledMediaItem,
        MessageFlags, MessageId, ReactionType, UserId,
    },
    small_fixed_array::FixedString,
};
use surrealdb::RecordId;

use crate::{
    db::Database,
    models::{character::Character, message::Message},
};

const SYSTEM_MESSAGE: &str = "Du kommer nu att gå in i ett rollspel med en användare. Under inga omständigheter får du bryta rollspelet, gå ur karaktär, eller prata åt användaren.";

const BEGIN_PERSONALITY: &str = "Beskriv nu karaktären du ska rollspela som.";

const BEGIN_PROMPT: &str = "Detta är dina instruktioner som du ska följa under hela rollspelet: ";

const BEGIN_SCENARIO: &str = "Detta är scenen du och användaren finner er själva i: ";

const BEGIN_EXAMPLE_MESSAGES: &str = "Det följande är exempel på hur du ska prata med användaren.";

const EXAMPLE_MESSAGE_SEPARATOR: &str = "Nytt exempelmeddelande.";

const BEGIN_MESSAGE: &str = "Rollspelet börjar nu. Efter denna punkt får du inte längra avbryta rollspelet, gå ur karaktär, eller skriva åt användaren.";

const CHARACTER_LIMIT: u16 = 4096;

const PREVIOUS: &str = "⬅️";
const NEXT: &str = "➡️";
const EDIT: &str = "✏️";
const UNDO: &str = "↩️";
const REDO: &str = "↪️";
const PIN: &str = "📌";

/// A log of messages between the user and a character.
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct History {
    /// The previous messages, the history of the chat.
    previous: NonEmpty<Message>,
    /// The current responses the user can pick between by "swiping" (pressing next/previous).
    choices: NonEmpty<Message>,
    /// The index of the current response the user has chosen.
    #[builder(default)]
    current: usize,
    /// The ulid ID of the currently responding character.
    #[builder(with = |id: &RecordId| id.to_owned())]
    character: RecordId,
    /// The Discord Message ID of this history.
    #[builder(with = |id: MessageId| RecordId::from(("history", id.to_string())))]
    id: RecordId,
}

impl History {
    pub fn edit_content(
        &mut self,
        author: impl Into<String>,
        content: impl Into<String>,
        editor: Option<impl Into<UserId>>,
    ) {
        self.choices[self.current].edit(author, content, editor);
    }

    pub fn previous(&mut self) {
        self.current = (self.current + self.choices.len() - 1) % self.choices.len();
    }

    pub const fn next(&mut self) {
        self.current += 1;
    }

    #[must_use]
    pub const fn current_choice(&self) -> usize {
        self.current
    }

    #[must_use]
    pub fn choices_len(&self) -> usize {
        self.choices.len()
    }

    pub fn undo(&mut self) {
        self.choices[self.current].undo();
    }

    pub fn redo(&mut self) {
        self.choices[self.current].redo();
    }

    #[must_use]
    pub fn chosen_choice_message(&self) -> &Message {
        &self.choices[self.current]
    }

    #[must_use]
    pub const fn id(&self) -> &RecordId {
        &self.id
    }

    pub fn set_id(&mut self, id: impl Into<MessageId>) {
        self.id = RecordId::from(("history", id.into().to_string()));
    }

    #[must_use]
    pub fn last(&self) -> &Message {
        self.previous.last()
    }

    #[must_use]
    pub const fn character(&self) -> &RecordId {
        &self.character
    }

    pub fn set_character(&mut self, character: RecordId) {
        self.character = character;
    }

    /// Rebuilds the character setup portion of history.
    /// This replaces personality, prompt, scenario, example messages, and system prompt
    /// with the new character's corresponding values.
    pub fn replace_setup_with(&mut self, character: &Character, user_id: UserId) {
        // Collect all non-setup messages (everything after BEGIN_MESSAGE)
        let conversation_start = self
            .previous
            .iter()
            .position(|msg| {
                msg.chosen_revision()
                    .head()
                    .content()
                    .contains(BEGIN_MESSAGE)
            })
            .map(|i| i + 1)
            .unwrap_or(0);

        // Build new setup for the character
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
                if let Some(user_message) = user_message {
                    new_previous.push(Message::new_user("Användaren", user_message, user_id));
                }
                new_previous.push(Message::new_assistant(assistant_message, character));
                new_previous.push(Message::new_system(EXAMPLE_MESSAGE_SEPARATOR));
            }
        }

        if let Some(system_prompt) = character.system_prompt() {
            new_previous.push(Message::new_system(system_prompt));
        }

        new_previous.push(Message::new_system(BEGIN_MESSAGE));

        // Append the actual conversation messages
        let conversation: Vec<Message> = self
            .previous
            .iter()
            .skip(conversation_start)
            .cloned()
            .collect();

        new_previous.extend(conversation);

        // Replace the previous messages
        self.previous = NonEmpty::from_vec(new_previous).expect("setup should not be empty");
    }

    pub fn push(&mut self, message: impl Into<Message>) {
        self.previous.push(message.into());
    }

    pub fn reset_choices(&mut self) {
        self.choices.tail.truncate(0);
    }

    pub fn set_choices(&mut self, choices: impl Into<Message>) {
        self.choices = NonEmpty::new(choices.into());
        self.current = 0;
    }

    pub fn push_choice(&mut self, choice: impl Into<Message>) {
        self.choices.push(choice.into());
        self.current = self.choices_len() - 1;
    }

    pub fn previous_messages(&self) -> &NonEmpty<Message> {
        &self.previous
    }

    #[must_use]
    pub fn to_placeholder<'a>(&self, character: &'a Character) -> CreateReply<'a> {
        let (has_finished, has_previous, has_edit) = (false, false, false);

        let footer = {
            let pages = if self.choices.is_empty() {
                String::new()
            } else {
                format!("-# {}/{}", self.current + 2, self.choices.len() + 1)
            };
            Cow::Owned(vec![CreateContainerComponent::TextDisplay(
                CreateTextDisplay::new(pages),
            )])
        };

        let title = Cow::Owned(vec![CreateContainerComponent::Section(CreateSection::new(
            Cow::Owned(vec![
                CreateSectionComponent::TextDisplay(CreateTextDisplay::new(format!(
                    "## {character}"
                ))),
                CreateSectionComponent::TextDisplay(CreateTextDisplay::new("…")),
            ]),
            CreateSectionAccessory::Thumbnail(CreateThumbnail::new(CreateUnfurledMediaItem::new(
                character
                    .avatar()
                    .unwrap_or("https://upload.wikimedia.org/wikipedia/commons/c/ca/1x1.png"),
            ))),
        ))]);

        let components = create_buttons(1, has_finished, has_previous, has_edit, Vec::new());

        let container = Cow::Owned(vec![CreateComponent::Container(CreateContainer::new(
            [title, components, footer].concat(),
        ))]);

        CreateReply::default()
            .flags(MessageFlags::IS_COMPONENTS_V2)
            .components(container)
    }

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

    #[must_use]
    pub async fn to_response<'a>(
        &'a self,
        character: &'a Character,
        id: MessageId,
        db: &Database,
        has_finished: bool,
    ) -> CreateReply<'a> {
        let chosen = self.chosen_choice_message();

        let has_previous = self.choices.len() > 1;
        let has_edit = chosen.revisions_len() > 0;
        let content = chosen.chosen_revision().head().content();
        let footer = {
            let pages = if self.choices.is_empty() {
                String::new()
            } else {
                format!("{}/{}", self.current + 1, self.choices.len())
            };

            let elapsed = if chosen.current_editor().is_some() {
                String::new()
            } else {
                chosen.time_taken().map_or_else(String::new, |elapsed| {
                    format!(" | tog {:.1}s", elapsed.as_secs_f64())
                })
            };

            let similarity = character.similarity();

            let editor = match chosen.current_editor() {
                Some(user_id) => db.substitute_name(user_id).await,
                None => String::new(),
            };

            let edit_pages = if chosen.revisions_len() == 0 {
                String::new()
            } else {
                format!(
                    " | {}/{} {}",
                    chosen.revision() + 1,
                    chosen.revisions_len() + 1,
                    editor
                )
            };

            let len = format!(" | {}/{CHARACTER_LIMIT}", content.len());

            let footer = format!("-# {pages}{similarity}{elapsed}{len}{edit_pages}");
            Cow::Owned(vec![CreateContainerComponent::TextDisplay(
                CreateTextDisplay::new(footer),
            )])
        };

        // content must contain at least 1 character, but we want it to remain visually empty
        let content = if content.is_empty() { " " } else { content };
        let (first, second) = match content.split_once('\n') {
            Some((first, second)) => (first, Some(second)),
            None => (content, None),
        };

        let title = Cow::Owned(vec![CreateContainerComponent::Section(CreateSection::new(
            Cow::Owned(vec![
                CreateSectionComponent::TextDisplay(CreateTextDisplay::new(format!(
                    "## {character}"
                ))),
                CreateSectionComponent::TextDisplay(CreateTextDisplay::new(first)),
            ]),
            CreateSectionAccessory::Thumbnail(CreateThumbnail::new(CreateUnfurledMediaItem::new(
                character
                    .avatar()
                    .unwrap_or("https://upload.wikimedia.org/wikipedia/commons/c/ca/1x1.png"),
            ))),
        ))]);

        let components = create_buttons(
            id.into(),
            has_finished,
            has_previous,
            has_edit,
            db.characters_by_usage().await.unwrap_or_default(),
        );

        let container = Cow::Owned(vec![CreateComponent::Container(CreateContainer::new(
            [
                title,
                Cow::Owned(second.map_or_else(Vec::new, |text| {
                    text.split('\n')
                        .filter(|l| !l.is_empty())
                        .map(|part| {
                            CreateContainerComponent::TextDisplay(CreateTextDisplay::new(part))
                        })
                        .collect()
                })),
                components,
                footer,
            ]
            .concat(),
        ))]);

        CreateReply::default()
            .flags(MessageFlags::IS_COMPONENTS_V2)
            .components(container)
    }
}

#[allow(clippy::needless_pass_by_value)]
fn create_buttons<'a>(
    id: u64,
    finished: bool,
    previous: bool,
    edit: bool,
    characters: Vec<Character>,
) -> Cow<'a, [CreateContainerComponent<'a>]> {
    let prev_msg_id = format!("{id}prev");
    let next_msg_id = format!("{id}next");
    let edit_msg_id = format!("{id}edit");
    let undo_id = format!("{id}undo");
    let redo_id = format!("{id}redo");
    let pin_id = format!("{id}pin");
    let char_id = format!("{id}char");

    let mut components = vec![
        CreateContainerComponent::ActionRow(CreateActionRow::Buttons(Cow::Owned(vec![
            create_button(prev_msg_id, PREVIOUS, !finished || !previous),
            create_button(next_msg_id, NEXT, !finished),
            create_button(edit_msg_id, EDIT, !finished),
            create_button(undo_id, UNDO, !edit),
            create_button(redo_id, REDO, !edit),
        ]))),
        CreateContainerComponent::ActionRow(CreateActionRow::Buttons(Cow::Owned(vec![
            create_button(pin_id, PIN, !finished),
        ]))),
    ];
    if !characters.is_empty() {
        components.push(CreateContainerComponent::ActionRow(
            CreateActionRow::SelectMenu(CreateSelectMenu::new(
                char_id,
                CreateSelectMenuKind::String {
                    options: Cow::Owned(
                        characters
                            .iter()
                            .map(|c| CreateSelectMenuOption::new(c.to_string(), c.id().to_string()))
                            .collect(),
                    ),
                },
            )),
        ));
    }
    Cow::Owned(components)
}

fn create_button(custom_id: String, emoji: &str, disabled: bool) -> CreateButton<'_> {
    CreateButton::new(custom_id)
        .disabled(disabled)
        .style(ButtonStyle::Secondary)
        .emoji(ReactionType::Unicode(FixedString::from_str_trunc(emoji)))
}

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
                if let Some(user_message) = user_message {
                    history.push(Message::new_user("Användaren", user_message, user_id));
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
