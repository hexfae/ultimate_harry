//! The bot's Discord slash command for previewing the assembled LLM prompt of a character.

use crate::{
    AppResult, Context,
    commands::{autocomplete, first_character_or_notify},
    error::SendMessageSnafu,
    llm::DEFAULT_PROMPT,
    models::{character::Character, history::History, message::Message},
    traits::SayEphemeral as _,
};
use nonempty::NonEmpty;
use poise::{
    CreateReply,
    serenity_prelude::{CreateAttachment, MessageId},
};
use snafu::ResultExt as _;

/// Discord's plain-message content limit, less the code fences wrapped around the transcript.
const TRANSCRIPT_LIMIT: usize = 1992;

/// The file name a transcript too long for a message is attached under.
const TRANSCRIPT_FILE: &str = "prompt.txt";

/// How a prompt preview is sent.
#[derive(Debug, PartialEq, Eq)]
enum Preview {
    /// Fenced in a code block as the message itself.
    Inline(String),
    /// Attached whole as a text file, since it does not fit in a message.
    File(String),
}

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
    match preview(render_context(&fresh_chat_context(&character, ctx.id()))) {
        Preview::Inline(block) => {
            ctx.say_ephemeral(block).await.context(SendMessageSnafu)?;
        }
        Preview::File(transcript) => {
            let reply = CreateReply::default()
                .content("Prompten är för lång för ett meddelande, så här är den som fil.")
                .attachment(CreateAttachment::bytes(
                    transcript.into_bytes(),
                    TRANSCRIPT_FILE,
                ))
                .ephemeral(true);
            ctx.send(reply).await.context(SendMessageSnafu)?;
        }
    }
    Ok(())
}

/// The full message list the model receives for the first reply of a fresh
/// chat with `character`: the conversation context, then the closing
/// instruction the request appends.
fn fresh_chat_context(character: &Character, id: u64) -> Vec<Message> {
    let history = History::builder()
        .id(MessageId::new(id))
        .character(character.id())
        .choices(NonEmpty::new(Message::new_assistant(character.greeting())))
        .build();
    let mut context = history.build_context(character);
    context.push(Message::new_system(DEFAULT_PROMPT));
    context
}

/// Renders the assembled context as a plain transcript, one message per block.
fn render_context(context: &[Message]) -> String {
    context
        .iter()
        .map(Message::transcript)
        .collect::<Vec<String>>()
        .join("\n\n")
}

/// Wraps the transcript in a code block when it fits in one Discord message,
/// and attaches it whole as a file when it does not.
fn preview(transcript: String) -> Preview {
    if transcript.chars().count() <= TRANSCRIPT_LIMIT {
        Preview::Inline(format!("```\n{transcript}\n```"))
    } else {
        Preview::File(transcript)
    }
}

/// Tests for the prompt-transcript rendering.
#[cfg(test)]
mod tests {
    use super::{Preview, TRANSCRIPT_LIMIT, fresh_chat_context, preview, render_context};
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

    /// A transcript too long for a message is attached whole instead of cut.
    #[test]
    fn a_long_transcript_is_attached_whole() {
        let long = "a".repeat(TRANSCRIPT_LIMIT.saturating_add(1));
        assert_eq!(
            preview(long.clone()),
            Preview::File(long),
            "nothing of the transcript is lost"
        );
    }

    /// A transcript within the limit is fenced as the message itself.
    #[test]
    fn a_short_transcript_is_fenced_inline() {
        assert_eq!(
            preview("system: hi".to_owned()),
            Preview::Inline("```\nsystem: hi\n```".to_owned()),
            "a short transcript is fenced untouched"
        );
    }

    /// The transcript limit leaves exactly enough room for the code fences within
    /// Discord's 2000-character content limit.
    #[test]
    fn transcript_limit_accounts_for_the_fences() {
        let exact = "a".repeat(TRANSCRIPT_LIMIT);
        let inline_length = match preview(exact) {
            Preview::Inline(block) => Some(block.chars().count()),
            Preview::File(_) => None,
        };
        assert_eq!(
            inline_length,
            Some(2000),
            "a transcript at the limit is sent inline, filling the content limit exactly"
        );
    }

    /// The preview ends with the closing instruction the request appends, so it
    /// shows everything the model receives.
    #[test]
    fn the_preview_ends_with_the_closing_instruction() {
        let context = fresh_chat_context(&full_character(), 1);
        assert_eq!(
            context.last().map(Message::transcript).as_deref(),
            Some("system: Fortsätt rollspelet."),
            "the closing instruction comes last"
        );
    }
}
