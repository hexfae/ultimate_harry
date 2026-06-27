//! Resolution of `:shortcode:` runs in user-submitted character text.
//!
//! Discord only expands shortcodes (custom `:cholol:` or unicode `:smile:`) in
//! the message composer as you type, never server-side. Character text arrives
//! through modals, which have no composer, so unresolved shortcodes would render
//! literally. We expand them ourselves when a modal is submitted: custom guild
//! emoji become their `<:name:id>` wire form and unicode shortcodes become the
//! literal emoji. Unrecognised runs are left untouched.

use crate::ApplicationContext;
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
pub fn resolve(text: &str, guild_emojis: &[Emoji]) -> String {
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

/// Returns whether `token` is a single custom-emoji markup token
/// (`<:name:id>` or `<a:name:id>`).
fn is_custom_emoji_token(token: &str) -> bool {
    (token.starts_with("<:") || token.starts_with("<a:")) && token.ends_with('>')
}

/// Removes custom-emoji markup (`<:name:id>` / `<a:name:id>`) from `text`,
/// leaving unicode emoji and ordinary text intact. Used for autocomplete labels,
/// where Discord renders only unicode emoji and shows custom-emoji markup as raw
/// text. Custom emoji never contain whitespace, so a token-wise filter is enough.
pub fn strip_custom_emoji(text: &str) -> String {
    text.split_whitespace()
        .filter(|token| !is_custom_emoji_token(token))
        .collect::<Vec<_>>()
        .join(" ")
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

#[cfg(test)]
mod tests {
    use super::{resolve, strip_custom_emoji};

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

    /// Custom emoji markup is dropped while unicode emoji and text survive.
    #[test]
    fn strips_custom_emoji_for_labels() {
        assert_eq!(strip_custom_emoji("<:cholol:123>"), "");
        assert_eq!(strip_custom_emoji("<a:wave:9> Bob"), "Bob");
        assert_eq!(strip_custom_emoji("🤩 Bob"), "🤩 Bob");
        assert_eq!(strip_custom_emoji("plain"), "plain");
    }
}
