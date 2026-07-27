//! The bot's Discord slash commands for managing a character's example messages.
//!
//! Example messages are the few-shot pairs the model reads as samples of how the
//! character talks, the strongest per-character quality dial. A subcommand group
//! under `/gubbe` mirroring `/röst`: `skapa` opens a two-field modal to append a
//! pair, `döda` removes one by its listed number, `visa` lists them. Edited in
//! place with no new version, like `/gubbe röst`/`färg`/`modell`.

use crate::{
    AppResult, ApplicationContext, Context,
    commands::{autocomplete, first_character_or_notify, say_transient},
    error::{SendMessageSnafu, ShowModalSnafu},
    models::modals::ExampleMessageModal,
    phrases,
    shortcodes::{guild_emojis, resolve},
    traits::SayEphemeral as _,
    util::ellipsize,
};
use poise::{
    execute_modal,
    serenity_prelude::{AutocompleteChoice, CreateAutocompleteResponse, ResolvedValue},
};
use snafu::ResultExt as _;

/// Hanterar en gubbes exempelmeddelanden.
#[poise::command(
    slash_command,
    subcommands("create", "delete", "view"),
    subcommand_required,
    rename = "exempel"
)]
#[expect(clippy::unused_async, reason = "poise requires commands to be async")]
pub async fn example(_: Context<'_>) -> AppResult {
    Ok(())
}

/// Lägger till ett exempelmeddelande till en gubbe.
#[poise::command(slash_command, rename = "skapa")]
pub async fn create(
    ctx: ApplicationContext<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
) -> AppResult {
    let Some(character) = first_character_or_notify(Context::Application(ctx), name).await? else {
        return Ok(());
    };

    let Some(modal) = execute_modal(ctx, None::<ExampleMessageModal>, None)
        .await
        .context(ShowModalSnafu)?
    else {
        return Ok(());
    };

    let guild_emojis = guild_emojis(ctx).await;
    let response = resolve(&modal.response, &guild_emojis);
    if response.trim().is_empty() {
        return say_transient(
            ctx,
            "Ett exempel utan svar säger ingenting. Ge gubben något att säga.",
        )
        .await;
    }
    let user = modal
        .user
        .filter(|value| !value.trim().is_empty())
        .map(|value| resolve(&value, &guild_emojis));

    ctx.data()
        .db
        .add_character_example(character.id(), user, response)
        .await?;
    say_transient(ctx, phrases::done()).await
}

/// Tar bort ett exempelmeddelande från en gubbe.
#[poise::command(slash_command, rename = "döda")]
pub async fn delete(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
    #[rename = "nummer"]
    #[description = "Vilket exempel (se /gubbe exempel visa)"]
    #[autocomplete = autocomplete_example]
    #[min = 1]
    number: u32,
) -> AppResult {
    let Some(character) = first_character_or_notify(ctx, name).await? else {
        return Ok(());
    };

    let one_based = usize::try_from(number).unwrap_or(0);
    let count = character.example_messages().len();
    if one_based == 0 || one_based > count {
        ctx.say_ephemeral("Det finns inget exempel med det numret.")
            .await
            .context(SendMessageSnafu)?;
        return Ok(());
    }

    let index = one_based.saturating_sub(1);
    ctx.data()
        .db
        .remove_character_example(character.id(), index)
        .await?;
    ctx.say_ephemeral(phrases::done())
        .await
        .context(SendMessageSnafu)?;
    Ok(())
}

/// Visar en gubbes exempelmeddelanden.
#[poise::command(slash_command, rename = "visa")]
pub async fn view(
    ctx: Context<'_>,
    #[rename = "namn"]
    #[description = "Gubbens namn"]
    #[autocomplete = autocomplete]
    name: String,
) -> AppResult {
    let Some(character) = first_character_or_notify(ctx, name).await? else {
        return Ok(());
    };

    let examples = character.example_messages();
    let message = if examples.is_empty() {
        "Den här gubben har inga exempelmeddelanden än.".to_owned()
    } else {
        examples
            .iter()
            .enumerate()
            .map(|(index, (user, response))| {
                let number = index.saturating_add(1);
                user.as_ref().map_or_else(
                    || format!("{number}. {}", preview(response, 200)),
                    |line| {
                        format!(
                            "{number}. Användare: {} → {}",
                            preview(line, 200),
                            preview(response, 200)
                        )
                    },
                )
            })
            .collect::<Vec<String>>()
            .join("\n")
    };
    ctx.say_ephemeral(message).await.context(SendMessageSnafu)?;
    Ok(())
}

/// Autocompletes the example numbers of the character named in the sibling `namn`
/// option, labelling each with a truncated single-line preview of the pair and
/// submitting its one-based number. Returns nothing until a character is chosen.
async fn autocomplete_example<'a>(
    ctx: Context<'_>,
    _partial: &str,
) -> CreateAutocompleteResponse<'a> {
    let Context::Application(app_ctx) = ctx else {
        return CreateAutocompleteResponse::new();
    };
    let Some(name) = app_ctx.args.iter().find_map(|option| match &option.value {
        ResolvedValue::String(value) if option.name == "namn" => Some((*value).to_owned()),
        _ => None,
    }) else {
        return CreateAutocompleteResponse::new();
    };
    let Ok(characters) = ctx.data().db.characters_by_similarity(name).await else {
        return CreateAutocompleteResponse::new();
    };
    let Some(character) = characters.into_iter().next() else {
        return CreateAutocompleteResponse::new();
    };

    let choices = character
        .example_messages()
        .iter()
        .enumerate()
        .take(25)
        .map(|(index, (user, response))| {
            let number = index.saturating_add(1);
            let label = user.as_ref().map_or_else(
                || format!("{number}. {}", preview(response, 80)),
                |line| {
                    format!(
                        "{number}. {} → {}",
                        preview(line, 40),
                        preview(response, 40)
                    )
                },
            );
            AutocompleteChoice::new(label, u64::try_from(number).unwrap_or(0))
        })
        .collect::<Vec<AutocompleteChoice<'_>>>();
    CreateAutocompleteResponse::new().set_choices(choices)
}

/// Renders `text` as a single line of at most `limit` characters, ellipsised when
/// it was cut, so a long or multi-line example fits an autocomplete label or list
/// line without breaking it.
fn preview(text: &str, limit: usize) -> String {
    ellipsize(&text.replace('\n', " "), limit)
}
