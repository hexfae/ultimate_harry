//! The bot's Discord slash command for showing the character usage leaderboard.

use alloc::collections::BTreeMap;
use core::fmt::Write as _;

use serenity::all::UserId;
use snafu::ResultExt as _;
use tracing::warn;

use crate::{
    AppResult, Context, error::SendMessageSnafu, models::character::Character,
    traits::SayEphemeral as _,
};

/// Swedish header for the character section of the stats message.
const CHARACTER_HEADER: &str = "**Gubbar**";
/// Swedish header for the per-user section of the stats message.
const USER_HEADER: &str = "**Användare**";

/// Visar en topplista över gubbarnas statistik.
#[poise::command(slash_command, rename = "statistik")]
pub async fn stats(ctx: Context<'_>) -> AppResult {
    let characters = ctx.data().db.characters_by_usage().await?;
    let user_stats = aggregate_user_stats(&characters);

    let mut names = BTreeMap::new();
    for stat in &user_stats {
        let name = ctx.data().db.substitute_name(stat.user).await;
        names.insert(stat.user, name);
    }

    let character_board = format_leaderboard(&characters);
    let user_board = format_user_leaderboard(&user_stats, &names);
    let text = format!("{CHARACTER_HEADER}\n{character_board}\n\n{USER_HEADER}\n{user_board}");

    ctx.say_ephemeral(text).await.context(SendMessageSnafu)?;
    Ok(())
}

/// One user's conversation stats aggregated across every character.
#[derive(Debug, PartialEq, Eq)]
struct UserStats {
    /// The user these stats belong to.
    user: UserId,
    /// The user's total conversations across all characters.
    conversations: u32,
    /// The user's characters, each with its per-user conversation count, ordered
    /// most-used first.
    characters: Vec<(String, u32)>,
}

/// Pivots the characters' per-user conversation counts into one entry per user.
///
/// Each entry carries the user's total conversations and which characters they
/// talked to. Users are ranked by total conversations descending, ties broken by
/// user id; each user's characters are ordered by count descending, ties broken
/// by display.
fn aggregate_user_stats(characters: &[Character]) -> Vec<UserStats> {
    let mut totals: BTreeMap<UserId, (u32, Vec<(String, u32)>)> = BTreeMap::new();
    for character in characters {
        let display = character.to_string();
        for (user, count) in character.conversations_per_user() {
            let entry = totals.entry(*user).or_insert_with(|| (0, Vec::new()));
            entry.0 = entry.0.saturating_add(*count);
            entry.1.push((display.clone(), *count));
        }
    }

    let mut stats: Vec<UserStats> = totals
        .into_iter()
        .map(|(user, (conversations, mut user_characters))| {
            user_characters
                .sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
            UserStats {
                user,
                conversations,
                characters: user_characters,
            }
        })
        .collect();
    stats.sort_by(|left, right| {
        right
            .conversations
            .cmp(&left.conversations)
            .then_with(|| left.user.cmp(&right.user))
    });
    stats
}

