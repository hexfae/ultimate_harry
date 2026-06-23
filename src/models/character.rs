//! The character model for Discord character chats.

//! This module contains the `Character` struct which represents an AI character
//! that users can chat with.

use bon::Builder;
use core::fmt::{Display, Formatter, Result as FmtResult, Write as _};
use tracing::warn;
use jiff::Zoned;
use poise::serenity_prelude::all::{Color, UserId};
use native_db::{ToKey as _, native_db};
use native_model::{Model as _, native_model};
use serde::{Deserialize, Serialize};
use crate::constants::MAX_RESULTS;
use alloc::collections::{BTreeMap, BTreeSet};
use ulid::Ulid;
use url::Url;

use crate::{
    database::Database,
    llm::ModelSettings,
    models::modals::{
        CreateCharacterModal, EditCharacterModal, SecondCreateCharacterModal,
        SecondEditCharacterModal,
    },
};

/// Compact date format for the "senast använd" leaderboard column.
///
/// See [`jiff::fmt::strtime`] for formatting details.
const LATEST_CONVERSATION_FORMAT: &str = "%Y-%m-%d %H:%M";

/// A character.
#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
#[native_model(id = 1, version = 1, with = crate::codec::Json)]
#[native_db]
pub struct Character {
    /// The character's unique ID (a ULID), generated on creation. Used as the primary key.
    #[primary_key]
    id: String,
    /// The character's name.
    #[builder(into)]
    name: String,
    /// The character's greeting.
    ///
    /// This is the first message in every conversation.
    #[builder(into)]
    greeting: String,
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
    next_version: Option<String>,
    /// The ID of the previous version of the character.
    ///
    /// This is used for rollback purposes.
    previous_version: Option<String>,
    /// The character's nickname.
    ///
    /// This is intended to be a short version of the name, to
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
    ///
    /// A `BTreeSet` (not a `HashSet`) so the JSON serialization is deterministic:
    /// `native_db` re-encodes the stored record on every update and rejects the
    /// write if the bytes differ from what is on disk.
    #[builder(default)]
    all_editors: BTreeSet<UserId>,
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
    /// If Some, the time this version was created.
    edited_at: Option<Zoned>,
    /// If Some, the time the character was deleted.
    deleted_at: Option<Zoned>,
    /// The latest time the character had a conversation with a user.
    latest_conversation: Option<Zoned>,
    /// The number of conversations the character has had with a user.
    ///
    /// This means the amount of times this character has been "spawned".
    ///
    /// A `BTreeMap` (not a `HashMap`) so the JSON serialization is deterministic:
    /// `native_db` re-encodes the stored record on every update and rejects the
    /// write if the bytes differ from what is on disk.
    #[builder(default)]
    conversations_had_with_user: BTreeMap<UserId, u32>,
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
    /// The similarity of the character to the input.
    ///
    /// This is `Some` when the user searches a character by name, e.g. `chat` or `delete`. and `None` in e.g. `view`.
    ///
    /// Transient view state, never persisted: rebuilt by `rank_by_similarity` at query time.
    #[serde(skip)]
    similarity: Option<f64>,
}

/// A lightweight projection of a character for the hand-off select menu.
///
/// The menu only needs the label shown to the user and the ID to switch to, so
/// this avoids cloning (and carrying around) whole [`Character`] records just to
/// render 25 select options.
#[derive(Debug, Clone)]
#[expect(
    clippy::module_name_repetitions,
    reason = "this is a projection of a Character, so the shared prefix is meaningful"
)]
pub struct CharacterOption {
    /// The display label (emoji + name), as produced by the character's `Display`.
    label: String,
    /// The character's ID, used as the select option's value.
    id: String,
}

impl CharacterOption {
    /// Returns the option's display label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Returns the option's character ID.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl Character {
    /// Returns the character's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the character's greeting message.
    #[must_use]
    pub fn greeting(&self) -> &str {
        &self.greeting
    }

    /// Returns the character's personality description.
    #[must_use]
    pub fn personality(&self) -> Option<&str> {
        self.personality.as_deref()
    }

    /// Returns the character's prompt instructions.
    #[must_use]
    pub fn prompt(&self) -> Option<&str> {
        self.prompt.as_deref()
    }

    /// Returns the character's system prompt.
    #[must_use]
    pub fn system_prompt(&self) -> Option<&str> {
        self.system_prompt.as_deref()
    }

    /// Returns the character's scenario.
    #[must_use]
    pub fn scenario(&self) -> Option<&str> {
        self.scenario.as_deref()
    }

