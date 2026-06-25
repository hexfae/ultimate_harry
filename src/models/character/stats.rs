//! Usage-stat recording and formatting for [`Character`]: bumping conversation
//! and generation counters and rendering them for the leaderboard.
//!
//! Split out from the model in `character.rs`; as a child module this can still
//! reach `Character`'s private fields.

use super::Character;
use crate::database::Database;
use core::fmt::Write as _;
use jiff::Zoned;
use serenity::all::UserId;
use tracing::warn;

/// Compact date format for the "senast använd" leaderboard column.
///
/// See [`jiff::fmt::strtime`] for formatting details.
const LATEST_CONVERSATION_FORMAT: &str = "%Y-%m-%d %H:%M";

#[expect(
    clippy::multiple_inherent_impl,
    reason = "the usage-stat recording and formatting is split into this child module to separate stats from the Character model"
)]
impl Character {
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

    /// Returns the character's lifetime generation totals, as `{words} ord, {tokens} tokens`.
    #[must_use]
    pub fn formatted_generation(&self) -> String {
        format!(
            "{} ord, {} tokens",
            self.words_generated, self.tokens_generated
        )
    }
}

/// Tests for the character generation-stat formatting.
#[cfg(test)]
mod tests {
    use super::Character;
    use serenity::all::UserId;

    /// Builds a character carrying the given lifetime generation totals.
    fn character(words: u32, tokens: u32) -> Character {
        Character::builder()
            .id("id".to_owned())
            .name("Harry")
            .greeting("hej")
            .creator(UserId::new(1))
            .words_generated(words)
            .tokens_generated(tokens)
            .build()
    }

    /// A fresh character with no generation yet reads as all zeroes.
    #[test]
    fn formatted_generation_shows_zero_when_nothing_generated() {
        assert_eq!(
            character(0, 0).formatted_generation(),
            "0 ord, 0 tokens",
            "an unused character shows zero words and tokens"
        );
    }

    /// The accumulated word and token totals are both rendered.
    #[test]
    fn formatted_generation_shows_word_and_token_totals() {
        assert_eq!(
            character(1234, 5678).formatted_generation(),
            "1234 ord, 5678 tokens",
            "both the word and token totals are shown"
        );
    }
}
