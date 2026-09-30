//! Per-user records: the display name and reaction emoji shown in replies.

use serenity::all::{ReactionType, UserId};
use snafu::ResultExt as _;

use super::store::{read_json, read_or, scan_dir};
use super::{Database, DatabaseError, GetSnafu, InsertSnafu};
use crate::models::config::UserPrefs;

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the per-user record operations are split into this child module to separate them from the other record types"
)]
impl Database {
    /// Loads the given user's preferences (or a fresh default), applies `apply`, and writes
    /// them back. Mirrors [`Self::mutate_character`] for the single-file user records.
    async fn mutate_user_prefs<T: Into<UserId>>(
        &self,
        user_id: T,
        apply: impl FnOnce(&mut UserPrefs),
    ) -> Result<(), DatabaseError> {
        let id = user_id.into().to_string();
        let path = self.user_path(&id);
        let mut prefs = read_json::<UserPrefs>(&path)
            .await
            .context(GetSnafu)?
            .unwrap_or_else(|| UserPrefs::new(id));
        apply(&mut prefs);
        self.write_json(&path, &prefs).await.context(InsertSnafu)
    }

    /// Updates or inserts a user's emoji, preserving their stored display name.
    pub async fn upsert_user_emoji<T: Into<UserId>>(
        &self,
        user_id: T,
        emoji: ReactionType,
    ) -> Result<(), DatabaseError> {
        self.mutate_user_prefs(user_id, |prefs| prefs.emoji = Some(emoji))
            .await
    }

    /// Returns every user's set emoji, paired with their Discord user ID.
    pub async fn user_emoji(&self) -> Result<Vec<(String, ReactionType)>, DatabaseError> {
        let prefs: Vec<UserPrefs> = scan_dir(&self.users_dir()).await.context(GetSnafu)?;
        Ok(prefs
            .into_iter()
            .filter_map(|pref| pref.emoji.map(|emoji| (pref.user_id, emoji)))
            .collect())
    }

    /// Returns a user's display name by their Discord user ID, defaulting to "User".
    pub async fn substitute_name<T: Into<UserId>>(&self, user_id: T) -> String {
        let path = self.user_path(&user_id.into().to_string());
        read_or(&path, "substitute name", UserPrefs::default)
            .await
            .name
            .unwrap_or_else(|| "User".to_owned())
    }

    /// Updates or inserts a user's display name, preserving their stored emoji.
    pub async fn upsert_user_name<T: Into<UserId>>(
        &self,
        user_id: T,
        name: String,
    ) -> Result<(), DatabaseError> {
        self.mutate_user_prefs(user_id, |prefs| prefs.name = Some(name))
            .await
    }
}
