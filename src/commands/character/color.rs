//! The bot's Discord slash command for setting a character's display color.

use crate::{
    AppResult, Context,
    commands::{autocomplete, first_character_or_notify},
    error::SendMessageSnafu,
    phrases,
    traits::SayEphemeral as _,
};
use poise::serenity_prelude::all::Color;
use snafu::ResultExt as _;

/// Parses a hex color (`#rrggbb` or `rrggbb`, case-insensitive), or `None` if invalid.
fn parse_color(input: &str) -> Option<Color> {
    let trimmed = input.trim();
    let digits = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16).ok().map(Color::new)
}

/// Ställer in en gubbes färg.
#[poise::command(slash_command, rename = "färg")]
pub async fn color(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
    #[rename = "färg"]
    #[description = "Hexfärg, t.ex. #ff0000 (lämna tomt för att visa nuvarande)"]
    color: Option<String>,
) -> AppResult {
    let db = &ctx.data().db;
    let Some(character) = first_character_or_notify(ctx, name).await? else {
        return Ok(());
    };

    let Some(input) = color else {
        let current = character
            .color()
            .map_or_else(|| "ingen".to_owned(), |colour| format!("#{}", colour.hex()));
        ctx.say_ephemeral(format!("färg: {current}"))
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };

    let Some(parsed) = parse_color(&input) else {
        ctx.say_ephemeral("Det där var ingen färg jag känner igen. Ge mig en hexfärg, typ #ff0000.")
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    };

    db.set_character_color(character.id(), parsed).await?;
    ctx.say_ephemeral(phrases::done()).await.context(SendMessageSnafu)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_color;
    use serenity::all::Color;

    /// A plain six-digit hex string parses to the matching color.
    #[test]
    fn parses_a_plain_six_digit_hex() {
        assert_eq!(parse_color("ff0000"), Some(Color::new(0x00ff_0000)));
    }

    /// A leading `#` is accepted and ignored.
    #[test]
    fn parses_a_hash_prefixed_hex() {
        assert_eq!(parse_color("#00ff00"), Some(Color::new(0x0000_ff00)));
    }

    /// Hex digits are case-insensitive.
    #[test]
    fn parses_case_insensitively() {
        assert_eq!(parse_color("#FFFFFF"), Some(Color::new(0x00ff_ffff)));
    }

    /// Surrounding whitespace is trimmed before parsing.
    #[test]
    fn ignores_surrounding_whitespace() {
        assert_eq!(parse_color("  #0000ff  "), Some(Color::new(0x0000_00ff)));
    }

    /// An empty string is not a valid color.
    #[test]
    fn rejects_an_empty_string() {
        assert_eq!(parse_color(""), None);
    }

    /// Three-digit shorthand is not supported.
    #[test]
    fn rejects_a_three_digit_shorthand() {
        assert_eq!(parse_color("#fff"), None);
    }

    /// More than six digits is rejected.
    #[test]
    fn rejects_too_many_digits() {
        assert_eq!(parse_color("#ff00000"), None);
    }

    /// Non-hex characters are rejected.
    #[test]
    fn rejects_non_hex_characters() {
        assert_eq!(parse_color("gg0000"), None);
    }

    /// A leading sign is rejected even at the right length.
    #[test]
    fn rejects_a_leading_sign() {
        assert_eq!(parse_color("+f0000"), None);
    }
}
