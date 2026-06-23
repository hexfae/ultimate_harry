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
    let editor_name = match character.latest_editor() {
        Some(editor_id) => Some(db.substitute_name(editor_id).await),
        None => None,
    };
    let created = character.created_at().strftime(GOOD_DATE_FORMAT).to_string();
    let edited = character
        .edited_at()
        .map(|time| time.strftime(GOOD_DATE_FORMAT).to_string());

    let mut embed = CreateEmbed::new()
        .title(title)
        .field(
            "Hälsning",
            truncate_field(character.greeting().to_owned()),
            true,
        )
        .field("Konversationer", conversations, true)
        .field("Version", character.version().saturating_add(1).to_string(), true);

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
        embed = embed.field("System Prompt", truncate_field(system_prompt.to_owned()), true);
    }

    if let Some(scenario) = character.scenario() {
        embed = embed.field("Scenario", truncate_field(scenario.to_owned()), true);
    }

    embed = embed.field("Skapare", creator_name, true);

    if let Some(name) = editor_name {
        embed = embed.field("Redigerare", name, true);
    }

    embed = embed.field("Skapad", created, false);
    if let Some(edited_text) = edited {
        embed = embed.field("Redigerad", edited_text, false);
    }
    embed = embed
        .field("ID", character.id().to_owned(), false)
        .footer(CreateEmbedFooter::new(footer_text.into()));

    if let Some(avatar) = character.avatar() {
        embed = embed.thumbnail(avatar.to_owned());
    }
    if let Some(color) = character.color() {
        embed = embed.color(color);
    }
    if let Some(description) = character.description() {
        embed = embed.description(description.to_owned());
    }
    embed
}
