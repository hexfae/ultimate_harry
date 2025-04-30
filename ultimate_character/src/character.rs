use bon::Builder;
use dashmap::DashMap;
use jiff::Zoned;
use poise::{
    CreateReply,
    serenity_prelude::{
        CreateInteractionResponse, CreateInteractionResponseMessage, ReactionType,
        all::{
            ButtonStyle, Color, CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter,
            CreateMessage, EditMessage, UserId,
        },
    },
};
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use snafu::{ResultExt, Snafu};
use std::{
    collections::{HashMap, HashSet},
    fmt::Display,
    fs::{read, write},
    path::Path,
    sync::LazyLock,
    time::Duration,
};
use strsim::normalized_damerau_levenshtein;
use tracing::warn;
use ulid::Ulid;
use ultimate_config::{CONFIG, ModelSettings};
use ultimate_modals::{
    CreateCharacterModal, EditCharacterModal, SecondCreateCharacterModal, SecondEditCharacterModal,
};
use ultimate_phrases::{NO_PHRASES, YES_PHRASES, sample};
use url::Url;

type Result<T, E = Error> = std::result::Result<T, E>;

const CHARACTERS_PATH: &str = "characters.ron";

const CHARACTER_LIMIT: u16 = 4096;

const PREVIOUS: &str = "⬅️";
const NEXT: &str = "➡️";
const EDIT: &str = "✏️";
const UNDO: &str = "↩️";
const REDO: &str = "↪️";

pub static CHARACTERS: LazyLock<Characters> =
    LazyLock::new(|| Characters::load().expect("valid characters"));

#[derive(Serialize, Deserialize)]
pub struct Characters(DashMap<Ulid, Character>);

#[derive(Debug, Snafu)]
pub enum Error {
    #[snafu(display("Error while reading characters: {source}"))]
    Read { source: std::io::Error },
    #[snafu(display("Error while deserializing characters: {source}"))]
    Deserialize { source: ron::de::SpannedError },
    #[snafu(display("Error while serializing characters: {source}"))]
    Serialize { source: ron::error::Error },
    #[snafu(display("Error while writing characters: {source}"))]
    Write { source: std::io::Error },
}

impl Characters {
    fn load() -> Result<Self, Error> {
        if Path::new(CHARACTERS_PATH).exists() {
            read(CHARACTERS_PATH)
                .context(ReadSnafu)
                .and_then(|bytes| {
                    ron::de::from_bytes::<Vec<Character>>(&bytes).context(DeserializeSnafu)
                })
                .map(|characters| {
                    characters
                        .into_iter()
                        .map(|character| (character.id, character))
                        .collect()
                })
                .map(Self)
        } else {
            Ok(Self(DashMap::new()))
        }
    }

    fn save(&self) {
        let characters = match ron::ser::to_string_pretty(
            &self
                .0
                .iter()
                .map(|c| c.value().to_owned())
                .collect::<Vec<Character>>(),
            PrettyConfig::new(),
        )
        .context(SerializeSnafu)
        {
            Ok(characters) => characters,
            Err(why) => {
                warn!("{why}");
                return;
            }
        };

        if let Err(why) = write(CHARACTERS_PATH, characters).context(WriteSnafu) {
            warn!("{why}");
        }
    }

    fn characters(&self) -> impl Iterator<Item = Character> {
        self.0
            .iter()
            .map(|c| c.value().to_owned())
            .filter(|character| character.deleted_by.is_none())
            .filter(|character| character.next_version.is_none())
    }

    pub fn insert(&self, character: Character) {
        self.0.insert(character.id, character);
        self.save();
    }

    #[must_use]
    pub fn get_all_sorted_by_usage(&self) -> Vec<Character> {
        let mut characters = self.characters().collect::<Vec<Character>>();
        characters.sort_by(|a, b| a.conversations_had.cmp(&b.conversations_had));
        characters.reverse();
        characters
    }

