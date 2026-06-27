//! The version/edit/clone lifecycle for [`Character`]: superseding a character
//! with a new version, rolling back to an old one, cloning a fresh copy, and the
//! modal-driven create/edit conversions.
//!
//! Split out from the model in `character.rs`; as a child module this can still
//! reach `Character`'s private fields.

use jiff::Zoned;
use poise::serenity_prelude::all::UserId;
use ulid::Ulid;
use url::Url;

use crate::models::{
    character::Character,
    modals::{
        CreateCharacterModal, EditCharacterModal, SecondCreateCharacterModal,
        SecondEditCharacterModal,
    },
};

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the version/edit/clone lifecycle is split into this child module to separate it from the Character model"
)]
impl Character {
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
        self.voice.clone_from(&old.voice);
    }

    /// Produces a fresh, independent copy of the character owned by `creator`.
    ///
    /// Mints a new ULID, resets the version lineage and every accumulated stat
    /// (editors, conversation counts, generated words/tokens, timestamps), and
    /// stamps `creator` as the new owner. Every content field is copied, including
    /// the model-settings and voice overrides. With `new_name` of `None`, the copy's
    /// name defaults to the original suffixed with " (kopia)".
    #[must_use]
    pub fn duplicate<U: Into<UserId>>(&self, creator: U, new_name: Option<String>) -> Self {
        let name = new_name.unwrap_or_else(|| format!("{} (kopia)", self.name));
        Self::builder()
            .id(Ulid::new().to_string())
            .name(name)
            .greeting(self.greeting.clone())
            .creator(creator)
            .maybe_nickname(self.nickname.clone())
            .maybe_description(self.description.clone())
            .maybe_personality(self.personality.clone())
            .maybe_prompt(self.prompt.clone())
            .maybe_system_prompt(self.system_prompt.clone())
            .maybe_scenario(self.scenario.clone())
            .maybe_avatar(self.avatar.clone())
            .maybe_emoji(self.emoji.clone())
            .maybe_color(self.color)
            .example_messages(self.example_messages.clone())
            .maybe_model_settings(self.model_settings.clone())
            .maybe_voice(self.voice.clone())
            .build()
    }

    /// Builds the two character-edit modals pre-filled with the current field values,
    /// so the edit forms open populated with the existing character instead of blank.
    ///
    /// The required name and greeting always carry a value; an absent optional field
    /// stays empty so the user is never shown a value the character does not have.
    #[must_use]
    pub fn edit_modal_defaults(&self) -> (EditCharacterModal, SecondEditCharacterModal) {
        let first = EditCharacterModal {
            name: Some(self.name.clone()),
            greeting: Some(self.greeting.clone()),
            nickname: self.nickname.clone(),
            description: self.description.clone(),
            personality: self.personality.clone(),
        };
        let second = SecondEditCharacterModal {
            avatar: self.avatar.clone(),
            emoji: self.emoji.clone(),
            system_prompt: self.system_prompt.clone(),
            prompt: self.prompt.clone(),
            scenario: self.scenario.clone(),
        };
        (first, second)
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

/// Validates a URL string, returning `Some` if it's a valid HTTP or HTTPS URL, `None` otherwise.
fn validate_url(maybe_url: Option<String>) -> Option<String> {
    maybe_url
        .and_then(|url| Url::parse(&url).ok())
        .filter(|url| matches!(url.scheme(), "https" | "http"))
        .map(|url| url.to_string())
}
