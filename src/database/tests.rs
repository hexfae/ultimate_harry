//! Characterization tests pinning the load-mutate-write methods.
//! They run against a fresh temporary database directory.

use super::store::is_safe_id;
use super::{Database, DatabaseError, StoreError};
use crate::llm::CharacterModelSettings;
use crate::models::character::Character;
use serenity::all::{Color, UserId};
use std::io;

/// A storage failure is only worth retrying when the filesystem was at fault; a
/// record whose JSON does not parse fails the same way every time.
#[test]
fn retryable_follows_the_underlying_store_failure() {
    let transient = DatabaseError::Get {
        source: StoreError::Io {
            source: io::Error::other("disk hiccup"),
        },
    };
    assert!(transient.retryable(), "a filesystem failure may pass later");

    let corrupt = serde_json::from_str::<u8>("not json")
        .err()
        .map(|source| DatabaseError::Get {
            source: StoreError::Deserialize { source },
        });
    assert!(
        corrupt.is_some_and(|error| !error.retryable()),
        "a corrupt record stays corrupt"
    );
}

/// A client-supplied character ID with path-traversal components is rejected,
/// so a crafted select value cannot read a file outside the characters dir.
#[test]
fn is_safe_id_rejects_path_traversal() {
    assert!(is_safe_id("01J0ABCDEF"), "a plain ULID is a safe id");
    assert!(is_safe_id("123456789"), "a numeric snowflake is a safe id");
    assert!(!is_safe_id(""), "an empty id is rejected");
    assert!(
        !is_safe_id("../config/tts_settings"),
        "a parent traversal is rejected"
    );
    assert!(!is_safe_id("sub/dir"), "a separator is rejected");
}

/// A path-traversal character ID must not reach a record on a *write* path,
/// the way it cannot on the [`Database::character`] read path. An `id` like
/// `../characters/<real>` resolves back to a stored record, so without the
/// guard in `mutate_character` a crafted select value would mutate it; the
/// guard now short-circuits every mutation to `None`, leaving it untouched.
#[tokio::test]
async fn mutations_reject_path_traversal_ids() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    insert(&db, character("sentinel", "Harry")).await;

    let traversal = "../characters/sentinel";

    let deleted = db.delete_character(traversal, UserId::new(2)).await;
    assert!(
        matches!(deleted, Ok(None)),
        "a traversal id is rejected on the delete path, returning None instead of mutating"
    );

    let recolored = db
        .set_character_color(traversal, Color::new(0x00ff_0000))
        .await;
    assert!(
        matches!(recolored, Ok(None)),
        "a traversal id is rejected on the color write path too"
    );

    let stored = db.character("sentinel").await.ok().flatten();
    assert!(
        stored.is_some_and(|record| record.is_visible() && record.color().is_none()),
        "the real character is left untouched by the traversal attempts"
    );
}

/// Builds a minimal visible character with the given ID and name.
fn character(id: &str, name: &str) -> Character {
    Character::builder()
        .id(id.to_owned())
        .name(name)
        .greeting("hello")
        .creator(UserId::new(1))
        .build()
}

/// Inserts a character, asserting the write succeeds.
async fn insert(db: &Database, character: Character) {
    assert!(
        db.insert_character(character).await.is_ok(),
        "inserting a character should succeed"
    );
}

/// `delete_character` soft-deletes the character and returns it; the stored
/// record becomes invisible.
#[tokio::test]
async fn delete_character_soft_deletes_and_returns_the_character() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    insert(&db, character("char-id", "Harry")).await;

    let deleted = db.delete_character("char-id", UserId::new(2)).await;
    assert!(
        deleted
            .as_ref()
            .is_ok_and(|found| found.as_ref().is_some_and(|record| !record.is_visible())),
        "deleting returns the now-invisible character"
    );

    let stored = db.character("char-id").await.ok().flatten();
    assert!(
        stored.is_some_and(|record| !record.is_visible()),
        "the stored character is left invisible"
    );
}

/// Deleting a missing character returns `None` rather than erroring.
#[tokio::test]
async fn delete_character_returns_none_when_missing() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    let deleted = db.delete_character("missing", UserId::new(2)).await;
    assert!(
        matches!(deleted, Ok(None)),
        "deleting a missing character is a no-op returning None"
    );
}