    pub fn get_all_sorted_by_similarity(&self, input: impl AsRef<str>) -> Vec<(f64, Character)> {
        let mut similarities_and_characters = self
            .characters()
            .map(|character| {
                (
                    normalized_damerau_levenshtein(input.as_ref(), character.name()),
                    character,
                )
            })
            .collect::<Vec<(f64, Character)>>();
        similarities_and_characters.sort_by(|(a, _), (b, _)| f64::total_cmp(a, b));
        similarities_and_characters.reverse();
        similarities_and_characters
    }

    pub fn get_by_id(&self, id: impl Into<Ulid>) -> Option<Character> {
        self.0.get(&id.into()).map(|c| c.value().to_owned())
    }

    pub fn delete_by_id(&self, id: impl Into<Ulid>, user: impl Into<UserId>) -> Option<()> {
        if let Some(mut character) = self.0.get_mut(&id.into()) {
            character.deleted_by = Some(user.into());
            character.deleted_at = Some(Zoned::now());
            self.save();
            Some(())
        } else {
            None
        }
    }

    pub fn supersede_by_id(&self, old_id: impl Into<Ulid>, new_id: impl Into<Ulid>) {
        if let Some(mut character) = self.0.get_mut(&old_id.into()) {
            character.next_version = Some(new_id.into());
            self.save();
        }
    }
}

/// A character.
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct Character {
    /// The character's name.
    #[builder(into)]
    name: String,
    /// The character's greeting.
    ///
    /// This is the first message in every conversation.
    #[builder(into)]
    greeting: String,
    /// The character's unique ID, generated on creation.
    #[builder(default = Ulid::new())]
    id: Ulid,
    /// The Discord user ID of the character's original creator.
    #[builder(into)]
    creator: UserId,
    /// The current version of the character.
    ///
    /// This starts at 0 and increments by 1 with each edit.
    #[builder(default)]
    version: u32,
    /// The ID of the next version of the character.
    ///
    /// If Some, the character will no longer be visible.
    next_version: Option<Ulid>,
    /// The ID of the previous version of the character.
    ///
    /// This is used for rollback purposes.
    previous_version: Option<Ulid>,
    /// The character's nickname.
    ///
    /// This is intented to be a short version of the name, to
    /// more easily start conversations with the character.
    nickname: Option<String>,
    /// A short description of the character.
    ///
    /// This is not read by the model, it is only used for display purposes.
    description: Option<String>,
    /// The character's personality.
    ///
    /// This should be a first-person description of the character.
    ///
    /// This is read by the model, as a guide on how to act.
    personality: Option<String>,
    /// The prompt for the character.
    ///
    /// This should be a list of instructions for how the character should
    /// behave.
    prompt: Option<String>,
    /// The system prompt, always placed as the latest message.
    system_prompt: Option<String>,
    /// The scenario for the character.
    ///
    /// This is meant as the scene, setting, or location that the character
    /// starts in.
    scenario: Option<String>,
    /// The character's avatar's URL.
    ///
    /// Note: Do not set this to the link of an uploaded image on Discord,
    /// as those are temporary. Instead, prefer a link to an image on a
    /// permanent host, such as a CDN or a public image host.
    avatar: Option<String>,
    /// The character's emoji. Usually displayed before the character's name.
    ///
    /// Although this is called "emoji", it actually being an emoji is never
    /// enforced.
    emoji: Option<String>,
    /// The Discord user IDs of anyone who has ever edited the character, if any.
    #[builder(default)]
    all_editors: HashSet<UserId>,
    /// The Discord user ID of the latest person to edit the character, if any.
    latest_editor: Option<UserId>,
    /// If Some, the Discord user ID of the character's deleter.
    ///
    /// If Some, the character will no longer be visible.
    deleted_by: Option<UserId>,
    /// The hex color of the character.
    color: Option<Color>,
    /// The time the character was created.
    #[builder(default = Zoned::now())]
    created_at: Zoned,
    /// The times the character was edited.
    #[builder(default)]
    edited_at: Vec<Zoned>,
    /// If Some, the time the character was deleted.
    deleted_at: Option<Zoned>,
    /// The latest time the character had a conversation with a user.
    latest_conversation: Option<Zoned>,
    /// The number of conversations the character has had with a user.
    ///
    /// This means the amount of times this character has been "spawned".
    #[builder(default)]
    conversations_had_with_user: HashMap<UserId, u32>,
    /// The number of conversations the character has had.
    ///
    /// This means the amount of times this character has been "spawned".
    #[builder(default)]
    conversations_had: u32,
    /// The number of words generated by the character.
    #[builder(default)]
    words_generated: u32,
    /// The number of tokens generated by the character.
    #[builder(default)]
    tokens_generated: u32,
    /// The example messages for the character.
    ///
    /// Setting example messages are a great way to teach the character how to
    /// act.
    ///
    /// The first element of the tuple is optionally a user's message, and the
    /// second is the character's response to that message.
    #[builder(default)]
    example_messages: Vec<(Option<String>, String)>,
    /// The model settings override for the character.
    ///
    /// If set, this overrides the default model settings for requests.
    model_settings: Option<ModelSettings>,
}

