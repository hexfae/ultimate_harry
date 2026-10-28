//! The bot's Discord slash commands.

pub mod character;
pub mod chat;
pub mod emoji;
pub mod model;
pub mod name;
pub mod pin_channel;
pub mod say;
pub mod stats;
pub mod tts;
pub mod voice;

pub use character::character;
pub use chat::chat;
pub use emoji::emoji;
pub use model::model;
pub use name::name;
pub use pin_channel::pin_channel;
pub use say::say;
pub use stats::stats;
pub use tts::tts;
pub use voice::voice;

use poise::serenity_prelude::{
    AutocompleteChoice, CreateAutocompleteResponse, InstallationContext, ResolvedOption,
    ResolvedValue,
};
use snafu::ResultExt as _;
use tokio::time::sleep;
use tracing::warn;

use crate::{
    AppResult, ApplicationContext, Context,
    app_state::AppState,
    constants::{MAX_RESULTS, TRANSIENT_LINGER},
    database::DatabaseError,
    error::{AppError, DeleteMessageSnafu, SendMessageSnafu},
    llm::{matching_endpoints, matching_model_ids, model_catalog, model_endpoints},
    models::character::Character,
    phrases::no_character,
    shortcodes::strip_custom_emoji,
    traits::SayEphemeral as _,
    tts::{TtsManager, filter_voices},
    util::{CLEAR_KEYWORD, ellipsize, report_error},
};

/// Returns the full list of the bot's slash commands.
///
/// Declared once here so framework setup and per-guild registration cannot drift apart.
pub fn commands() -> Vec<poise::Command<AppState, AppError>> {
    vec![
        character(),
        chat(),
        emoji(),
        model(),
        pin_channel(),
        name(),
        say(),
        stats(),
        tts(),
        voice(),
    ]
}

/// A command list split for registration: the globally registered commands and
/// the guild-scoped commands.
type CommandSplit = (
    Vec<poise::Command<AppState, AppError>>,
    Vec<poise::Command<AppState, AppError>>,
);

/// Splits [`commands`] into the user-installable commands, which must be
/// registered globally so they work in servers and DMs where the bot is absent,
/// and the rest, which are registered per guild.
pub fn partitioned_commands() -> CommandSplit {
    commands().into_iter().partition(is_user_installable)
}

/// Whether a command is user-installable, i.e. its install context includes
/// [`InstallationContext::User`].
fn is_user_installable(command: &poise::Command<AppState, AppError>) -> bool {
    command
        .install_context
        .as_ref()
        .is_some_and(|contexts| contexts.contains(&InstallationContext::User))
}

/// Sends the shared "no character found" notice ephemerally, then deletes it
/// after a short linger, so every command that finds no character responds the
/// same transient way.
pub async fn notify_no_character(ctx: Context<'_>) -> AppResult {
    let message = ctx
        .say_ephemeral(no_character())
        .await
        .context(SendMessageSnafu)?;
    sleep(TRANSIENT_LINGER).await;
    message.delete(ctx).await.context(DeleteMessageSnafu)?;
    Ok(())
}

/// Returns the visible character most similar to `name`, or sends the shared
/// "no character found" notice and returns `None` when the search finds nothing.
/// The single search-then-pick-first-or-notify policy shared by the commands that
/// act on one named character.
pub async fn first_character_or_notify(
    ctx: Context<'_>,
    name: String,
) -> AppResult<Option<Character>> {
    let Some(character) = ctx
        .data()
        .db
        .characters_by_similarity(name)
        .await?
        .into_iter()
        .next()
    else {
        notify_no_character(ctx).await?;
        return Ok(None);
    };
    Ok(Some(character))
}

/// Sends `text` as an ephemeral reply, lets it linger briefly, then deletes it,
/// the transient success notice shared by the create and edit commands (the
/// [`ApplicationContext`] sibling of the paginator's `respond_then_clear`).
pub async fn say_transient<T: Into<String>>(ctx: ApplicationContext<'_>, text: T) -> AppResult {
    let message = ctx
        .say_ephemeral(text.into())
        .await
        .context(SendMessageSnafu)?;
    sleep(TRANSIENT_LINGER).await;
    message
        .delete(Context::Application(ctx))
        .await
        .context(DeleteMessageSnafu)?;
    Ok(())
}

