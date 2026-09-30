//! Character records: listing, search, the version chain, and the
//! load-mutate-write helpers every character edit goes through.

use core::cmp::Reverse;
use nanorand::Rng as _;
use serenity::all::{Color, UserId};
use snafu::{IntoError, OptionExt as _, ResultExt as _};
use tracing::warn;

use super::store::{is_safe_id, read_json, scan_dir};
use super::{
    Database, DatabaseError, DeleteSnafu, GetSnafu, InsertSnafu, NoCharacterSnafu, StoreError,
    UpdateSnafu,
};
use crate::constants::MAX_RESULTS;
use crate::llm::CharacterModelSettings;
use crate::models::character::{Character, CharacterOption};

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the character record operations are split into this child module to separate them from the other record types"
)]
impl Database {
    /// Returns every character currently stored, regardless of visibility.
    async fn all_characters(&self) -> Result<Vec<Character>, DatabaseError> {
        scan_dir(&self.characters_dir()).await.context(GetSnafu)
    }

    /// Returns a single character by its ID.
    pub async fn character(&self, id: &str) -> Result<Option<Character>, DatabaseError> {
        if !is_safe_id(id) {
            return Ok(None);
        }
        read_json(&self.character_path(id)).await.context(GetSnafu)
    }

    /// Returns up to 25 characters, sorted by the most similar ones to the given name.
    pub async fn characters_by_similarity<T: Into<String>>(
        &self,
        name: T,
    ) -> Result<Vec<Character>, DatabaseError> {
        Ok(Character::rank_by_similarity(
            self.all_characters().await?,
            &name.into(),
        ))
    }

    /// Returns up to 25 soft-deleted characters, sorted by the most similar ones
    /// to the given name. Used by the restore command, since deleted characters
    /// are hidden from every other listing.
    pub async fn deleted_characters_by_similarity<T: Into<String>>(
        &self,
        name: T,
    ) -> Result<Vec<Character>, DatabaseError> {
        Ok(Character::rank_deleted_by_similarity(
            self.all_characters().await?,
            &name.into(),
        ))
    }

    /// Returns every visible (not deleted, not superseded) character.
    async fn visible_characters(&self) -> Result<Vec<Character>, DatabaseError> {
        Ok(self
            .all_characters()
            .await?
            .into_iter()
            .filter(Character::is_visible)
            .collect())
    }

    /// Returns up to 25 visible characters, sorted by the most commonly used ones.
    pub async fn characters_by_usage(&self) -> Result<Vec<Character>, DatabaseError> {
        let mut characters = self.visible_characters().await?;
        characters.sort_by_key(|character| Reverse(character.conversations_had()));
        characters.truncate(MAX_RESULTS);
        Ok(characters)
    }

    /// Returns up to 25 visible characters as lightweight hand-off menu options,
    /// sorted by the most commonly used ones.
    ///
    /// This is the shape the chat select menu needs; computing it once per reply
    /// (rather than re-scanning the whole character table on every streaming tick
    /// and button press) is the point of the projection.
    pub async fn character_menu_options(&self) -> Result<Vec<CharacterOption>, DatabaseError> {
        Ok(self
            .characters_by_usage()
            .await?
            .iter()
            .map(Character::to_menu_option)
            .collect())
    }

    /// Returns up to 25 visible characters, sorted randomly.
    pub async fn random_characters(&self) -> Result<Vec<Character>, DatabaseError> {
        let mut characters = self.visible_characters().await?;
        let mut rng = nanorand::tls_rng();
        rng.shuffle(&mut characters);
        characters.truncate(MAX_RESULTS);
        Ok(characters)
    }

    /// Inserts a character.
    pub async fn insert_character(&self, character: Character) -> Result<(), DatabaseError> {
        self.write_json(&self.character_path(character.id()), &character)
            .await
            .context(InsertSnafu)
    }

    /// Loads the character `id`, applies `apply`, and writes it back, returning the mutated
    /// character. Returns `Ok(None)` if the character is missing. `context` selects the error
    /// variant for any failure, so each caller keeps its own error message.
    async fn mutate_character<C>(
        &self,
        id: &str,
        context: C,
        apply: impl FnOnce(&mut Character),
    ) -> Result<Option<Character>, DatabaseError>
    where
        C: IntoError<DatabaseError, Source = StoreError> + Copy,
    {
        if !is_safe_id(id) {
            return Ok(None);
        }
        let path = self.character_path(id);
        let Some(mut character) = read_json::<Character>(&path).await.context(context)? else {
            return Ok(None);
        };
        apply(&mut character);
        self.write_json(&path, &character).await.context(context)?;
        Ok(Some(character))
    }

