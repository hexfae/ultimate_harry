use async_openai::types::ChatCompletionRequestMessage;
use bon::Builder;
use nonempty::NonEmpty;
use poise::CreateReply;
use serde::{Deserialize, Serialize};
use serenity::all::{
    ButtonStyle, CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter, MessageId,
    ReactionType, UserId,
};
use surrealdb::RecordId;
use ultimate_character::Character;
use ultimate_config::CONFIG;
use ultimate_message::Message;

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

    #[must_use]
    pub fn to_placeholder(&self, character: &Character) -> CreateReply {
        let (has_finished, has_previous, has_edit) = (false, false, false);

        let footer = {
            let pages = if self.choices.is_empty() {
                String::new()
            } else {
                format!("{}/{}", self.current + 2, self.choices.len() + 1)
            };
            CreateEmbedFooter::new(pages)
        };

        let mut embed = CreateEmbed::new()
            .title(character.name())
            .description("…")
            .footer(footer);

        if let Some(avatar) = character.avatar() {
            embed = embed.thumbnail(avatar);
        }
        if let Some(color) = character.color() {
            embed = embed.color(color);
        }

        let components = create_buttons(1, has_finished, has_previous, has_edit);

        CreateReply::default().embed(embed).components(components)
    }

    #[must_use]
    pub fn to_bare_response(&self, character: &Character, link: String) -> CreateReply {
        let content = self
            .chosen_choice_message()
            .chosen_revision()
            .head()
            .content();
        let footer = {
            let editor = self
                .chosen_choice_message()
                .current_editor()
                .map_or_else(String::new, |editor| {
                    format!(" (redigerad av {})", CONFIG.read().substitute_name(editor))
                });
            let len = format!("{}/{CHARACTER_LIMIT}", content.len());

            let footer = format!("{len}{editor}");
            CreateEmbedFooter::new(footer)
        };
        let mut embed = CreateEmbed::new()
            .title(character.name())
            .description(content)
            .footer(footer);

        if let Some(avatar) = character.avatar() {
            embed = embed.thumbnail(avatar);
        }
        if let Some(color) = character.color() {
            embed = embed.color(color);
        }
        CreateReply::default().content(link).embed(embed)
    }

    #[must_use]
    pub fn to_response(&self, character: &Character, id: MessageId) -> CreateReply {
        let chosen = self.chosen_choice_message();

        let has_previous = self.choices.len() > 1;
        let has_edit = chosen.revisions_len() > 0;
        let content = chosen.chosen_revision().head().content();
        let has_finished = true;
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

            let editor = chosen.current_editor().map_or_else(String::new, |editor| {
                format!("(redigerad av {})", CONFIG.read().substitute_name(editor))
            });

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

            let footer = format!("{pages}{similarity}{elapsed}{len}{edit_pages}");
            CreateEmbedFooter::new(footer)
        };

        let mut embed = CreateEmbed::new()
            .title(character.name())
            .description(content)
            .footer(footer);

        if let Some(avatar) = character.avatar() {
            embed = embed.thumbnail(avatar);
        }
        if let Some(color) = character.color() {
            embed = embed.color(color);
        }

        let components = create_buttons(id.into(), has_finished, has_previous, has_edit);

        CreateReply::default().embed(embed).components(components)
    }
}

#[allow(clippy::needless_pass_by_value)]
fn create_buttons(id: u64, finished: bool, previous: bool, edit: bool) -> Vec<CreateActionRow> {
    let prev_msg_id = format!("{id}prev");
    let next_msg_id = format!("{id}next");
    let edit_msg_id = format!("{id}edit");
    let undo_id = format!("{id}undo");
    let redo_id = format!("{id}redo");

    vec![CreateActionRow::Buttons(vec![
        create_button(prev_msg_id, PREVIOUS, !finished || !previous),
        create_button(next_msg_id, NEXT, !finished),
        create_button(edit_msg_id, EDIT, !finished),
        create_button(undo_id, UNDO, !edit),
        create_button(redo_id, REDO, !edit),
    ])]
}

fn create_button(custom_id: String, emoji: &'static str, disabled: bool) -> CreateButton {
    CreateButton::new(custom_id)
        .disabled(disabled)
        .style(ButtonStyle::Secondary)
        .emoji(ReactionType::Unicode(emoji.to_owned()))
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

impl From<History> for Vec<ChatCompletionRequestMessage> {
    fn from(history: History) -> Self {
        history
            .previous
            .into_iter()
            .flat_map(Into::<NonEmpty<ChatCompletionRequestMessage>>::into)
            .collect()
    }
}