/// Builds the plain-text autocomplete label for a character: its name preceded
/// by any unicode emoji, with custom server emoji dropped. Discord renders only
/// unicode emoji in autocomplete choices and would show `<:name:id>` markup as
/// raw text, so we strip it here.
fn autocomplete_label(character: &Character) -> String {
    let Some(emoji) = character.emoji() else {
        return character.name().to_owned();
    };
    let visible = strip_custom_emoji(emoji);
    let trimmed = visible.trim();
    if trimmed.is_empty() {
        character.name().to_owned()
    } else {
        format!("{trimmed} {}", character.name())
    }
}

/// Builds an autocomplete response listing the given characters by name.
///
/// Each choice submits the character's ID rather than its name, so the command
/// receiving it finds exactly the picked character (see
/// [`Character::rank_by_similarity`]) even when two share a name.
fn choices_from<'a>(characters: &[Character]) -> CreateAutocompleteResponse<'a> {
    autocomplete_response(
        characters
            .iter()
            .map(|character| (autocomplete_label(character), character.id().to_owned()))
            .collect(),
    )
}

/// Discord's limit on an autocomplete choice's label and on its value, in
/// characters; a response holding a longer one is rejected outright.
const CHOICE_LIMIT: usize = 100;

/// Fits a list of `(label, value)` choices into what Discord accepts in one
/// autocomplete response: at most [`MAX_RESULTS`] of them, each label cut to
/// [`CHOICE_LIMIT`], and any choice whose value is too long to submit dropped
/// (a cut value would no longer name what it stood for).
fn fit_choices(choices: Vec<(String, String)>) -> Vec<(String, String)> {
    choices
        .into_iter()
        .filter(|(_, value)| value.chars().count() <= CHOICE_LIMIT)
        .take(MAX_RESULTS)
        .map(|(label, value)| (ellipsize(&label, CHOICE_LIMIT), value))
        .collect()
}

/// Builds an autocomplete response from `(label, value)` pairs, fitted to
/// Discord's limits by [`fit_choices`].
fn autocomplete_response<'a>(choices: Vec<(String, String)>) -> CreateAutocompleteResponse<'a> {
    let fitted = fit_choices(choices)
        .into_iter()
        .map(|(label, value)| AutocompleteChoice::new(label, value))
        .collect::<Vec<AutocompleteChoice<'_>>>();
    CreateAutocompleteResponse::new().set_choices(fitted)
}

/// Builds an autocomplete response from the given names, using each as both the
/// displayed label and the submitted value.
pub fn autocomplete_from_names<'a>(names: Vec<String>) -> CreateAutocompleteResponse<'a> {
    autocomplete_response(names.into_iter().map(|name| (name.clone(), name)).collect())
}

/// Builds an autocomplete response from a similarity-ranked character query,
/// swallowing a ranking failure into an empty list (logged) so the picker stays
/// responsive. `what` names the queried set for the warning.
fn autocomplete_from<'a>(
    ranked: Result<Vec<Character>, DatabaseError>,
    what: &str,
) -> CreateAutocompleteResponse<'a> {
    let characters = match ranked {
        Ok(characters) => characters,
        Err(why) => {
            warn!("failed to rank {what} for autocomplete, returning none");
            report_error(why);
            Vec::new()
        }
    };
    choices_from(&characters)
}

/// Builds a model-id autocomplete response from the `OpenRouter` catalog,
/// optionally restricted to models accepting `modality` as input, swallowing a
/// catalog fetch failure into an empty list (logged) so the picker stays responsive.
async fn autocomplete_models<'a>(
    partial: &str,
    modality: Option<&str>,
) -> CreateAutocompleteResponse<'a> {
    let catalog = match model_catalog().await {
        Ok(catalog) => catalog,
        Err(why) => {
            warn!("failed to fetch the model catalog for autocomplete, returning none");
            report_error(why);
            Vec::new()
        }
    };
    autocomplete_from_names(matching_model_ids(&catalog, partial, modality))
}