/// `supersede_character` links the old record to the new version's ID.
#[tokio::test]
async fn supersede_character_sets_the_next_version() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    insert(&db, character("old-id", "Harry")).await;

    let superseded = db.supersede_character("new-id".to_owned(), "old-id").await;
    assert!(
        superseded.as_ref().is_ok_and(|found| found
            .as_ref()
            .is_some_and(|record| record.next_version() == Some("new-id"))),
        "superseding records the new version's ID on the old character"
    );
}

/// Superseding a missing character is an error (unlike delete/model-settings,
/// which return None).
#[tokio::test]
async fn supersede_character_errors_when_missing() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    let superseded = db.supersede_character("new-id".to_owned(), "missing").await;
    assert!(
        superseded.is_err(),
        "superseding a missing character errors"
    );
}

/// `set_character_model_settings` records a per-character override and returns
/// the updated character; a missing character returns `None`.
#[tokio::test]
async fn set_character_model_settings_records_an_override() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    insert(&db, character("char-id", "Harry")).await;

    let updated = db
        .set_character_model_settings(
            "char-id",
            CharacterModelSettings {
                model: Some("char-model".to_owned()),
                provider: Some("vendor/fp8".to_owned()),
                temperature: Some(0.5),
            },
        )
        .await;
    assert!(
        updated
            .as_ref()
            .is_ok_and(|found| found.as_ref().is_some_and(|record| record
                .model_settings()
                .and_then(|settings| settings.model.as_deref())
                == Some("char-model"))),
        "setting model settings records the override's contents and returns the character"
    );

    let missing = db
        .set_character_model_settings("missing", CharacterModelSettings::default())
        .await;
    assert!(
        matches!(missing, Ok(None)),
        "setting model settings on a missing character returns None"
    );
}

/// `set_character_voice` links a voice to a character and returns it; clearing
/// it removes the link, and a missing character returns `None`.
#[tokio::test]
async fn set_character_voice_links_and_clears_a_voice() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    insert(&db, character("char-id", "Harry")).await;

    let linked = db
        .set_character_voice("char-id", Some("voice-abc".to_owned()))
        .await;
    assert!(
        linked.as_ref().is_ok_and(|found| found
            .as_ref()
            .is_some_and(|record| record.voice() == Some("voice-abc"))),
        "linking a voice records it and returns the character"
    );

    let cleared = db.set_character_voice("char-id", None).await;
    assert!(
        cleared.as_ref().is_ok_and(|found| found
            .as_ref()
            .is_some_and(|record| record.voice().is_none())),
        "clearing the voice removes the link"
    );

    let missing = db
        .set_character_voice("missing", Some("voice".to_owned()))
        .await;
    assert!(
        matches!(missing, Ok(None)),
        "setting a voice on a missing character returns None"
    );
}

/// `set_character_color` records a color on a character and returns it; a missing
/// character returns `None`.
#[tokio::test]
async fn set_character_color_records_a_color() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    insert(&db, character("char-id", "Harry")).await;

    let set = db
        .set_character_color("char-id", Color::new(0x00ff_0000))
        .await;
    assert!(
        set.as_ref().is_ok_and(|found| found
            .as_ref()
            .is_some_and(|record| record.color() == Some(Color::new(0x00ff_0000)))),
        "setting a color records it and returns the character"
    );

    let missing = db
        .set_character_color("missing", Color::new(0x00ff_0000))
        .await;
    assert!(
        matches!(missing, Ok(None)),
        "setting a color on a missing character returns None"
    );
}