    /// Returns the character's example messages.
    #[must_use]
    pub const fn example_messages(&self) -> &[(Option<String>, String)] {
        self.example_messages.as_slice()
    }

    /// Returns the character's avatar URL.
    #[must_use]
    pub fn avatar(&self) -> Option<&str> {
        self.avatar.as_deref()
    }

    /// Returns the character's embed color.
    #[must_use]
    pub const fn color(&self) -> Option<Color> {
        self.color
    }

    /// Returns the character's nickname.
    #[must_use]
    pub fn nickname(&self) -> Option<&str> {
        self.nickname.as_deref()
    }

    /// Returns the character's short description.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns the character's version number (zero-based).
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    /// Returns the Discord user ID of the character's original creator.
    #[must_use]
    pub const fn creator(&self) -> UserId {
        self.creator
    }

    /// Returns the Discord user ID of the latest editor, if any.
    #[must_use]
    pub const fn latest_editor(&self) -> Option<UserId> {
        self.latest_editor
    }

    /// Returns the time the character was created.
    #[must_use]
    pub const fn created_at(&self) -> &Zoned {
        &self.created_at
    }

    /// Returns the time this version was created, if it is an edit.
    #[must_use]
    pub const fn edited_at(&self) -> Option<&Zoned> {
        self.edited_at.as_ref()
    }

    /// Returns the character's unique ID.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns a lightweight hand-off menu option (label + ID) for this character.
    #[must_use]
    pub fn to_menu_option(&self) -> CharacterOption {
        CharacterOption {
            label: self.to_string(),
            id: self.id.clone(),
        }
    }

    /// Returns whether the character is visible (not deleted and not superseded by a newer version).
    #[must_use]
    pub const fn is_visible(&self) -> bool {
        self.deleted_at.is_none() && self.next_version.is_none()
    }

