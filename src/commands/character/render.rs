//! Renders a character as a Discord embed for the character paginators.

use poise::serenity_prelude::all::{CreateEmbed, CreateEmbedFooter};

use crate::{database::Database, models::character::Character};

/// E.g. `16 May, Friday, 2025 | 17:41:14 | 2025-05-16`.
///
/// See [`jiff::fmt::strtime`] for formatting details.
const GOOD_DATE_FORMAT: &str = "%e %B, %A, %G | %T | %F";

/// Discord's per-field value limit for embeds; longer values are rejected.
const FIELD_VALUE_LIMIT: usize = 1024;

/// Truncates a character field to Discord's embed field value limit, cutting on
/// a character boundary and appending an ellipsis when anything was removed.
fn truncate_field(value: String) -> String {
    if value.chars().count() <= FIELD_VALUE_LIMIT {
        return value;
    }
    let mut truncated: String = value
        .chars()
        .take(FIELD_VALUE_LIMIT.saturating_sub(1))
        .collect();
    truncated.push('…');
    truncated
}

/// Builds a Discord embed presenting the character with the given footer text.
///
/// Contains all character information such as name, greeting, personality,
/// scenario, and metadata.
pub async fn character_embed<F: Into<String>>(
    character: &Character,
    footer_text: F,
    db: &Database,
) -> CreateEmbed<'static> {
    let title = character.to_string();
    let conversations = character.formatted_conversations_had(db).await;
    let creator_name = db.substitute_name(character.creator()).await;
    let mut editor_names = Vec::new();
    for editor_id in character.all_editors() {
        editor_names.push(db.substitute_name(*editor_id).await);
    }
    let latest_editor_name = match character.latest_editor() {
        Some(editor_id) => Some(db.substitute_name(editor_id).await),
        None => None,
    };
    let deleter_name = match character.deleted_by() {
        Some(deleter_id) => Some(db.substitute_name(deleter_id).await),
        None => None,
    };
    let created = character
        .created_at()
        .strftime(GOOD_DATE_FORMAT)
        .to_string();
    let edited = character
        .edited_at()
        .map(|time| time.strftime(GOOD_DATE_FORMAT).to_string());
    let deleted = character
        .deleted_at()
        .map(|time| time.strftime(GOOD_DATE_FORMAT).to_string());

    let mut embed = CreateEmbed::new()
        .title(title)
        .field(
            "Hälsning",
            truncate_field(character.greeting().to_owned()),
            true,
        )
        .field("Konversationer", conversations, true)
        .field(
            "Version",
            character.version().saturating_add(1).to_string(),
            true,
        )
        .field(
            "Senast använd",
            character.formatted_latest_conversation(),
            true,
        )
        .field("Genererat", character.formatted_generation(), true);

    if let Some(nickname) = character.nickname() {
        embed = embed.field("Smeknamn", truncate_field(nickname.to_owned()), true);
    }

    if let Some(personality) = character.personality() {
        embed = embed.field("Personlighet", truncate_field(personality.to_owned()), true);
    }

    if let Some(prompt) = character.prompt() {
        embed = embed.field("Prompt", truncate_field(prompt.to_owned()), true);
    }

    if let Some(system_prompt) = character.system_prompt() {
        embed = embed.field(
            "Systemprompt",
            truncate_field(system_prompt.to_owned()),
            true,
        );
    }

    if let Some(scenario) = character.scenario() {
        embed = embed.field("Scenario", truncate_field(scenario.to_owned()), true);
    }

    embed = embed.field("Skapare", creator_name, true);

    if let Some(name) = latest_editor_name {
        embed = embed.field("Senaste redigerare", name, true);
    }

    if !editor_names.is_empty() {
        embed = embed.field("Redigerare", editor_names.join(", "), true);
    }

    embed = embed.field("Skapad", created, false);
    if let Some(edited_text) = edited {
        embed = embed.field("Redigerad", edited_text, false);
    }

    if let Some(name) = deleter_name {
        embed = embed.field("Dödad av", name, true);
    }
    if let Some(deleted_text) = deleted {
        embed = embed.field("Dödad", deleted_text, false);
    }

    embed = embed
        .field("ID", character.id().to_owned(), false)
        .footer(CreateEmbedFooter::new(footer_text.into()));

    if let Some(avatar) = character.avatar() {
        embed = embed.thumbnail(avatar.to_owned(), None);
    }
    if let Some(color) = character.color() {
        embed = embed.color(color);
    }
    if let Some(description) = character.description() {
        embed = embed.description(description.to_owned());
    }
    embed
}

/// Tests for the stats and metadata surfaced on the character embed.
#[cfg(test)]
mod tests {
    use super::{FIELD_VALUE_LIMIT, character_embed, truncate_field};
    use crate::{database::Database, models::character::Character};
    use alloc::collections::BTreeSet;
    use serenity::all::UserId;