/// Returns an autocomplete response of `OpenRouter` model ids matching the input.
pub async fn autocomplete_model<'a>(
    _ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    autocomplete_models(partial, None).await
}

/// Returns an autocomplete response of vision-capable (image input) `OpenRouter`
/// model ids matching the input.
pub async fn autocomplete_vision_model<'a>(
    _ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    autocomplete_models(partial, Some("image")).await
}

/// Returns an autocomplete response of audio-capable `OpenRouter` model ids
/// matching the input.
pub async fn autocomplete_audio_model<'a>(
    _ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    autocomplete_models(partial, Some("audio")).await
}

/// The options of the command invocation being autocompleted.
///
/// An autocomplete interaction carries every option of the command, not just
/// the focused one, so a picker can read what the user already chose for an
/// earlier option. A prefix invocation has no such options.
const fn invocation_options(ctx: Context<'_>) -> &[ResolvedOption<'_>] {
    match ctx {
        Context::Application(app) => app.args,
        Context::Prefix(_) => &[],
    }
}

/// The value the user already supplied for the option named `name`, if any.
///
/// Both a filled option and the focused one being typed carry their text, so a
/// picker downstream of a model option sees the model as soon as it is picked.
fn chosen_option<'a>(args: &'a [ResolvedOption<'a>], name: &str) -> Option<&'a str> {
    args.iter()
        .find(|option| option.name == name)
        .and_then(|option| match &option.value {
            ResolvedValue::String(value) | ResolvedValue::Autocomplete { value, .. } => {
                Some(*value)
            }
            _ => None,
        })
        .filter(|value| !value.is_empty())
}

/// Builds an autocomplete response of the provider slugs serving `model`, from
/// the `OpenRouter` endpoint listing.
///
/// Exempt from mutation testing: it only shapes what a live `OpenRouter` lookup
/// returns, and the shaping itself is [`fit_choices`], which is tested.
#[mutants::skip]
async fn autocomplete_providers_of<'a>(
    model: &str,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    let endpoints = model_endpoints(model).await;
    let mut choices: Vec<(String, String)> = clear_choice(partial).into_iter().collect();
    choices.extend(
        matching_endpoints(&endpoints, partial)
            .into_iter()
            .map(|endpoint| (endpoint.label(), endpoint.tag().to_owned())),
    );
    autocomplete_response(choices)
}

/// The "no pin" choice offered first in the provider autocomplete, submitting
/// the clear keyword, as long as what the user has typed could still be it.
fn clear_choice(partial: &str) -> Option<(String, String)> {
    CLEAR_KEYWORD
        .starts_with(&partial.trim().to_lowercase())
        .then(|| ("ingen (ingen låsning)".to_owned(), CLEAR_KEYWORD.to_owned()))
}

/// Returns an autocomplete response of the `OpenRouter` provider slugs serving
/// the model chosen in the same `/modell` invocation, falling back to the
/// configured model while that option is still empty.
///
/// A slug names one endpoint of one model, so the list has to follow the model
/// rather than being a fixed set.
pub async fn autocomplete_provider<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    let args = invocation_options(ctx);
    let chosen = chosen_option(args, "modell").map(str::to_owned);
    let Some(model) = chosen else {
        let settings = ctx.data().db.model_settings().await;
        return autocomplete_providers_of(&settings.model, partial).await;
    };
    autocomplete_providers_of(&model, partial).await
}

/// Returns an autocomplete response of the `OpenRouter` provider slugs serving
/// the model a `/gubbe modell` invocation will end up using: the model chosen in
/// the same invocation, else the named character's own model, else the global
/// one.
pub async fn autocomplete_character_provider<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    let model = character_provider_model(ctx).await;
    autocomplete_providers_of(&model, partial).await
}