    /// Soft-deletes a character by recording its deleter and the time of deletion.
    pub async fn delete_character<T: Into<UserId>>(
        &self,
        id: &str,
        deleted_by: T,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, DeleteSnafu, |character| {
            character.mark_deleted(deleted_by.into());
        })
        .await
    }

    /// Restores a soft-deleted character by clearing its deleted state. Returns
    /// the restored character, or `Ok(None)` if it is missing.
    pub async fn restore_character(&self, id: &str) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, Character::restore)
            .await
    }

    /// Returns every version of the character chain containing `id`, ordered
    /// oldest to newest. Walks back to the chain's root via `previous_version`,
    /// then forward via `next_version`. Returns an empty vector if `id` is
    /// missing, and stops walking at the first dangling link.
    pub async fn character_versions(&self, id: &str) -> Result<Vec<Character>, DatabaseError> {
        let Some(start) = self.character(id).await? else {
            return Ok(Vec::new());
        };
        let mut root = start;
        while let Some(previous_id) = root.previous_version().map(str::to_owned) {
            match self.character(&previous_id).await? {
                Some(previous) => root = previous,
                None => break,
            }
        }
        let mut chain = vec![root.clone()];
        let mut current = root;
        while let Some(next_id) = current.next_version().map(str::to_owned) {
            match self.character(&next_id).await? {
                Some(next) => {
                    chain.push(next.clone());
                    current = next;
                }
                None => break,
            }
        }
        Ok(chain)
    }

    /// Rolls a character back to the content of an older version `target_id`.
    ///
    /// Creates a new head atop `head_id` with the target version's content (see
    /// [`Character::rollback_to`]), keeping the head's accumulated stats, then
    /// supersedes the head with it. Returns the new head, or `Ok(None)` if either
    /// the head or the target version is missing.
    pub async fn rollback_character(
        &self,
        head_id: &str,
        target_id: &str,
        editor: UserId,
    ) -> Result<Option<Character>, DatabaseError> {
        let Some(target) = self.character(target_id).await? else {
            return Ok(None);
        };
        let Some(mut head) = self.character(head_id).await? else {
            return Ok(None);
        };
        head.rollback_to(editor, &target);
        let new_id = head.id().to_owned();
        self.insert_character(head.clone()).await?;
        self.supersede_character(new_id, head_id).await?;
        Ok(Some(head))
    }

    /// Sets the `next_version` field on the given old character ID to point to the given new character ID.
    pub async fn supersede_character(
        &self,
        new_id: String,
        old_id: &str,
    ) -> Result<Option<Character>, DatabaseError> {
        let superseded = self
            .mutate_character(old_id, UpdateSnafu, |character| {
                character.set_next_version(new_id);
            })
            .await?
            .with_context(|| NoCharacterSnafu {
                found: old_id.to_owned(),
                span: 0..old_id.len(),
            })?;
        Ok(Some(superseded))
    }

    /// Sets a character's AI model settings override.
    pub async fn set_character_model_settings(
        &self,
        id: &str,
        model_settings: CharacterModelSettings,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.set_model_settings(model_settings);
        })
        .await
    }

    /// Links (or, with `None`, clears) a character's `ElevenLabs` voice. Returns the
    /// updated character, or `Ok(None)` if the character is missing.
    pub async fn set_character_voice(
        &self,
        id: &str,
        voice: Option<String>,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.set_voice(voice);
        })
        .await
    }

    /// Sets a character's embed color. Returns the updated character, or `Ok(None)`
    /// if the character is missing.
    pub async fn set_character_color(
        &self,
        id: &str,
        color: Color,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.set_color(color);
        })
        .await
    }

    /// Appends an example-message pair (an optional user line and the character's
    /// response) to a character. Returns the updated character, or `Ok(None)` if the
    /// character is missing.
    pub async fn add_character_example(
        &self,
        id: &str,
        user: Option<String>,
        response: String,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.add_example_message(user, response);
        })
        .await
    }

    /// Removes the example-message pair at `index` (zero-based) from a character.
    /// Returns the updated character, or `Ok(None)` if the character is missing; an
    /// out-of-range index leaves the examples untouched.
    pub async fn remove_character_example(
        &self,
        id: &str,
        index: usize,
    ) -> Result<Option<Character>, DatabaseError> {
        self.mutate_character(id, UpdateSnafu, |character| {
            character.remove_example_message(index);
        })
        .await
    }

    /// Records a character spawn (a new conversation) for the given user.
    ///
    /// The stats land on the character's latest version (walking the version
    /// chain past any edits), so they carry across edits even when the
    /// conversation is pinned to an older version. Best-effort: warns and
    /// returns `Ok` if the character is missing.
    pub async fn record_character_spawn(
        &self,
        id: &str,
        user: UserId,
    ) -> Result<(), DatabaseError> {
        self.record_on_latest_version(id, "a spawn", |character| character.record_spawn(user))
            .await
    }

    /// Records the words and tokens a character generated in a single reply.
    ///
    /// The stats land on the character's latest version (walking the version
    /// chain past any edits), so they carry across edits even when the
    /// conversation is pinned to an older version. Best-effort: warns and
    /// returns `Ok` if the character is missing.
    pub async fn record_character_generation(
        &self,
        id: &str,
        words: u32,
        tokens: u32,
    ) -> Result<(), DatabaseError> {
        self.record_on_latest_version(id, "generation", |character| {
            character.record_generation(words, tokens);
        })
        .await
    }

    /// Walks `id` to its latest version and applies `apply` to that character,
    /// writing the result back. `label` names the stat for the missing-character
    /// warning. Best-effort: warns and returns `Ok` if the character is missing.
    async fn record_on_latest_version(
        &self,
        id: &str,
        label: &str,
        apply: impl FnOnce(&mut Character),
    ) -> Result<(), DatabaseError> {
        let Some(mut character) = self.character(id).await? else {
            warn!("tried to record {label} for a missing character: {id}");
            return Ok(());
        };
        while let Some(next_id) = character.next_version().map(str::to_owned) {
            let Some(next) = self.character(&next_id).await? else {
                break;
            };
            character = next;
        }
        apply(&mut character);
        self.write_json(&self.character_path(character.id()), &character)
            .await
            .context(UpdateSnafu)
    }
}