    /// A value within the field limit is returned verbatim; an over-limit value is
    /// cut to the limit on a char boundary and gains an ellipsis.
    #[test]
    fn truncate_field_caps_overlong_values_on_a_char_boundary() {
        let at_limit = "a".repeat(FIELD_VALUE_LIMIT);
        assert_eq!(
            truncate_field(at_limit.clone()),
            at_limit,
            "a value exactly at the limit is returned unchanged"
        );

        let over_limit = "a".repeat(FIELD_VALUE_LIMIT.saturating_add(1));
        let truncated = truncate_field(over_limit);
        assert_eq!(
            truncated.chars().count(),
            FIELD_VALUE_LIMIT,
            "an over-limit value is cut to the field limit"
        );
        assert!(
            truncated.ends_with('…'),
            "the truncated value ends with an ellipsis"
        );

        let multibyte = "å".repeat(FIELD_VALUE_LIMIT.saturating_add(5));
        let cut = truncate_field(multibyte);
        assert_eq!(
            cut.chars().count(),
            FIELD_VALUE_LIMIT,
            "a multibyte value is cut on a char boundary to the field limit"
        );
        assert!(
            cut.ends_with('…'),
            "a multibyte value also gains an ellipsis"
        );
    }

    /// Builds a minimal character with the given ID and name.
    fn character(id: &str, name: &str) -> Character {
        Character::builder()
            .id(id.to_owned())
            .name(name)
            .greeting("hej")
            .creator(UserId::new(1))
            .build()
    }

    /// The embed surfaces the generation totals and last-used time the bot
    /// already tracks, both of which were previously only shown on the leaderboard.
    #[tokio::test]
    async fn embed_shows_generation_totals_and_last_used() {
        let Ok(db) = Database::temporary().await else {
            return;
        };
        let character = Character::builder()
            .id("char-id".to_owned())
            .name("Harry")
            .greeting("hej")
            .creator(UserId::new(1))
            .words_generated(1234_u32)
            .tokens_generated(5678_u32)
            .build();

        let embed = character_embed(&character, "footer", &db).await;
        let json = serde_json::to_string(&embed).unwrap_or_default();

        assert!(
            json.contains("Genererat"),
            "the embed has a generation field"
        );
        assert!(
            json.contains("1234 ord, 5678 tokens"),
            "the generation field shows the word and token totals"
        );
        assert!(
            json.contains("Senast använd"),
            "the embed has a last-used field"
        );
        assert!(
            json.contains("aldrig"),
            "a never-used character reports last-used as 'aldrig'"
        );
    }

    /// The embed lists every editor by resolved name, not just the latest one.
    #[tokio::test]
    async fn embed_lists_every_editor_by_name() {
        let Ok(db) = Database::temporary().await else {
            return;
        };
        assert!(
            db.upsert_user_name(UserId::new(2), "Anna".to_owned())
                .await
                .is_ok(),
            "saving the first editor name should succeed"
        );
        assert!(
            db.upsert_user_name(UserId::new(3), "Bertil".to_owned())
                .await
                .is_ok(),
            "saving the second editor name should succeed"
        );
        let editors: BTreeSet<UserId> = [UserId::new(2), UserId::new(3)].into_iter().collect();
        let character = Character::builder()
            .id("char-id".to_owned())
            .name("Harry")
            .greeting("hej")
            .creator(UserId::new(1))
            .all_editors(editors)
            .latest_editor(UserId::new(3))
            .build();

        let embed = character_embed(&character, "footer", &db).await;
        let json = serde_json::to_string(&embed).unwrap_or_default();

        assert!(
            json.contains("Redigerare"),
            "the embed has an editors field"
        );
        assert!(json.contains("Anna"), "the first editor is listed by name");
        assert!(
            json.contains("Bertil"),
            "the second editor is listed by name"
        );
        assert!(
            json.contains("Senaste redigerare"),
            "the latest editor is shown as its own field"
        );
    }

    /// A deleted character's embed surfaces who deleted it and when.
    #[tokio::test]
    async fn embed_shows_deletion_metadata_when_deleted() {
        let Ok(db) = Database::temporary().await else {
            return;
        };
        assert!(
            db.upsert_user_name(UserId::new(4), "Cesar".to_owned())
                .await
                .is_ok(),
            "saving the deleter name should succeed"
        );
        let mut character = character("char-id", "Harry");
        character.mark_deleted(UserId::new(4));

        let embed = character_embed(&character, "footer", &db).await;
        let json = serde_json::to_string(&embed).unwrap_or_default();

        assert!(
            json.contains("Dödad av"),
            "a deleted character shows who deleted it"
        );
        assert!(json.contains("Cesar"), "the deleter is shown by name");
    }

    /// A visible character's embed carries no deletion fields.
    #[tokio::test]
    async fn embed_omits_deletion_metadata_when_visible() {
        let Ok(db) = Database::temporary().await else {
            return;
        };
        let character = character("char-id", "Harry");

        let embed = character_embed(&character, "footer", &db).await;
        let json = serde_json::to_string(&embed).unwrap_or_default();

        assert!(
            !json.contains("Dödad"),
            "a visible character has no deletion fields"
        );
    }
}
