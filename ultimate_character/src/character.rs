use std::{
    collections::{HashMap, HashSet},
    fs::{read, write},
    path::{Path, PathBuf},
    sync::LazyLock,
};

use bon::Builder;
use jiff::Zoned;
use parking_lot::RwLock;
use ron::ser::PrettyConfig;
use serde::{Deserialize, Serialize};
use serenity::all::{
    ButtonStyle, Color, CreateActionRow, CreateButton, CreateEmbed, CreateEmbedFooter,
    CreateMessage, EditMessage, ReactionType, UserId,
};
use snafu::{ResultExt, Snafu};
use tracing::warn;
use ulid::Ulid;

type Result<T, E = Error> = std::result::Result<T, E>;

const CHARACTERS_PATH: &str = "characters.ron";

pub static CHARACTERS: LazyLock<RwLock<Characters>> =
    LazyLock::new(|| RwLock::new(Characters::load().expect("valid characters")));

#[derive(Serialize, Deserialize)]
pub struct Characters(HashMap<Ulid, Character>);

#[derive(Debug, Snafu)]
enum Error {
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
        if PathBuf::from(CHARACTERS_PATH).exists() {
            read(CHARACTERS_PATH)
                .context(ReadSnafu)
                .and_then(|bytes| {
                    ron::de::from_bytes::<Vec<Character>>(&bytes).context(DeserializeSnafu)
                })
                .map(|characters| {
                    characters
                        .into_iter()
                        .map(|character| (character.id, character))
                        .collect::<HashMap<Ulid, Character>>()
                })
                .map(Self)
        } else {
            Ok(Self::default())
        }
    }

    fn save(&self) {
        if let Err(why) = ron::ser::to_string_pretty(
            &self.0.values().collect::<Vec<&Character>>(),
            PrettyConfig::new(),
        )
        .context(SerializeSnafu)
        .and_then(|string| write(CHARACTERS_PATH, string).context(WriteSnafu))
        {
            warn!("Error while saving characters: {why}");
        }
    }

    pub fn insert(&mut self, character: Character) {
        self.0.insert(character.id, character);
        self.save();
    }
}

impl Default for Characters {
    fn default() -> Self {
        let characters = Self(HashMap::new());
        if !Path::new(CHARACTERS_PATH).exists() {
            characters.save();
        }
        characters
    }
}

/// A character.
#[derive(Serialize, Deserialize, Builder)]
pub struct Character {
    /// The character's name.
    #[builder(into)]
    name: String,
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
    /// The character's greeting.
    ///
    /// This is the first message in every conversation.
    #[builder(into)]
    greeting: String,
    /// The character's unique ID, generated on creation.
    #[builder(default = Ulid::new())]
    id: Ulid,
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
    /// The Discord user ID of the character's original creator.
    #[builder(into)]
    creator: UserId,
    /// The Discord user IDs of anyone who has ever edited the character, if any.
    #[builder(default)]
    all_editors: HashSet<UserId>,
    /// The Discord user ID of the latest person to edit the character, if any.
    latest_editor: Option<UserId>,
    /// The current version of the character.
    ///
    /// This starts at 0 and increments by 1 with each edit.
    #[builder(default)]
    version: u32,
    /// Whether the character is "killed."
    ///
    /// If true, the character will no longer be visible.
    #[builder(default)]
    killed: bool,
    /// Whether the character has been superseded (a newer version exists).
    ///
    /// If true, the character will no longer be visible.
    #[builder(default)]
    superseded: bool,
    /// The hex color of the character.
    color: Option<Color>,
    /// The time the character was created.
    #[builder(default = Zoned::now())]
    created_at: Zoned,
    /// The times the character was edited.
    #[builder(default)]
    edited_at: Vec<Zoned>,
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
    /// The system prompt, always placed as the latest message.
    system_prompt: Option<String>,
    /// The prompt for the character.
    ///
    /// This should be a list of instructions for how the character should
    /// behave.
    prompt: Option<String>,
    /// The scenario for the character.
    ///
    /// This is meant as the scene, setting, or location that the character
    /// starts in.
    scenario: Option<String>,
    /// The frequency penalty for the character.
    ///
    /// If set, this overrides the default frequency penalty for requests.
    frequency_penalty: Option<f32>,
    /// The presence penalty for the character.
    ///
    /// If set, this overrides the default presence penalty for requests.
    presence_penalty: Option<f32>,
    /// The temperature for the character.
    ///
    /// If set, this overrides the default temperature for requests.
    temperature: Option<f32>,
    /// The top-p value for the character.
    ///
    /// If set, this overrides the default top-p value for requests.
    top_p: Option<f32>,
    /// The previous version of the character.
    ///
    /// This is used for rollback purposes.
    previous_version: Option<Ulid>,
}

impl Character {
    #[must_use]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    #[must_use]
    pub fn avatar(&self) -> Option<String> {
        self.avatar.clone()
    }

    #[must_use]
    pub const fn color(&self) -> Option<Color> {
        self.color
    }

    pub fn to_create_message(&self, id: impl Into<u64>) -> CreateMessage {
        let buttons = create_buttons(id, false);
        let footer = CreateEmbedFooter::new("1/1 | tar 0.0s | 0/4096");
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
        finished: bool,
    ) -> EditMessage {
        let index = index.into();
        let input = input.into();
        let elapsed = elapsed.into();
        let len = input.len();
        let buttons = create_buttons(id, finished);
        let footer_text = format!("{index}/{index} | tar {elapsed}s | {len}/4096");
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
}

fn create_buttons(id: impl Into<u64>, finished: bool) -> Vec<CreateActionRow> {
    let id = id.into();
    let prev_id = format!("{id}prev");
    let next_id = format!("{id}next");
    let edit_id = format!("{id}edit");
    vec![CreateActionRow::Buttons(vec![
        CreateButton::new(prev_id)
            .disabled(!finished)
            .emoji(ReactionType::Unicode("◀".to_owned())),
        CreateButton::new(next_id)
            .disabled(!finished)
            .emoji(ReactionType::Unicode("▶".to_owned())),
        CreateButton::new(edit_id)
            .disabled(!finished)
            .style(ButtonStyle::Secondary)
            .emoji(ReactionType::Unicode("✎".to_owned())),
    ])]
}