/// The model a `/gubbe modell` invocation's provider list should follow: the
/// model chosen in the same invocation, else the named character's own model,
/// else the global one.
async fn character_provider_model(ctx: Context<'_>) -> String {
    let args = invocation_options(ctx);
    if let Some(model) = chosen_option(args, "modell") {
        return model.to_owned();
    }
    let db = &ctx.data().db;
    if let Some(name) = chosen_option(args, "namn")
        && let Ok(ranked) = db.characters_by_similarity(name.to_owned()).await
        && let Some(character) = ranked.into_iter().next()
    {
        return db.resolved_model_settings(&character).await.model;
    }
    db.model_settings().await.model
}

/// Returns an auto completion response from characters found in the database, sorted by similarity to the input.
pub async fn autocomplete<'a>(ctx: Context<'_>, partial: &str) -> CreateAutocompleteResponse<'a> {
    autocomplete_from(
        ctx.data().db.characters_by_similarity(partial).await,
        "characters",
    )
}

/// Returns an auto completion response from soft-deleted characters, sorted by
/// similarity to the input. Used by the restore command, since deleted
/// characters are hidden from the regular autocomplete.
pub async fn autocomplete_deleted<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    autocomplete_from(
        ctx.data()
            .db
            .deleted_characters_by_similarity(partial)
            .await,
        "deleted characters",
    )
}

/// Autocompletes `ElevenLabs` voices for the röst-id parameters, fetched live
/// from the account's voice list: the choice shows the voice's name and
/// submits its voice ID. A fetch failure (or a missing API key) is swallowed
/// into an empty list (logged) so the picker stays responsive and a raw ID can
/// still be pasted.
///
/// Exempt from mutation testing: it needs a live command context and the
/// `ElevenLabs` voice list, and the shaping itself is [`fit_choices`], which is
/// tested.
#[mutants::skip]
pub async fn autocomplete_elevenlabs_voice<'a>(
    ctx: Context<'_>,
    partial: &str,
) -> CreateAutocompleteResponse<'a> {
    let settings = ctx.data().db.tts_settings().await;
    let voices = match TtsManager::new(settings).list_voices().await {
        Ok(voices) => voices,
        Err(why) => {
            warn!("failed to list ElevenLabs voices for autocomplete, returning none");
            report_error(why);
            Vec::new()
        }
    };
    autocomplete_response(
        filter_voices(voices, partial)
            .into_iter()
            .map(|voice| (voice.name, voice.voice_id))
            .collect(),
    )
}

/// Tests for the registration split.
#[cfg(test)]
mod tests {
    use super::{
        CHOICE_LIMIT, autocomplete_from, autocomplete_from_names, clear_choice, fit_choices,
        is_user_installable, partitioned_commands,
    };
    use crate::app_state::AppState;
    use crate::commands::{character, model, say, tts};
    use crate::constants::MAX_RESULTS;
    use crate::error::AppError;
    use crate::models::character::Character;
    use serenity::all::UserId;

    /// Autocomplete choices are capped at Discord's count, labels are cut to
    /// its length limit, and a value too long to submit is dropped.
    #[test]
    fn fit_choices_respects_discords_limits() {
        let long = "x".repeat(CHOICE_LIMIT.saturating_add(1));
        let mut choices: Vec<(String, String)> = (0_u32..30)
            .map(|index| (format!("val {index}"), format!("val {index}")))
            .collect();
        choices.insert(0, (long.clone(), "kort".to_owned()));
        choices.insert(1, ("kort".to_owned(), long.clone()));
        let fitted = fit_choices(choices);
        assert_eq!(fitted.len(), MAX_RESULTS, "the list is capped");
        assert_eq!(
            fitted.first().map(|(label, _)| label.chars().count()),
            Some(CHOICE_LIMIT),
            "an over-long label is cut to the limit"
        );
        assert!(
            fitted.iter().all(|(_, value)| value != &long),
            "an over-long value is dropped rather than cut"
        );
        let exact = "y".repeat(CHOICE_LIMIT);
        assert_eq!(
            fit_choices(vec![(exact.clone(), exact.clone())]),
            vec![(exact.clone(), exact)],
            "a label and value exactly at the limit are kept whole"
        );
    }

