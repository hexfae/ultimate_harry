//! Resolution of `:shortcode:` runs in user-submitted character text.
//!
//! Discord only expands shortcodes (custom `:cholol:` or unicode `:smile:`) in
//! the message composer as you type, never server-side. Character text arrives
//! through modals, which have no composer, so unresolved shortcodes would render
//! literally. We expand them ourselves when a modal is submitted: custom guild
//! emoji become their `<:name:id>` wire form and unicode shortcodes become the
//! literal emoji. Unrecognised runs are left untouched.

use crate::{
    ApplicationContext,
    models::modals::{
        CreateCharacterModal, EditCharacterModal, SecondCreateCharacterModal,
        SecondEditCharacterModal,
    },
};
use poise::serenity_prelude::Emoji;

/// Returns whether `ch` may appear inside a shortcode name.
///
/// Custom guild emoji names are alphanumeric plus underscore; unicode
/// shortcodes additionally use `+` and `-` (such as `:+1:` and `:e-mail:`).
const fn is_shortcode_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '_' | '+' | '-')
}

/// Renders a single shortcode `name` if it matches a custom guild emoji or a
/// known unicode emoji, otherwise returns `None`. A guild emoji wins over a
/// unicode shortcode of the same name.
fn render_shortcode(name: &str, guild_emojis: &[Emoji]) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    if let Some(emoji) = guild_emojis.iter().find(|emoji| &*emoji.name == name) {
        return Some(emoji.to_string());
    }
    emojis::get_by_shortcode(name).map(|emoji| emoji.as_str().to_owned())
}

/// Expands every `:shortcode:` run in `text` into its rendered emoji, leaving
/// unrecognised runs untouched.
fn resolve(text: &str, guild_emojis: &[Emoji]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut name = String::new();
    let mut in_code = false;
    for ch in text.chars() {
        if ch == ':' {
            if in_code {
                if let Some(rendered) = render_shortcode(&name, guild_emojis) {
                    out.push_str(&rendered);
                    in_code = false;
                } else {
                    // `name` was not an emoji: emit the opening colon and the
                    // run literally, then treat this colon as a fresh opener.
                    out.push(':');
                    out.push_str(&name);
                }
                name.clear();
            } else {
                in_code = true;
            }
        } else if in_code {
            if is_shortcode_char(ch) {
                name.push(ch);
            } else {
                // an interrupting character cannot belong to a shortcode, so
                // flush the candidate (and its opening colon) literally.
                out.push(':');
                out.push_str(&name);
                out.push(ch);
                name.clear();
                in_code = false;
            }
        } else {
            out.push(ch);
        }
    }
    if in_code {
        out.push(':');
        out.push_str(&name);
    }
    out
}

/// Resolves shortcodes in an optional field in place, leaving `None` untouched.
fn resolve_field(field: &mut Option<String>, guild_emojis: &[Emoji]) {
    if let Some(value) = field {
        *value = resolve(value, guild_emojis);
    }
}

/// Fetches the guild's custom emoji for shortcode resolution, falling back to an
/// empty list (so unicode shortcodes still resolve) when there is no guild or
/// the fetch fails.
pub async fn guild_emojis(ctx: ApplicationContext<'_>) -> Vec<Emoji> {
    let Some(guild_id) = ctx.guild_id() else {
        return Vec::new();
    };
    match guild_id.emojis(ctx.http()).await {
        Ok(emojis) => emojis,
        Err(error) => {
            tracing::warn!(%error, "failed to fetch guild emoji for shortcode resolution");
            Vec::new()
        }
    }
}

/// Expands shortcodes in every text field of the character creation modals. The
/// avatar is a URL and so is left untouched.
pub fn resolve_create_modals(
    first: &mut CreateCharacterModal,
    second: &mut SecondCreateCharacterModal,
    guild_emojis: &[Emoji],
) {
    first.name = resolve(&first.name, guild_emojis);
    first.greeting = resolve(&first.greeting, guild_emojis);
    resolve_field(&mut first.nickname, guild_emojis);
    resolve_field(&mut first.description, guild_emojis);
    resolve_field(&mut first.personality, guild_emojis);
    resolve_field(&mut second.emoji, guild_emojis);
    resolve_field(&mut second.system_prompt, guild_emojis);
    resolve_field(&mut second.prompt, guild_emojis);
    resolve_field(&mut second.scenario, guild_emojis);
}

/// Expands shortcodes in every text field of the character edit modals. The
/// avatar is a URL and so is left untouched.
pub fn resolve_edit_modals(
    first: &mut EditCharacterModal,
    second: &mut SecondEditCharacterModal,
    guild_emojis: &[Emoji],
) {
    resolve_field(&mut first.name, guild_emojis);
    resolve_field(&mut first.greeting, guild_emojis);
    resolve_field(&mut first.nickname, guild_emojis);
    resolve_field(&mut first.description, guild_emojis);
    resolve_field(&mut first.personality, guild_emojis);
    resolve_field(&mut second.emoji, guild_emojis);
    resolve_field(&mut second.system_prompt, guild_emojis);
    resolve_field(&mut second.prompt, guild_emojis);
    resolve_field(&mut second.scenario, guild_emojis);
}

#[cfg(test)]
mod tests {
    use super::resolve;

    /// A known unicode shortcode is replaced with the literal emoji.
    #[test]
    fn resolves_unicode_shortcode() {
        let Some(emoji) = emojis::get_by_shortcode("smile") else {
            return;
        };
        assert_eq!(resolve("hej :smile:", &[]), format!("hej {}", emoji.as_str()));
    }

    /// An unknown shortcode is left exactly as written.
    #[test]
    fn leaves_unknown_shortcodes_untouched() {
        let text = "a :totally_not_an_emoji: b";
        assert_eq!(resolve(text, &[]), text);
    }

    /// Stray colons that do not delimit a shortcode survive unchanged.
    #[test]
    fn preserves_non_shortcode_colons() {
        assert_eq!(resolve("kl 12:30:45", &[]), "kl 12:30:45");
    }

    /// Adjacent shortcodes both resolve.
    #[test]
    fn resolves_adjacent_shortcodes() {
        let Some(emoji) = emojis::get_by_shortcode("smile") else {
            return;
        };
        let rendered = emoji.as_str();
        assert_eq!(resolve(":smile::smile:", &[]), format!("{rendered}{rendered}"));
    }
}