impl Character {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn greeting(&self) -> &str {
        &self.greeting
    }

    #[must_use]
    pub fn personality(&self) -> Option<&str> {
        self.personality.as_deref()
    }

    #[must_use]
    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }

    #[must_use]
    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    #[must_use]
    pub fn scenario(&self) -> Option<&str> {
        self.scenario.as_deref()
    }

    #[must_use]
    pub const fn example_messages(&self) -> &[(Option<String>, String)] {
        self.example_messages.as_slice()
    }

    #[must_use]
    pub fn avatar(&self) -> Option<&str> {
        self.avatar.as_deref()
    }

    #[must_use]
    pub const fn color(&self) -> Option<Color> {
        self.color
    }

    #[must_use]
    pub const fn id(&self) -> Ulid {
        self.id
    }

    #[must_use]
    pub const fn conversations_had(&self) -> u32 {
        self.conversations_had
    }

    #[must_use]
    pub fn model_settings(&self) -> Option<ModelSettings> {
        self.model_settings.clone()
    }

    pub fn edit_from_modals(
        &mut self,
        editor: impl Into<UserId>,
        modal: EditCharacterModal,
        second_modal: SecondEditCharacterModal,
    ) {
        let editor = editor.into();
        self.latest_editor = Some(editor);
        self.all_editors.insert(editor);
        self.edited_at.push(Zoned::now());
        self.version += 1;
        self.previous_version = Some(self.id);
        self.id = Ulid::new();
        let avatar = validate_url(second_modal.avatar);
        // the reason why these can't just be `self.foo = bar` is because
        // if the user doesn't fill in a field, it will be None, and we
        // don't want to overwrite a potentially existing value
        if let Some(name) = modal.name {
            self.name = name;
        }
        if let Some(greeting) = modal.greeting {
            self.greeting = greeting;
        }
        if let Some(nickname) = modal.nickname {
            self.nickname = Some(nickname);
        }
        if let Some(description) = modal.description {
            self.description = Some(description);
        }
        if let Some(personality) = modal.personality {
            self.personality = Some(personality);
        }
        if let Some(avatar) = avatar {
            self.avatar = Some(avatar);
        }
        if let Some(emoji) = second_modal.emoji {
            self.emoji = Some(emoji);
        }
        if let Some(system_prompt) = second_modal.system_prompt {
            self.system_prompt = Some(system_prompt);
        }
        if let Some(prompt) = second_modal.prompt {
            self.prompt = Some(prompt);
        }
        if let Some(scenario) = second_modal.scenario {
            self.scenario = Some(scenario);
        }
    }

    pub fn to_create_message(&self, id: impl Into<u64>) -> CreateMessage {
        let buttons = create_buttons(
            id,
            HasPrevious::No,
            HasFinished::No,
            HasUndo::No,
            HasRedo::No,
        );
        let footer = CreateEmbedFooter::new(format!("1/1 | tar 0.0s | 0/{CHARACTER_LIMIT}"));
        let mut embed = CreateEmbed::new()
            .title(self.name())
            .description("…")
            .footer(footer);
        if let Some(avatar) = self.avatar() {
            embed = embed.thumbnail(avatar);
        }
        if let Some(color) = self.color() {
            embed = embed.color(color);
        }
        CreateMessage::new().embed(embed).components(buttons)
    }

    pub fn to_edit_message(
        &self,
        id: impl Into<u64>,
        index: impl Into<usize>,
        input: impl Into<String>,
        elapsed: impl Into<f64>,
        finished: HasFinished,
    ) -> EditMessage {
        let index = index.into();
        let input = input.into();
        let elapsed = elapsed.into();
        let len = input.len();
        let buttons = create_buttons(id, HasPrevious::No, finished, HasUndo::No, HasRedo::No);
        let footer_text = format!("{index}/{index} | tar {elapsed}s | {len}/{CHARACTER_LIMIT}");
        let footer = CreateEmbedFooter::new(footer_text);
        let mut embed = CreateEmbed::new()
            .title(self.name())
            .description(input)
            .footer(footer);
        if let Some(avatar) = self.avatar() {
            embed = embed.thumbnail(avatar);
        }
        if let Some(color) = self.color() {
            embed = embed.color(color);
        }
        EditMessage::new().embed(embed).components(buttons)
    }

    // it is necessary
    #[allow(clippy::too_many_arguments)]
    pub fn master_reply(
        &self,
        id: impl Into<u64>,
        (current_page, total_pages): (usize, usize),
        (current_edit, total_edits, current_editor): (usize, usize, Option<impl Into<UserId>>),
        similarity: Option<f64>,
        content: impl Into<String>,
        elapsed: Option<Duration>,
        has_finished: HasFinished,
    ) -> CreateReply {
        let name = &self.name;
        let content = content.into();
        let avatar = &self.avatar;
        let color = self.color;

        let footer = {
            let pages = if total_pages == 0 {
                String::new()
            } else {
                format!("{}/{}", current_page + 1, total_pages)
            };

            let elapsed = elapsed.map_or_else(String::new, |elapsed| {
                if has_finished == HasFinished::Yes {
                    format!(" | tog {:.1}s", elapsed.as_secs_f64())
                } else {
                    format!(" | tar {:.1}s", elapsed.as_secs_f64())
                }
            });

            let similarity = similarity.map_or_else(String::new, |similarity| {
                format!(" | {:.0}% namnlikhet", similarity * 100.0)
            });

            let editor = current_editor.map_or_else(String::new, |editor| {
                format!(
                    "(redigerad av {})",
                    CONFIG.read().substitute_name(editor.into())
                )
            });

            let edit_pages = if total_edits == 0 {
                String::new()
            } else {
                format!(" | {}/{} {}", current_edit + 1, total_edits + 1, editor)
            };

            let len = format!(" | {}/{CHARACTER_LIMIT}", content.len());

            let footer = format!("{pages}{similarity}{elapsed}{len}{edit_pages}");
            CreateEmbedFooter::new(footer)
        };

        let mut embed = CreateEmbed::new()
            .title(name)
            .description(content)
            .footer(footer);

        if let Some(avatar) = avatar {
            embed = embed.thumbnail(avatar);
        }
        if let Some(color) = color {
            embed = embed.color(color);
        }

        let has_previous = if total_pages > 1 {
            HasPrevious::Yes
        } else {
            HasPrevious::No
        };
        let has_undo = if total_edits > 0 {
            HasUndo::Yes
        } else {
            HasUndo::No
        };
        let has_redo = if current_edit < total_edits {
            HasRedo::Yes
        } else {
            HasRedo::No
        };
        let components = create_buttons(id, has_previous, has_finished, has_undo, has_redo);

        CreateReply::default().embed(embed).components(components)
    }

    pub fn to_reply_with_similarity_from_message_edit(
        &self,
        id: impl Into<u64> + Copy,
        similarity: f64,
        content: impl Into<String>,
    ) -> CreateReply {
        let (embed, components) = self.to_greeting_embed_with_similarity_and_message_buttons(
            id,
            similarity,
            content.into(),
        );

        CreateReply::default().embed(embed).components(components)
    }

    pub fn to_greeting_reply_with_similarity_and_message(
        &self,
        id: impl Into<u64> + Copy,
        similarity: f64,
        message: impl Into<String>,
    ) -> CreateReply {
        let (embed, components) =
            self.to_greeting_embed_with_similarity_and_message_buttons(id, similarity, message);

        CreateReply::default().embed(embed).components(components)
    }

    #[must_use]
    pub fn to_greeting_reply_with_similarity(
        &self,
        id: impl Into<u64> + Copy,
        similarity: f64,
    ) -> CreateReply {
        self.to_greeting_reply_with_similarity_and_message(id, similarity, self.greeting())
    }

    pub fn to_greeting_embed_with_similarity_and_message_buttons(
        &self,
        id: impl Into<u64>,
        similarity: f64,
        message: impl Into<String>,
    ) -> (CreateEmbed, Vec<CreateActionRow>) {
        let name = &self.name;
        let content = message.into();
        let avatar = &self.avatar;
        let color = self.color;
        let footer = CreateEmbedFooter::new(format!("{:.0}% namnlikhet", similarity * 100.0));
        let mut embed = CreateEmbed::new()
            .title(name)
            .description(content)
            .footer(footer);

        if let Some(avatar) = avatar {
            embed = embed.thumbnail(avatar);
        }
        if let Some(color) = color {
            embed = embed.color(color);
        }
        let components = create_buttons(
            id,
            HasPrevious::Yes,
            HasFinished::Yes,
            HasUndo::No,
            HasRedo::No,
        );
        (embed, components)
    }

    pub fn to_create_interaction_response(
        &self,
        id: impl Into<u64>,
        similarity: f64,
        custom_message: impl Into<String>,
    ) -> CreateInteractionResponse {
        let name = &self.name;
        let content = custom_message.into();
        let avatar = &self.avatar;
        let color = self.color;
        let components = create_buttons(
            id,
            HasPrevious::No,
            HasFinished::Yes,
            HasUndo::No,
            HasRedo::No,
        );
        let footer = CreateEmbedFooter::new(format!("{:.0}% namnlikhet", similarity * 100.0));
        let mut embed = CreateEmbed::new()
            .title(name)
            .description(content)
            .footer(footer);

        if let Some(avatar) = avatar {
            embed = embed.thumbnail(avatar);
        }
        if let Some(color) = color {
            embed = embed.color(color);
        }
        CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new()
                .embed(embed)
                .components(components),
        )
    }

    pub fn to_embed_with_footer_text(&self, footer_text: impl Into<String>) -> CreateEmbed {
        let name = &self.name;
        let greeting = &self.greeting;
        let description = &self.description;
        let avatar = &self.avatar;
        let color = self.color;
        let footer = CreateEmbedFooter::new(footer_text.into());

        let mut embed = CreateEmbed::new()
            .title(name)
            .field("Hälsning", greeting, false)
            .footer(footer);

        if let Some(description) = description {
            embed = embed.description(description);
        }
        if let Some(avatar) = avatar {
            embed = embed.thumbnail(avatar);
        }
        if let Some(color) = color {
            embed = embed.color(color);
        }
        for (user, count) in &self.conversations_had_with_user {
            embed = embed.field(
                CONFIG.read().substitute_name(user),
                count.to_string(),
                false,
            );
        }
        embed
    }

    pub fn to_embed(&self) -> CreateEmbed {
        self.to_embed_with_footer_text(format!("{} konversationer", self.conversations_had()))
    }

    #[must_use]
    pub fn to_embed_reply(&self) -> CreateReply {
        CreateReply::default().embed(self.to_embed())
    }

    #[must_use]
    pub fn to_confirm_reply(&self, id: impl Into<u64>, content: impl Into<String>) -> CreateReply {
        let buttons = create_confirm_buttons(id);
        self.to_embed_reply().content(content).components(buttons)
    }

    #[must_use]
    pub fn to_confirm_interaction_response(
        &self,
        id: impl Into<u64>,
        content: impl Into<String>,
    ) -> CreateInteractionResponse {
        let buttons = create_confirm_buttons(id);
        CreateInteractionResponse::UpdateMessage(
            CreateInteractionResponseMessage::new()
                .content(content.into())
                .components(buttons),
        )
    }
}