/// TTS settings round-trip through the config file, defaulting before any are saved.
#[tokio::test]
async fn tts_settings_round_trip_through_the_config_file() {
    use crate::tts::{TtsSettings, VoiceEntry};
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };

    assert!(
        db.tts_settings().await.api_key.is_empty(),
        "the default settings have no API key before any are saved"
    );

    let saved = db
        .upsert_tts_settings(TtsSettings {
            api_key: "secret".to_owned(),
            default_voice: Some("voice-1".to_owned()),
            model: "eleven_multilingual_v2".to_owned(),
            tag_model: Some("vendor/tagger".to_owned()),
            voices: vec![VoiceEntry {
                name: "Anna".to_owned(),
                voice_id: "voice-anna".to_owned(),
                emoji: "🎭".to_owned(),
                description: "lugn".to_owned(),
                model: None,
            }],
        })
        .await;
    assert!(saved.is_ok(), "saving the settings should succeed");

    let loaded = db.tts_settings().await;
    assert_eq!(loaded.api_key, "secret", "the saved API key is read back");
    assert_eq!(
        loaded.default_voice.as_deref(),
        Some("voice-1"),
        "the saved default voice is read back"
    );
    assert_eq!(
        loaded.model, "eleven_multilingual_v2",
        "the saved synthesis model is read back"
    );
    assert_eq!(
        loaded.tag_model.as_deref(),
        Some("vendor/tagger"),
        "the saved audio-tag model is read back"
    );
    assert_eq!(
        loaded
            .voices
            .iter()
            .map(|voice| voice.voice_id.as_str())
            .collect::<Vec<_>>(),
        vec!["voice-anna"],
        "the saved voice palette is read back"
    );
}

/// `record_character_generation` applies stats to the latest version, walking
/// past an edit in the version chain.
#[tokio::test]
async fn record_character_generation_lands_on_the_latest_version() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    let mut old = character("old-id", "Harry");
    old.set_next_version("new-id".to_owned());
    insert(&db, old).await;
    insert(&db, character("new-id", "Harry")).await;

    assert!(
        db.record_character_generation("old-id", 3, 9).await.is_ok(),
        "recording generation succeeds"
    );

    let latest = db.character("new-id").await.ok().flatten();
    assert!(
        latest
            .is_some_and(|record| record.words_generated() == 3 && record.tokens_generated() == 9),
        "generation stats land on the latest version"
    );
    let original = db.character("old-id").await.ok().flatten();
    assert!(
        original
            .is_some_and(|record| record.words_generated() == 0 && record.tokens_generated() == 0),
        "the original version receives no stats"
    );
}

/// `restore_character` clears a soft-deleted character's deleted state and
/// returns it visible again; a missing character returns `None`.
#[tokio::test]
async fn restore_character_restores_a_deleted_character() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    insert(&db, character("char-id", "Harry")).await;
    let deleted = db.delete_character("char-id", UserId::new(2)).await;
    assert!(deleted.is_ok(), "deleting the character should succeed");

    let restored = db.restore_character("char-id").await;
    assert!(
        restored
            .as_ref()
            .is_ok_and(|found| found.as_ref().is_some_and(Character::is_visible)),
        "restoring returns the now-visible character"
    );

    let stored = db.character("char-id").await.ok().flatten();
    assert!(
        stored.is_some_and(|record| record.is_visible()),
        "the stored character is visible again"
    );

    let missing = db.restore_character("missing").await;
    assert!(
        matches!(missing, Ok(None)),
        "restoring a missing character returns None"
    );
}

/// `deleted_characters_by_similarity` ranks only the soft-deleted characters,
/// never the visible ones.
#[tokio::test]
async fn deleted_characters_by_similarity_returns_only_deleted() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    insert(&db, character("visible", "Harry")).await;
    insert(&db, character("ghost", "Harry")).await;
    let deleted = db.delete_character("ghost", UserId::new(2)).await;
    assert!(deleted.is_ok(), "deleting the character should succeed");

    let found = db
        .deleted_characters_by_similarity("Harry")
        .await
        .unwrap_or_default();
    let ids = found
        .iter()
        .map(|record| record.id().to_owned())
        .collect::<Vec<String>>();
    assert_eq!(
        ids,
        vec!["ghost".to_owned()],
        "only the deleted character is returned, never the visible one"
    );
}

/// `character_versions` walks the version chain to its root and back, returning
/// every version oldest to newest regardless of which version it starts from.
#[tokio::test]
async fn character_versions_returns_the_chain_oldest_to_newest() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    let mut old = character("v0", "Harry");
    old.set_next_version("v1".to_owned());
    insert(&db, old).await;
    let new = Character::builder()
        .id("v1".to_owned())
        .name("Harry")
        .greeting("hello")
        .creator(UserId::new(1))
        .version(1_u32)
        .previous_version("v0".to_owned())
        .build();
    insert(&db, new).await;

    let from_new = db.character_versions("v1").await.unwrap_or_default();
    let ids_from_new = from_new
        .iter()
        .map(|record| record.id().to_owned())
        .collect::<Vec<String>>();
    assert_eq!(
        ids_from_new,
        vec!["v0".to_owned(), "v1".to_owned()],
        "the chain is returned oldest to newest"
    );

    let from_old = db.character_versions("v0").await.unwrap_or_default();
    let ids_from_old = from_old
        .iter()
        .map(|record| record.id().to_owned())
        .collect::<Vec<String>>();
    assert_eq!(
        ids_from_old,
        vec!["v0".to_owned(), "v1".to_owned()],
        "walking from any version returns the full chain"
    );
}