    /// Returns whether the character has been soft-deleted.
    #[must_use]
    pub const fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }

    /// Returns the ID of the next version of this character, if it has been superseded.
    #[must_use]
    pub fn next_version(&self) -> Option<&str> {
        self.next_version.as_deref()
    }

    /// Returns the ID of the previous version of this character, if it has one.
    #[must_use]
    pub fn previous_version(&self) -> Option<&str> {
        self.previous_version.as_deref()
    }

    /// Filters out deleted and superseded characters, then returns up to [`MAX_RESULTS`] ranked by
    /// name (and nickname) similarity to the input, breaking ties by number of conversations had.
    ///
    /// This replaces the old `MOST_SIMILAR_TO` `SurrealQL` query and sets each returned
    /// character's `similarity` for display.
    #[must_use]
    pub fn rank_by_similarity(characters: Vec<Self>, input: &str) -> Vec<Self> {
        Self::rank_filtered(characters, input, Self::is_visible)
    }

    /// Like [`rank_by_similarity`](Self::rank_by_similarity), but ranks only the
    /// soft-deleted characters, used by the restore command to find a character
    /// to bring back from deletion.
    #[must_use]
    pub fn rank_deleted_by_similarity(characters: Vec<Self>, input: &str) -> Vec<Self> {
        Self::rank_filtered(characters, input, Self::is_deleted)
    }

    /// Keeps the characters for which `keep` returns true, then returns up to
    /// [`MAX_RESULTS`] ranked by name (and nickname) similarity to the input,
    /// breaking ties by number of conversations had, setting each returned
    /// character's `similarity` for display.
    fn rank_filtered(
        characters: Vec<Self>,
        input: &str,
        keep: impl Fn(&Self) -> bool,
    ) -> Vec<Self> {
        let mut ranked: Vec<Self> = characters
            .into_iter()
            .filter(|character| keep(character))
            .map(|mut character| {
                let name_similarity =
                    strsim::normalized_damerau_levenshtein(&character.name, input);
                let similarity = character.nickname.as_deref().map_or(
                    name_similarity,
                    |nickname| {
                        name_similarity
                            .max(strsim::normalized_damerau_levenshtein(nickname, input))
                    },
                );
                character.similarity = Some(similarity);
                character
            })
            .collect();
        ranked.sort_by(|left, right| {
            let left_similarity = left.similarity.unwrap_or_default();
            let right_similarity = right.similarity.unwrap_or_default();
            right_similarity
                .total_cmp(&left_similarity)
                .then_with(|| right.conversations_had.cmp(&left.conversations_had))
        });
        ranked.truncate(MAX_RESULTS);
        ranked
    }

    /// Returns the total number of conversations the character has had.
    #[must_use]
    pub const fn conversations_had(&self) -> u32 {
        self.conversations_had
    }

    /// Returns the number of words this character has generated.
    #[must_use]
    pub const fn words_generated(&self) -> u32 {
        self.words_generated
    }

    /// Returns the number of tokens this character has generated.
    #[must_use]
    pub const fn tokens_generated(&self) -> u32 {
        self.tokens_generated
    }

    /// Records that this character was spawned into a new conversation by the given user.
    ///
    /// Bumps the total and per-user conversation counts and refreshes the
    /// latest-conversation timestamp.
    pub fn record_spawn(&mut self, user: UserId) {
        self.conversations_had = self.conversations_had.saturating_add(1);
        let count = self.conversations_had_with_user.entry(user).or_default();
        *count = count.saturating_add(1);
        self.latest_conversation = Some(Zoned::now());
    }

    /// Records the words and tokens this character generated in a single reply.
    pub const fn record_generation(&mut self, words: u32, tokens: u32) {
        self.words_generated = self.words_generated.saturating_add(words);
        self.tokens_generated = self.tokens_generated.saturating_add(tokens);
    }

    /// Returns a formatted string of conversation counts, grouped by user.
    #[must_use]
    pub async fn formatted_conversations_had(&self, db: &Database) -> String {
        let mut string = format!("Totalt: {}", self.conversations_had);
        // a BTreeMap already iterates in sorted key order, so no separate sort is needed
        for (id, count) in &self.conversations_had_with_user {
            let name = db.substitute_name(id).await;
            if let Err(why) = write!(string, "\nMed {name}: {count}") {
                warn!("error while writing to string: {why}");
            }
        }
        string
    }

    /// Returns the character's last-used time, compactly formatted, or `aldrig`
    /// when the character has never been spawned into a conversation.
    #[must_use]
    pub fn formatted_latest_conversation(&self) -> String {
        self.latest_conversation.as_ref().map_or_else(
            || "aldrig".to_owned(),
            |time| time.strftime(LATEST_CONVERSATION_FORMAT).to_string(),
        )
    }

    /// Returns the character's model settings override, if any.
    #[must_use]
    pub const fn model_settings(&self) -> Option<&ModelSettings> {
        self.model_settings.as_ref()
    }

    /// Returns whether the character has its own model settings override.
    #[must_use]
    pub const fn has_model_settings(&self) -> bool {
        self.model_settings.is_some()
    }

    /// Sets the character's model settings override.
    pub fn set_model_settings(&mut self, model_settings: ModelSettings) {
        self.model_settings = Some(model_settings);
    }

    /// Marks the character as deleted by the given user, recording the time of deletion.
    pub fn mark_deleted(&mut self, deleted_by: UserId) {
        self.deleted_by = Some(deleted_by);
        self.deleted_at = Some(Zoned::now());
    }

    /// Clears the character's deleted state, making it visible again.
    pub fn restore(&mut self) {
        self.deleted_by = None;
        self.deleted_at = None;
    }

    /// Supersedes `self` with a fresh version atop itself.
    ///
    /// Records the editor, stamps the edit time, bumps the version, links the
    /// new version back to the head, and mints a fresh ULID. Shared prologue of
    /// [`rollback_to`](Self::rollback_to) and
    /// [`edit_from_modals`](Self::edit_from_modals); the caller then copies the
    /// new content fields over the carried-forward stats.
    fn begin_new_version<E: Into<UserId>>(&mut self, editor: E) {
        let editor_id = editor.into();
        self.latest_editor = Some(editor_id);
        self.all_editors.insert(editor_id);
        self.edited_at = Some(Zoned::now());
        self.version = self.version.saturating_add(1);
        self.previous_version = Some(self.id.clone());
        self.id = Ulid::new().to_string();
    }

    /// Rolls the character back to the content of an older version `old`.
    ///
    /// Like [`edit_from_modals`](Self::edit_from_modals), this turns `self` (the
    /// current head) into a fresh version atop itself: it keeps the head's
    /// accumulated stats, creator, and creation time, copies every content field
    /// from `old`, bumps the version, and links the new version back to the head.
    /// The caller supersedes the head with this new version.
    pub fn rollback_to<E: Into<UserId>>(&mut self, editor: E, old: &Self) {
        self.begin_new_version(editor);
        self.name.clone_from(&old.name);
        self.greeting.clone_from(&old.greeting);
        self.nickname.clone_from(&old.nickname);
        self.description.clone_from(&old.description);
        self.personality.clone_from(&old.personality);
        self.prompt.clone_from(&old.prompt);
        self.system_prompt.clone_from(&old.system_prompt);
        self.scenario.clone_from(&old.scenario);
        self.avatar.clone_from(&old.avatar);
        self.emoji.clone_from(&old.emoji);
        self.color = old.color;
        self.example_messages.clone_from(&old.example_messages);
        self.model_settings.clone_from(&old.model_settings);
    }

    /// Sets the ID of the next version of this character, hiding it from view.
    pub fn set_next_version(&mut self, next_version: String) {
        self.next_version = Some(next_version);
    }

    /// Returns the similarity indicator for display.
    ///
    /// Returns e.g. ` | 75% namnlikhet` if `Some(0.75)`, otherwise returns an empty String.
    #[must_use]
    pub fn similarity(&self) -> String {
        self.similarity.map_or_else(String::new, |similarity| {
            format!(" | {:.0}% namnlikhet", similarity * 100.0_f64)
        })
    }

    /// Edits the character using data from the edit modals.
    ///
    /// Updates the character's fields with the new values from the modals,
    /// increments the version, and sets the previous version ID.
    pub fn edit_from_modals<E: Into<UserId>>(
        &mut self,
        editor: E,
        modal: EditCharacterModal,
        second_modal: SecondEditCharacterModal,
    ) {
        self.begin_new_version(editor);
        let avatar_url = validate_url(second_modal.avatar);
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
        if let Some(avatar) = avatar_url {
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

}

impl From<(CreateCharacterModal, SecondCreateCharacterModal, UserId)> for Character {
    fn from(
        (first, second, creator): (CreateCharacterModal, SecondCreateCharacterModal, UserId),
    ) -> Self {
        let avatar = validate_url(second.avatar);
        Self::builder()
            .name(first.name)
            .greeting(first.greeting)
            .id(Ulid::new().to_string())
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
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        if let Some(emoji) = &self.emoji {
            write!(f, "{emoji} ")?;
        }
        write!(f, "{}", self.name())
    }
}

/// Validates a URL string, returning `Some` if it's a valid HTTP or HTTPS URL, `None` otherwise.
fn validate_url(maybe_url: Option<String>) -> Option<String> {
    maybe_url
        .and_then(|url| Url::parse(&url).ok())
        .filter(|url| matches!(url.scheme(), "https" | "http"))
        .map(|url| url.to_string())
}

/// Tests for character similarity ranking.
#[cfg(test)]
mod tests {
    use super::{Character, MAX_RESULTS};
    use serenity::all::UserId;

    /// Builds a minimal visible character with the given ID and name.
    fn basic_character(id: &str, name: &str) -> Character {
        Character::builder()
            .id(id.to_owned())
            .name(name)
            .greeting("hello")
            .creator(UserId::new(1))
            .build()
    }

    /// Returns the index of the character with the given ID in the ranked list.
    fn position_of(ranked: &[Character], id: &str) -> Option<usize> {
        ranked.iter().position(|character| character.id() == id)
    }

    /// Ranking filters deleted and superseded characters, orders by similarity (honoring the
    /// nickname), and breaks ties by number of conversations had.
    #[test]
    fn ranks_visible_characters_by_similarity_and_conversations() {
        let exact = basic_character("id-exact", "Banana");
        let close = basic_character("id-close", "Bananas");
        let nickname = Character::builder()
            .id("id-nickname".to_owned())
            .name("Xyzzy")
            .greeting("hello")
            .creator(UserId::new(1))
            .nickname("Banan".to_owned())
            .build();
        let far = basic_character("id-far", "Zzzzzz");

        let mut deleted = basic_character("id-deleted", "Banana");
        deleted.mark_deleted(UserId::new(2));
        let mut superseded = basic_character("id-superseded", "Banana");
        superseded.set_next_version("id-exact".to_owned());

        let tie_high = Character::builder()
            .id("id-tie-high".to_owned())
            .name("Tie")
            .greeting("hello")
            .creator(UserId::new(1))
            .conversations_had(9_u32)
            .build();
        let tie_low = Character::builder()
            .id("id-tie-low".to_owned())
            .name("Tie")
            .greeting("hello")
            .creator(UserId::new(1))
            .conversations_had(1_u32)
            .build();

        let ranked = Character::rank_by_similarity(
            vec![
                exact, close, nickname, far, deleted, superseded, tie_high, tie_low,
            ],
            "Banana",
        );

        assert_eq!(
            ranked.len(),
            6,
            "deleted and superseded characters are filtered out"
        );
        assert!(
            position_of(&ranked, "id-deleted").is_none(),
            "a deleted character never appears in the ranking"
        );
        assert!(
            position_of(&ranked, "id-superseded").is_none(),
            "a superseded character never appears in the ranking"
        );
        assert_eq!(
            ranked.first().map(Character::name),
            Some("Banana"),
            "the exact name match ranks first"
        );
        assert!(
            position_of(&ranked, "id-nickname") < position_of(&ranked, "id-far"),
            "a close nickname outranks a dissimilar name"
        );
        assert!(
            position_of(&ranked, "id-tie-high") < position_of(&ranked, "id-tie-low"),
            "equal similarity breaks ties toward more conversations"
        );
    }

    /// Restoring a deleted character clears its deleted state, making it visible again.
    #[test]
    fn restore_makes_a_deleted_character_visible_again() {
        let mut character = basic_character("id", "Harry");
        character.mark_deleted(UserId::new(2));
        assert!(
            character.is_deleted(),
            "marking a character deleted sets its deleted state"
        );
        assert!(
            !character.is_visible(),
            "a deleted character is not visible"
        );

        character.restore();
        assert!(
            !character.is_deleted(),
            "restoring clears the deleted state"
        );
        assert!(
            character.is_visible(),
            "a restored character is visible again"
        );
    }

    /// Rolling back to an older version reverts the content fields to that version
    /// but keeps the current version's accumulated stats and links the new version
    /// back to the one it superseded.
    #[test]
    fn rollback_to_reverts_content_and_keeps_stats() {
        let old = Character::builder()
            .id("v0".to_owned())
            .name("Old Name")
            .greeting("old greeting")
            .creator(UserId::new(1))
            .personality("old personality".to_owned())
            .build();
        let mut current = Character::builder()
            .id("v1".to_owned())
            .name("New Name")
            .greeting("new greeting")
            .creator(UserId::new(1))
            .version(1_u32)
            .personality("new personality".to_owned())
            .conversations_had(7_u32)
            .build();

        current.rollback_to(UserId::new(3), &old);

        assert_eq!(
            current.name(),
            "Old Name",
            "the content reverts to the old version's name"
        );
        assert_eq!(
            current.greeting(),
            "old greeting",
            "the content reverts to the old version's greeting"
        );
        assert_eq!(
            current.personality(),
            Some("old personality"),
            "the content reverts to the old version's personality"
        );
        assert_eq!(
            current.conversations_had(),
            7,
            "the accumulated stats from the current version are kept"
        );
        assert_eq!(
            current.version,
            2,
            "the rolled-back version's number is bumped past the current one"
        );
        assert_eq!(
            current.previous_version(),
            Some("v1"),
            "the new version links back to the version it superseded"
        );
        assert_ne!(
            current.id(),
            "v1",
            "a rollback creates a new version with a fresh id"
        );
        assert!(
            current.is_visible(),
            "the rolled-back version is visible"
        );
    }

    /// Deleted-character ranking filters to only deleted characters and orders
    /// them by name similarity to the input.
    #[test]
    fn rank_deleted_by_similarity_returns_only_deleted_characters() {
        let visible = basic_character("id-visible", "Banana");
        let mut deleted_match = basic_character("id-deleted", "Banana");
        deleted_match.mark_deleted(UserId::new(2));
        let mut deleted_other = basic_character("id-deleted-other", "Zzzzzz");
        deleted_other.mark_deleted(UserId::new(2));

        let ranked = Character::rank_deleted_by_similarity(
            vec![visible, deleted_match, deleted_other],
            "Banana",
        );

        assert!(
            position_of(&ranked, "id-visible").is_none(),
            "a visible character never appears among deleted results"
        );
        assert_eq!(
            ranked.first().map(Character::id),
            Some("id-deleted"),
            "the closest-matching deleted character ranks first"
        );
        assert_eq!(ranked.len(), 2, "only deleted characters are returned");
    }

    /// `rank_by_similarity` caps its output at the shared `MAX_RESULTS`, matching the database
    /// listing queries and Discord's 25-option select-menu limit.
    #[test]
    fn ranking_is_capped_at_max_results() {
        let characters = (0..MAX_RESULTS.saturating_add(5))
            .map(|index| basic_character(&format!("id-{index}"), "Banana"))
            .collect();

        let ranked = Character::rank_by_similarity(characters, "Banana");

        assert_eq!(
            ranked.len(),
            MAX_RESULTS,
            "the ranking never returns more than MAX_RESULTS characters"
        );
    }
}
