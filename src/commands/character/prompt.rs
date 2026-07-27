//! The bot's Discord slash command for previewing the assembled LLM prompt of a character.

use crate::{
    AppResult, Context,
    commands::{autocomplete, first_character_or_notify},
    error::SendMessageSnafu,
    models::{history::History, message::Message},
    traits::SayEphemeral as _,
    util::ellipsize,
};
use nonempty::NonEmpty;
use poise::serenity_prelude::MessageId;
use snafu::ResultExt as _;

/// Discord's plain-message content limit, less the code fences wrapped around the transcript.
const TRANSCRIPT_LIMIT: usize = 1992;

/// Visar hela prompten som AI-modellen får när gubben svarar.
#[poise::command(slash_command, rename = "prompt")]
pub async fn prompt(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
) -> AppResult {
    let Some(character) = first_character_or_notify(ctx, name).await? else {
        return Ok(());
    };
    let history = History::builder()
        .id(MessageId::new(ctx.id()))
        .character(character.id())
        .choices(NonEmpty::new(Message::new_assistant(character.greeting())))
        .build();
    let context = history.build_context(&character);
    ctx.say_ephemeral(code_block(&render_context(&context)))
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}

/// Renders the assembled context as a plain transcript, one message per block.
fn render_context(context: &[Message]) -> String {
    context
        .iter()
        .map(Message::transcript)
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// Wraps the transcript in a code block, ellipsized so the whole reply fits in
/// one Discord message.
fn code_block(transcript: &str) -> String {
    format!("```\n{}\n```", ellipsize(transcript, TRANSCRIPT_LIMIT))
}

/// Tests for the prompt-transcript rendering.
#[cfg(test)]
mod tests {
    use super::{TRANSCRIPT_LIMIT, code_block, render_context};
    use crate::models::{character::Character, history::History, message::Message};
    use nonempty::NonEmpty;
    use serenity::all::{MessageId, UserId};

    /// Builds a character with every prompt-affecting field set.
    fn full_character() -> Character {
        Character::builder()
            .id("character-id".to_owned())
            .name("Harry")
            .greeting("the greeting")
            .creator(UserId::new(1))
            .personality("the personality".to_owned())
            .prompt("the instructions".to_owned())
            .scenario("the scenario".to_owned())
            .system_prompt("the system prompt".to_owned())
            .example_messages(vec![(
                Some("example question".to_owned()),
                "example answer".to_owned(),
            )])
            .build()
    }

    /// Builds a fresh (no previous messages) history for the given character.
    fn fresh_history(character: &Character) -> History {
        History::builder()
            .id(MessageId::new(1))
            .character(character.id())
            .choices(NonEmpty::new(Message::new_assistant(character.greeting())))
            .build()
    }

    /// Whether `earlier` appears before `later` in `text`, with both present.
    fn appears_before(text: &str, earlier: &str, later: &str) -> bool {
        text.find(earlier)
            .zip(text.find(later))
            .is_some_and(|(first, second)| first < second)
    }

    /// The transcript renders messages as role-labeled blocks separated by blank lines.
    #[test]
    fn render_context_separates_messages_with_blank_lines() {
        let context = vec![Message::new_system("first"), Message::new_user("second")];
        assert_eq!(
            render_context(&context),
            "system: first\n\nuser: second",
            "each message is a role-labeled block, separated by a blank line"
        );
    }

    /// An empty context renders as an empty transcript.
    #[test]
    fn render_context_of_nothing_is_empty() {
        assert!(
            render_context(&[]).is_empty(),
            "no messages render to no transcript"
        );
    }

    /// The fresh-chat preview carries every prompt-affecting character field, in
    /// scaffolding order, with the system prompt last.
    #[test]
    fn preview_renders_the_full_scaffolding_in_order() {
        let character = full_character();
        let transcript = render_context(&fresh_history(&character).build_context(&character));
        assert!(
            appears_before(&transcript, "the personality", "the instructions"),
            "the personality precedes the instructions"
        );
        assert!(
            appears_before(&transcript, "the instructions", "the scenario"),
            "the instructions precede the scenario"
        );
        assert!(
            appears_before(&transcript, "the scenario", "example answer"),
            "the scenario precedes the example messages"
        );
        assert!(
            transcript.trim_end().ends_with("system: the system prompt"),
            "the system prompt is the last block of the preview"
        );
    }

    /// A long transcript is ellipsized inside the fences so the whole reply fits
    /// within Discord's 2000-character content limit.
    #[test]
    fn code_block_fits_discords_content_limit() {
        let long = "a".repeat(4000);
        let block = code_block(&long);
        assert!(
            block.starts_with("```\n") && block.ends_with("\n```"),
            "the transcript is wrapped in code fences"
        );
        assert!(
            block.chars().count() <= 2000,
            "the whole reply fits in one Discord message"
        );
        assert!(block.contains('…'), "the cut is marked with an ellipsis");
    }

    /// A transcript within the limit is wrapped whole, without an ellipsis.
    #[test]
    fn code_block_leaves_short_transcripts_whole() {
        assert_eq!(
            code_block("system: hi"),
            "```\nsystem: hi\n```",
            "a short transcript is fenced untouched"
        );
    }

    /// The transcript limit leaves exactly enough room for the code fences within
    /// Discord's 2000-character content limit.
    #[test]
    fn transcript_limit_accounts_for_the_fences() {
        let exact = "a".repeat(TRANSCRIPT_LIMIT);
        assert_eq!(
            code_block(&exact).chars().count(),
            2000,
            "a transcript at the limit fills the content limit exactly"
        );
    }
}