/// Builds the Swedish per-user leaderboard text from aggregated user stats.
///
/// Each line ranks a user by total conversations and lists which characters they
/// talked to. `names` maps each user to their resolved display name; a user
/// absent from it falls back to a mention. Returns an empty-state message when no
/// user has used any character.
fn format_user_leaderboard(stats: &[UserStats], names: &BTreeMap<UserId, String>) -> String {
    if stats.is_empty() {
        return "Ingen har använt någon gubbe än.".to_owned();
    }

    let mut text = String::new();
    for (index, stat) in stats.iter().enumerate() {
        let rank = index.saturating_add(1);
        let user = stat.user;
        let conversations = stat.conversations;
        let name = names
            .get(&user)
            .cloned()
            .unwrap_or_else(|| format!("<@{user}>"));
        let breakdown = stat
            .characters
            .iter()
            .map(|(display, count)| format!("{display}: {count}"))
            .collect::<Vec<_>>()
            .join(", ");
        let newline = if text.is_empty() { "" } else { "\n" };
        if let Err(why) = write!(
            text,
            "{newline}{rank}. {name}: {conversations} konversationer ({breakdown})"
        ) {
            warn!("error while writing user leaderboard line: {why}");
        }
    }
    text
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
    use super::{
        UserStats, abbreviate, aggregate_user_stats, format_leaderboard, format_user_leaderboard,
    };
    use crate::models::character::Character;
    use alloc::collections::BTreeMap;
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

    /// Builds a character whose per-user conversation counts are the given pairs.
    fn character_with_users(name: &str, emoji: &str, users: &[(u64, u32)]) -> Character {
        let map: BTreeMap<UserId, u32> = users
            .iter()
            .map(|(id, count)| (UserId::new(*id), *count))
            .collect();
        Character::builder()
            .id(name.to_owned())
            .name(name)
            .emoji(emoji.to_owned())
            .greeting("hej")
            .creator(UserId::new(1))
            .conversations_had_with_user(map)
            .build()
    }

    /// A user stat entry, built for the formatting tests.
    fn user_stats(user: u64, conversations: u32, characters: &[(&str, u32)]) -> UserStats {
        UserStats {
            user: UserId::new(user),
            conversations,
            characters: characters
                .iter()
                .map(|(display, count)| ((*display).to_owned(), *count))
                .collect(),
        }
    }

    /// Without any characters there are no per-user stats to aggregate.
    #[test]
    fn aggregate_user_stats_is_empty_without_characters() {
        assert!(aggregate_user_stats(&[]).is_empty());
    }

    /// A character nobody has talked to contributes no per-user stats.
    #[test]
    fn aggregate_user_stats_ignores_characters_with_no_user_data() {
        let characters = [character_with_users("Harry", "🎭", &[])];
        assert!(aggregate_user_stats(&characters).is_empty());
    }

    /// Each user's conversations are summed across every character, ranked by
    /// total descending, with their characters listed most-used first.
    #[test]
    fn aggregate_user_stats_pivots_conversations_across_characters() {
        let characters = [
            character_with_users("Harry", "🎭", &[(1, 30), (2, 5)]),
            character_with_users("Gandalf", "🧙", &[(1, 22)]),
            character_with_users("Robot", "🤖", &[(2, 7)]),
        ];
        assert_eq!(
            aggregate_user_stats(&characters),
            vec![
                user_stats(1, 52, &[("🎭 Harry", 30), ("🧙 Gandalf", 22)]),
                user_stats(2, 12, &[("🤖 Robot", 7), ("🎭 Harry", 5)]),
            ]
        );
    }

    /// Users tied on total conversations are ordered by user id, ascending.
    #[test]
    fn aggregate_user_stats_breaks_total_ties_by_user_id() {
        let characters = [character_with_users("Harry", "🎭", &[(2, 10), (1, 10)])];
        assert_eq!(
            aggregate_user_stats(&characters),
            vec![
                user_stats(1, 10, &[("🎭 Harry", 10)]),
                user_stats(2, 10, &[("🎭 Harry", 10)]),
            ]
        );
    }

    /// A user's characters tied on count are ordered by display, ascending.
    #[test]
    fn aggregate_user_stats_breaks_character_ties_by_display() {
        let characters = [
            character_with_users("Zebra", "🅰", &[(1, 5)]),
            character_with_users("Apa", "🅰", &[(1, 5)]),
        ];
        assert_eq!(
            aggregate_user_stats(&characters),
            vec![user_stats(1, 10, &[("🅰 Apa", 5), ("🅰 Zebra", 5)])]
        );
    }

    /// An empty per-user leaderboard renders the empty-state message.
    #[test]
    fn an_empty_user_leaderboard_shows_the_empty_state() {
        assert_eq!(
            format_user_leaderboard(&[], &BTreeMap::new()),
            "Ingen har använt någon gubbe än."
        );
    }

    /// A user line shows the resolved name, total, and character breakdown.
    #[test]
    fn a_user_line_shows_total_and_character_breakdown() {
        let stats = [user_stats(1, 52, &[("🎭 Harry", 30), ("🧙 Gandalf", 22)])];
        let mut names = BTreeMap::new();
        names.insert(UserId::new(1), "Anna".to_owned());
        assert_eq!(
            format_user_leaderboard(&stats, &names),
            "1. Anna: 52 konversationer (🎭 Harry: 30, 🧙 Gandalf: 22)"
        );
    }

    /// Users are numbered in the given order, each with their resolved name.
    #[test]
    fn users_are_ranked_in_order_with_resolved_names() {
        let stats = [
            user_stats(1, 52, &[("🎭 Harry", 52)]),
            user_stats(2, 12, &[("🤖 Robot", 12)]),
        ];
        let mut names = BTreeMap::new();
        names.insert(UserId::new(1), "Anna".to_owned());
        names.insert(UserId::new(2), "Bob".to_owned());
        assert_eq!(
            format_user_leaderboard(&stats, &names),
            "1. Anna: 52 konversationer (🎭 Harry: 52)\n2. Bob: 12 konversationer (🤖 Robot: 12)"
        );
    }

    /// A user missing from the name map falls back to a Discord mention.
    #[test]
    fn a_missing_name_falls_back_to_a_mention() {
        let stats = [user_stats(7, 3, &[("🎭 Harry", 3)])];
        assert_eq!(
            format_user_leaderboard(&stats, &BTreeMap::new()),
            "1. <@7>: 3 konversationer (🎭 Harry: 3)"
        );
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
