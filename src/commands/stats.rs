//! The bot's Discord slash command for showing the character usage leaderboard.

use core::fmt::Write as _;

use snafu::ResultExt as _;
use tracing::warn;

use crate::{
    AppResult, Context, error::SendMessageSnafu, models::character::Character,
    traits::SayEphemeral as _,
};

/// Visar en topplista över gubbarnas statistik.
#[poise::command(slash_command, rename = "statistik")]
pub async fn stats(ctx: Context<'_>) -> AppResult {
    let characters = ctx.data().db.characters_by_usage().await?;
    ctx.say_ephemeral(format_leaderboard(&characters))
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}

/// Builds the Swedish leaderboard text from characters already sorted by usage.
///
/// Each line ranks a character by conversation count and also surfaces the words
/// and tokens it has generated. Returns an empty-state message when there is
/// nothing to rank.
fn format_leaderboard(characters: &[Character]) -> String {
    if characters.is_empty() {
        return "Det finns inga gubbar än.".to_owned();
    }

    let mut text = String::new();
    for (index, character) in characters.iter().enumerate() {
        let rank = index.saturating_add(1);
        let conversations = character.conversations_had();
        let words = abbreviate(character.words_generated());
        let tokens = abbreviate(character.tokens_generated());
        let last_used = character.formatted_latest_conversation();
        let newline = if text.is_empty() { "" } else { "\n" };
        if let Err(why) = write!(
            text,
            "{newline}{rank}. {character}: {conversations} konversationer, {words} ord, {tokens} tokens, senast använd: {last_used}"
        ) {
            warn!("error while writing leaderboard line: {why}");
        }
    }
    text
}

/// Formats a count compactly, abbreviating thousands as e.g. `1.2k` and
/// millions as e.g. `1.5M`.
fn abbreviate(count: u32) -> String {
    if count < 1000 {
        return count.to_string();
    }
    if count >= 1_000_000 {
        let millions = count.checked_div(1_000_000).unwrap_or(0);
        let hundred_thousands = count
            .checked_rem(1_000_000)
            .and_then(|remainder| remainder.checked_div(100_000))
            .unwrap_or(0);
        return format!("{millions}.{hundred_thousands}M");
    }
    let thousands = count.checked_div(1000).unwrap_or(0);
    let hundreds = count
        .checked_rem(1000)
        .and_then(|remainder| remainder.checked_div(100))
        .unwrap_or(0);
    format!("{thousands}.{hundreds}k")
}

/// Tests for the leaderboard formatting helpers.
#[cfg(test)]
mod tests {
    use super::{abbreviate, format_leaderboard};
    use crate::models::character::Character;
    use core::slice::from_ref;
    use jiff::Zoned;
    use serenity::all::UserId;

    /// Builds a character carrying the given name, emoji, and generation stats.
    fn character(name: &str, emoji: &str, conversations: u32, words: u32, tokens: u32) -> Character {
        Character::builder()
            .id(name.to_owned())
            .name(name)
            .emoji(emoji.to_owned())
            .greeting("hej")
            .creator(UserId::new(1))
            .conversations_had(conversations)
            .words_generated(words)
            .tokens_generated(tokens)
            .build()
    }

    /// An empty leaderboard renders the empty-state message, not a bare header.
    #[test]
    fn an_empty_leaderboard_shows_the_empty_state() {
        assert_eq!(format_leaderboard(&[]), "Det finns inga gubbar än.");
    }

    /// A never-used character is ranked first, showing its stats and `aldrig`.
    #[test]
    fn a_single_character_is_ranked_with_its_stat_columns() {
        let text = format_leaderboard(&[character("Harry", "🎭", 42, 1234, 3402)]);
        assert_eq!(
            text,
            "1. 🎭 Harry: 42 konversationer, 1.2k ord, 3.4k tokens, senast använd: aldrig"
        );
    }

    /// A character with a last-used time renders it in the compact date format.
    #[test]
    fn the_last_used_time_is_shown_when_present() {
        let parsed = "2026-06-22T19:33:00[UTC]".parse::<Zoned>();
        assert!(
            parsed.is_ok(),
            "the test timestamp must parse (jiff tzdb available)"
        );
        let Ok(time) = parsed else {
            return;
        };
        let last_used = Character::builder()
            .id("Harry".to_owned())
            .name("Harry")
            .emoji("🎭".to_owned())
            .greeting("hej")
            .creator(UserId::new(1))
            .conversations_had(42_u32)
            .latest_conversation(time)
            .build();

        assert_eq!(
            format_leaderboard(from_ref(&last_used)),
            "1. 🎭 Harry: 42 konversationer, 0 ord, 0 tokens, senast använd: 2026-06-22 19:33"
        );
    }

    /// Characters are numbered in the order given, preserving the usage sort.
    #[test]
    fn characters_are_numbered_in_the_given_order() {
        let text = format_leaderboard(&[
            character("Harry", "🎭", 42, 0, 0),
            character("Gandalf", "🧙", 30, 0, 0),
            character("Robot", "🤖", 12, 0, 0),
        ]);
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines.iter().any(|line| line.starts_with("1. 🎭 Harry:")),
            "the first character is ranked first"
        );
        assert!(
            lines.iter().any(|line| line.starts_with("2. 🧙 Gandalf:")),
            "the second character is ranked second"
        );
        assert!(
            lines.iter().any(|line| line.starts_with("3. 🤖 Robot:")),
            "the third character is ranked third"
        );
    }

    /// Counts below a thousand are shown verbatim, with no abbreviation.
    #[test]
    fn abbreviate_keeps_small_counts_verbatim() {
        assert_eq!(abbreviate(0), "0");
        assert_eq!(abbreviate(42), "42");
        assert_eq!(abbreviate(999), "999");
    }

    /// Counts of a thousand or more are abbreviated to one decimal place.
    #[test]
    fn abbreviate_compacts_thousands() {
        assert_eq!(abbreviate(1000), "1.0k");
        assert_eq!(abbreviate(1234), "1.2k");
        assert_eq!(abbreviate(3402), "3.4k");
        assert_eq!(abbreviate(12345), "12.3k");
    }

    /// Counts of a million or more are abbreviated with an `M` suffix, not `k`.
    #[test]
    fn abbreviate_compacts_millions() {
        assert_eq!(abbreviate(1_000_000), "1.0M");
        assert_eq!(abbreviate(1_500_000), "1.5M");
        assert_eq!(abbreviate(12_345_678), "12.3M");
    }
}