impl From<(CreateCharacterModal, SecondCreateCharacterModal, UserId)> for Character {
    fn from(
        (first, second, creator): (CreateCharacterModal, SecondCreateCharacterModal, UserId),
    ) -> Self {
        let avatar = validate_url(second.avatar);
        Self::builder()
            .name(first.name)
            .greeting(first.greeting)
            .maybe_nickname(first.nickname)
            .maybe_description(first.description)
            .maybe_personality(first.personality)
            .maybe_avatar(avatar)
            .maybe_emoji(second.emoji)
            .creator(creator)
            .maybe_system_prompt(second.system_prompt)
            .maybe_prompt(second.prompt)
            .maybe_scenario(second.scenario)
            .build()
    }
}

impl Display for Character {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(emoji) = &self.emoji {
            write!(f, "{emoji} ")?;
        }
        write!(f, "{}", self.name())
    }
}

fn validate_url(url: Option<String>) -> Option<String> {
    url.and_then(|url| Url::parse(&url).ok())
        .filter(|url| url.scheme() == "https" || url.scheme() == "http")
        .map(|url| url.to_string())
}

#[allow(clippy::needless_pass_by_value)]
fn create_buttons(
    id: impl Into<u64>,
    previous: HasPrevious,
    finished: HasFinished,
    undo: HasUndo,
    redo: HasRedo,
) -> Vec<CreateActionRow> {
    let id = id.into();
    let prev_msg_id = format!("{id}prev");
    let next_msg_id = format!("{id}next");
    let edit_msg_id = format!("{id}edit");
    let undo_id = format!("{id}undo");
    let redo_id = format!("{id}redo");

    let has_finished = finished == HasFinished::Yes;
    let has_previous = previous == HasPrevious::Yes;
    let has_undo = undo == HasUndo::Yes;
    let has_redo = redo == HasRedo::Yes;

    vec![CreateActionRow::Buttons(vec![
        CreateButton::new(prev_msg_id)
            .disabled(!has_finished || !has_previous)
            .style(ButtonStyle::Secondary)
            .emoji(ReactionType::Unicode(PREVIOUS.to_owned())),
        CreateButton::new(next_msg_id)
            .disabled(!has_finished)
            .style(ButtonStyle::Secondary)
            .emoji(ReactionType::Unicode(NEXT.to_owned())),
        CreateButton::new(edit_msg_id)
            .disabled(!has_finished)
            .style(ButtonStyle::Secondary)
            .emoji(ReactionType::Unicode(EDIT.to_owned())),
        CreateButton::new(undo_id)
            .disabled(!has_undo)
            .style(ButtonStyle::Secondary)
            .emoji(ReactionType::Unicode(UNDO.to_owned())),
        CreateButton::new(redo_id)
            .disabled(!has_redo)
            .style(ButtonStyle::Secondary)
            .emoji(ReactionType::Unicode(REDO.to_owned())),
    ])]
}

fn create_confirm_buttons(id: impl Into<u64>) -> Vec<CreateActionRow> {
    let id = id.into();
    let confirm_id = format!("{id}confirm");
    let cancel_id = format!("{id}cancel");
    vec![CreateActionRow::Buttons(vec![
        CreateButton::new(confirm_id)
            .style(ButtonStyle::Secondary)
            .label(sample(YES_PHRASES)),
        CreateButton::new(cancel_id)
            .style(ButtonStyle::Secondary)
            .label(sample(NO_PHRASES)),
    ])]
}

#[derive(PartialEq, Eq)]
pub enum HasFinished {
    Yes,
    No,
}

/// If this message has a previous version (a previous page).
///
/// Note that this should be `Self::Yes` if the current page is the first page
/// and there are multiple pages (to allow wrapping around to the last page).
/// The only time this should be `Self::No` is if this is the first (and only)
/// page.
#[derive(PartialEq, Eq)]
enum HasPrevious {
    Yes,
    No,
}

#[derive(PartialEq, Eq)]
enum HasUndo {
    Yes,
    No,
}

#[derive(PartialEq, Eq)]
enum HasRedo {
    Yes,
    No,
}