/// `rollback_character` creates a new head with the old version's content,
/// keeps the previous head's accumulated stats, and supersedes that head.
#[tokio::test]
async fn rollback_character_supersedes_the_head_with_an_old_version() {
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    let mut old = Character::builder()
        .id("v0".to_owned())
        .name("Old")
        .greeting("old greeting")
        .creator(UserId::new(1))
        .personality("old personality".to_owned())
        .build();
    old.set_next_version("v1".to_owned());
    insert(&db, old).await;
    let head = Character::builder()
        .id("v1".to_owned())
        .name("New")
        .greeting("new greeting")
        .creator(UserId::new(1))
        .version(1_u32)
        .previous_version("v0".to_owned())
        .personality("new personality".to_owned())
        .conversations_had(7_u32)
        .build();
    insert(&db, head).await;

    let rolled = db.rollback_character("v1", "v0", UserId::new(3)).await;
    assert!(
        rolled.as_ref().is_ok_and(|found| found
            .as_ref()
            .is_some_and(|record| record.name() == "Old"
                && record.is_visible()
                && record.conversations_had() == 7)),
        "rolling back returns a visible new head with the old content and kept stats"
    );

    let head_now = db.character("v1").await.ok().flatten();
    assert!(
        head_now.is_some_and(|record| record.next_version().is_some()),
        "the previous head is superseded by the rolled-back version"
    );

    let missing = db.rollback_character("missing", "v0", UserId::new(3)).await;
    assert!(
        matches!(missing, Ok(None)),
        "rolling back a missing head returns None"
    );
}

/// A history round-trips through `upsert_history`/`history`, preserving its
/// choices and context messages (content, not just IDs).
#[tokio::test]
async fn history_round_trips_through_the_chat_file() {
    use crate::models::history::History;
    use crate::models::message::Message;
    use nonempty::NonEmpty;
    use serenity::all::MessageId;

    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };

    let mut choices = NonEmpty::new(Message::new_system("first choice"));
    choices.push(Message::new_system("second choice"));
    let history = History::builder()
        .id(MessageId::new(42))
        .character("character-id")
        .choices(choices)
        .current(1_usize)
        .previous(vec![Message::new_user("hello")])
        .build();
    assert!(
        db.upsert_history(history).await.is_ok(),
        "storing a history should succeed"
    );

    let loaded = db.history(MessageId::new(42)).await.ok().flatten();
    assert!(loaded.is_some(), "the stored history reads back");
    let Some(reloaded) = loaded else { return };
    assert_eq!(
        reloaded.character(),
        "character-id",
        "the character link survives the round-trip"
    );
    assert_eq!(
        reloaded.current_choice(),
        1,
        "the chosen index survives the round-trip"
    );
    assert_eq!(
        reloaded.chosen_message().chosen_revision().head().content(),
        "second choice",
        "the chosen choice's content survives the round-trip"
    );
    assert_eq!(
        reloaded
            .previous_messages()
            .first()
            .map(|message| message.chosen_revision().head().content()),
        Some("hello"),
        "the context message's content survives the round-trip"
    );
}

/// A reading for a missing chat ID yields `None` rather than erroring.
#[tokio::test]
async fn history_returns_none_when_missing() {
    use serenity::all::MessageId;
    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    let loaded = db.history(MessageId::new(999)).await;
    assert!(
        matches!(loaded, Ok(None)),
        "a missing history reads back as None"
    );
}

