//! The low-level JSON-file store underlying the [`Database`](super::Database):
//! the character-agnostic read/scan/write helpers and their error type.

use serde::Serialize;
use serde::de::DeserializeOwned;
use snafu::{ResultExt as _, Snafu};
use std::ffi::OsStr;
use std::io::{self, ErrorKind};
use std::path::Path;
use tokio::fs;
use tokio::sync::Mutex;
use tracing::error;

/// Reads and deserializes a JSON record from `path`, returning `None` if the file does not exist.
pub(super) async fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, StoreError> {
    let bytes = match fs::read(path).await {
        Ok(bytes) => bytes,
        Err(why) if why.kind() == ErrorKind::NotFound => return Ok(None),
        Err(why) => return Err(StoreError::Io { source: why }),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .context(DeserializeSnafu)
}

/// Reads and deserializes every `*.json` file in `dir`, returning an empty vector if the directory
/// does not exist. Skips the transient `*.json.tmp` files written during an atomic save.
pub(super) async fn scan_dir<T: DeserializeOwned>(dir: &Path) -> Result<Vec<T>, StoreError> {
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(why) if why.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(why) => return Err(StoreError::Io { source: why }),
    };
    let mut records = Vec::new();
    while let Some(entry) = entries.next_entry().await.context(IoSnafu)? {
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) == Some("json")
            && let Some(record) = read_json(&path).await?
        {
            records.push(record);
        }
    }
    Ok(records)
}

/// Reads a JSON record from `path`, falling back to `default` rather than blocking the bot.
///
/// A legitimately absent file defaults silently; a genuine read or parse *failure* (corruption) is
/// logged at error level under `label`, since it would otherwise surface downstream as a confusing
/// unrelated error (for example an empty API key).
pub(super) async fn read_or<T: DeserializeOwned>(
    path: &Path,
    label: &str,
    default: impl FnOnce() -> T,
) -> T {
    match read_json(path).await {
        Ok(Some(value)) => value,
        Ok(None) => default(),
        Err(why) => {
            error!("failed to read {label}, using default: {why}");
            default()
        }
    }
}

/// Whether `id` is a safe single path segment: non-empty and free of path
/// separators or parent-directory components, so it cannot escape its directory
/// when interpolated into a record's file path. Record IDs are server-minted
/// ULIDs or numeric Discord snowflakes, but a component interaction can submit
/// an arbitrary select value, so a client-supplied ID is validated before it
/// reaches the filesystem.
pub(super) fn is_safe_id(id: &str) -> bool {
    !id.is_empty() && !id.contains(['/', '\\']) && !id.contains("..")
}

/// Serializes `value` to pretty JSON and writes it to `path` atomically (write to a temp file,
/// then rename over the target), so a crash mid-write never leaves a partial file. `write_lock`
/// serializes concurrent writes so two saves cannot interleave their temp-file renames.
pub(super) async fn write_json<T: Serialize + Sync>(
    write_lock: &Mutex<()>,
    path: &Path,
    value: &T,
) -> Result<(), StoreError> {
    let bytes = serde_json::to_vec_pretty(value).context(SerializeSnafu)?;
    let _guard = write_lock.lock().await;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.context(IoSnafu)?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, &bytes).await.context(IoSnafu)?;
    fs::rename(&temp, path).await.context(IoSnafu)?;
    Ok(())
}

/// A low-level storage failure underlying a [`DatabaseError`](super::DatabaseError): a filesystem
/// error or a JSON (de)serialization error.
#[derive(Debug, Snafu)]
pub enum StoreError {
    /// A filesystem operation failed.
    #[snafu(display("filfel: {source}"))]
    Io {
        /// The source of the error.
        source: io::Error,
    },
    /// Serializing a record to JSON failed.
    #[snafu(display("kunde inte serialisera posten: {source}"))]
    Serialize {
        /// The source of the error.
        source: serde_json::Error,
    },
    /// Deserializing a record from JSON failed.
    #[snafu(display("kunde inte tolka posten: {source}"))]
    Deserialize {
        /// The source of the error.
        source: serde_json::Error,
    },
}