    /// The autocomplete builders carry their choices into the response, with a
    /// character offered under its emoji-prefixed label.
    #[test]
    fn autocomplete_builders_carry_their_choices() {
        let names = serde_json::to_string(&autocomplete_from_names(vec!["Adam".to_owned()]))
            .unwrap_or_default();
        assert!(names.contains("Adam"), "the name is offered: {names}");
        let character = Character::builder()
            .id("id".to_owned())
            .name("Harry")
            .greeting("hej")
            .creator(UserId::new(1))
            .emoji("🤖".to_owned())
            .build();
        let characters =
            serde_json::to_string(&autocomplete_from(Ok(vec![character]), "characters"))
                .unwrap_or_default();
        assert!(
            characters.contains("🤖 Harry"),
            "the character is offered under its label: {characters}"
        );
        assert!(
            characters.contains("\"value\":\"id\""),
            "and submits its ID: {characters}"
        );
    }

    /// The provider list offers the "no pin" choice while the input could still
    /// be the clear keyword, and drops it once it cannot.
    #[test]
    fn clear_choice_follows_the_input() {
        for partial in ["", "in", "INGEN"] {
            assert_eq!(
                clear_choice(partial).map(|(_, value)| value).as_deref(),
                Some("ingen"),
                "{partial:?} could still be the keyword"
            );
        }
        assert_eq!(
            clear_choice("baidu"),
            None,
            "a provider name is not the keyword"
        );
    }

    /// Whether `command` has a parameter named `parameter` with an autocomplete callback.
    fn has_autocomplete(command: &poise::Command<AppState, AppError>, parameter: &str) -> bool {
        command
            .parameters
            .iter()
            .find(|param| param.name == parameter)
            .is_some_and(|param| param.autocomplete_callback.is_some())
    }

    /// Every `OpenRouter` model-id parameter offers model autocomplete.
    #[test]
    fn model_id_parameters_offer_autocomplete() {
        let global = model();
        assert!(
            has_autocomplete(&global, "modell"),
            "/modell's modell parameter offers model autocomplete"
        );
        assert!(
            has_autocomplete(&global, "leverantör"),
            "/modell's leverantör parameter offers provider autocomplete"
        );
        assert!(
            has_autocomplete(&global, "syn-modell"),
            "/modell's syn-modell parameter offers model autocomplete"
        );
        assert!(
            has_autocomplete(&global, "ljud-modell"),
            "/modell's ljud-modell parameter offers model autocomplete"
        );
        assert!(
            has_autocomplete(&tts(), "tagg-modell"),
            "/tal's tagg-modell parameter offers model autocomplete"
        );
        let character_model = character()
            .subcommands
            .into_iter()
            .find(|subcommand| subcommand.name == "modell");
        assert!(
            character_model
                .as_ref()
                .is_some_and(|subcommand| has_autocomplete(subcommand, "modell")),
            "/gubbe modell's modell parameter offers model autocomplete"
        );
        assert!(
            character_model
                .as_ref()
                .is_some_and(|subcommand| has_autocomplete(subcommand, "leverantör")),
            "/gubbe modell's leverantör parameter offers provider autocomplete"
        );
    }

    /// `/säg` is user-installable so it can be invoked in other servers and DMs.
    #[test]
    fn say_is_user_installable() {
        assert!(
            is_user_installable(&say()),
            "/säg must be user-installable to work outside the bot's guild"
        );
    }

    /// A guild-only command like `/model` is not user-installable.
    #[test]
    fn other_commands_are_not_user_installable() {
        assert!(
            !is_user_installable(&model()),
            "/model is guild-only and must not be user-installable"
        );
    }

    /// Only `/säg` registers globally; every other command stays guild-scoped.
    #[test]
    fn only_say_registers_globally() {
        let (global, guild_scoped) = partitioned_commands();
        assert_eq!(
            global.len(),
            1,
            "exactly one command, /säg, is registered globally"
        );
        assert_eq!(
            global.first().map(|command| command.name.as_ref()),
            Some("säg"),
            "the single global command is /säg"
        );
        assert!(
            guild_scoped
                .iter()
                .all(|command| command.name.as_ref() != "säg"),
            "/säg must not also be registered per guild"
        );
    }
}