/// Per-character overrides replace only the fields they supply, leaving the
/// other global settings untouched.
#[tokio::test]
async fn resolved_model_settings_applies_only_supplied_overrides() {
    use crate::llm::ModelSettings;

    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    let global = ModelSettings {
        model: "global-model".to_owned(),
        temperature: 0.5_f32,
        ..ModelSettings::default()
    };
    assert!(
        db.upsert_model_settings(global).await.is_ok(),
        "storing the global settings should succeed"
    );

    let mut model_only = character("model-only", "Harry");
    model_only.set_model_settings(CharacterModelSettings {
        model: Some("char-model".to_owned()),
        provider: None,
        temperature: None,
    });
    let model_resolved = db.resolved_model_settings(&model_only).await;
    assert_eq!(
        model_resolved.model, "char-model",
        "a model override replaces the global model"
    );
    assert_eq!(
        model_resolved.temperature.to_bits(),
        0.5_f32.to_bits(),
        "the global temperature is kept when only the model is overridden"
    );
    assert_eq!(
        model_resolved.provider, None,
        "a character on another model does not inherit the global provider pin, \
         which names an endpoint of the global model"
    );

    let mut temperature_only = character("temperature-only", "Harry");
    temperature_only.set_model_settings(CharacterModelSettings {
        model: None,
        provider: None,
        temperature: Some(0.9_f32),
    });
    let temperature_resolved = db.resolved_model_settings(&temperature_only).await;
    assert_eq!(
        temperature_resolved.model, "global-model",
        "the global model is kept when only the temperature is overridden"
    );
    assert_eq!(
        temperature_resolved.temperature.to_bits(),
        0.9_f32.to_bits(),
        "a temperature override replaces the global temperature"
    );

    let plain_resolved = db
        .resolved_model_settings(&character("plain", "Harry"))
        .await;
    assert_eq!(
        plain_resolved.model, "global-model",
        "a character with no override leaves the global model unchanged"
    );
}

/// A provider pin belongs to the model it was picked from, so it is inherited
/// only by a character running the global model, and a character's own pin
/// replaces it whatever model that character runs.
#[tokio::test]
async fn resolved_model_settings_scopes_the_provider_pin_to_its_model() {
    use crate::llm::ModelSettings;

    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    let global = ModelSettings {
        model: "global-model".to_owned(),
        provider: Some("vendor/global-fp8".to_owned()),
        ..ModelSettings::default()
    };
    assert!(
        db.upsert_model_settings(global).await.is_ok(),
        "storing the global settings should succeed"
    );

    let inherits = db
        .resolved_model_settings(&character("inherit", "Harry"))
        .await;
    assert_eq!(
        inherits.provider.as_deref(),
        Some("vendor/global-fp8"),
        "a character on the global model inherits the global pin"
    );

    let mut same_model = character("same-model", "Harry");
    same_model.set_model_settings(CharacterModelSettings {
        model: Some("global-model".to_owned()),
        provider: None,
        temperature: None,
    });
    let same_model_resolved = db.resolved_model_settings(&same_model).await;
    assert_eq!(
        same_model_resolved.provider.as_deref(),
        Some("vendor/global-fp8"),
        "a character redundantly restating the global model keeps the global pin"
    );

    let mut own_pin = character("own-pin", "Harry");
    own_pin.set_model_settings(CharacterModelSettings {
        model: Some("char-model".to_owned()),
        provider: Some("vendor/char-fp8".to_owned()),
        temperature: None,
    });
    let own_pin_resolved = db.resolved_model_settings(&own_pin).await;
    assert_eq!(
        own_pin_resolved.provider.as_deref(),
        Some("vendor/char-fp8"),
        "a character's own pin replaces the global one"
    );
}

/// A corrupt config file falls back to the default rather than erroring, so a
/// bad write cannot wedge the bot.
#[tokio::test]
async fn model_settings_falls_back_to_default_on_corruption() {
    use crate::llm::ModelSettings;
    use tokio::fs;

    let opened = Database::temporary().await;
    assert!(
        opened.is_ok(),
        "opening a temporary database should succeed"
    );
    let Ok(db) = opened else { return };
    let written = fs::write(&db.model_settings_path(), b"{ not valid json").await;
    assert!(written.is_ok(), "writing the corrupt file should succeed");

    let settings = db.model_settings().await;
    assert_eq!(
        settings.model,
        ModelSettings::default().model,
        "a corrupt model-settings file falls back to the default"
    );
}
