//! The low-level JSON-file store underlying the [`Database`](super::Database):
//! the character-agnostic read/scan/write helpers and their error type.

use serde::Serialize;
use serde::de::DeserializeOwned;
use snafu::{ResultExt as _, Snafu};
use std::ffi::OsStr;
use std::fs::Permissions;
use std::io::{self, ErrorKind};
use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt as _;
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

/// The owner-only file mode every record is written with, so the config files holding the
/// `OpenRouter` and `ElevenLabs` API keys are not readable by other users on the host.
const RECORD_MODE: u32 = 0o600;

/// Serializes `value` to pretty JSON and writes it to `path` atomically (write to a temp file,
/// then rename over the target), so a crash mid-write never leaves a partial file. `write_lock`
/// serializes concurrent writes so two saves cannot interleave their temp-file renames. The temp
/// file is created with [`RECORD_MODE`] and the rename carries that mode onto the target.
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
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(RECORD_MODE)
        .open(&temp)
        .await
        .context(IoSnafu)?;
    // a temp file left behind by a crashed write keeps its old mode, since `mode` only applies to
    // a freshly created file
    fs::set_permissions(&temp, Permissions::from_mode(RECORD_MODE))
        .await
        .context(IoSnafu)?;
    file.write_all(&bytes).await.context(IoSnafu)?;
    file.flush().await.context(IoSnafu)?;
    drop(file);
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

impl StoreError {
    /// Whether retrying might succeed (a transient filesystem failure) rather than a
    /// permanent one (a record whose JSON does not match its type, in either direction).
    #[must_use]
    pub const fn retryable(&self) -> bool {
        matches!(self, Self::Io { .. })
    }
}

#[cfg(test)]
mod tests {
    use super::{RECORD_MODE, write_json};
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};
    use std::{env, process};
    use tokio::fs;
    use tokio::sync::Mutex;

    /// A unique path under the temp dir for a test record, named after `label`.
    fn temp_record(label: &str) -> PathBuf {
        let pid = process::id();
        env::temp_dir().join(format!("harry-store-{pid}-{label}.json"))
    }

    /// The permission bits of `path`, or `None` if it cannot be inspected.
    async fn mode_of(path: &Path) -> Option<u32> {
        let metadata = fs::metadata(path).await.ok()?;
        Some(metadata.permissions().mode() & 0o777)
    }

    /// A written record is owner-only, so the config files holding the API keys are
    /// not readable by other users on the host.
    #[tokio::test]
    async fn write_json_writes_owner_only_records() {
        let path = temp_record("fresh");
        let written = write_json(&Mutex::new(()), &path, &"secret-key").await;
        assert!(written.is_ok(), "writing a record should succeed");

        assert_eq!(
            mode_of(&path).await,
            Some(RECORD_MODE),
            "a record is readable only by its owner"
        );
        drop(fs::remove_file(&path).await);
    }

    /// A temp file left behind by a crashed write is not created afresh, so its mode is
    /// reset explicitly rather than carried onto the renamed record.
    #[tokio::test]
    async fn write_json_tightens_a_leftover_temp_file() {
        let path = temp_record("stale");
        let temp = path.with_extension("json.tmp");
        assert!(
            fs::write(&temp, b"leftover").await.is_ok(),
            "seeding a leftover temp file should succeed"
        );
        assert!(
            fs::set_permissions(&temp, Permissions::from_mode(0o644))
                .await
                .is_ok(),
            "loosening the leftover temp file should succeed"
        );

        let written = write_json(&Mutex::new(()), &path, &"secret-key").await;
        assert!(written.is_ok(), "writing a record should succeed");

        assert_eq!(
            mode_of(&path).await,
            Some(RECORD_MODE),
            "the loose mode of the leftover temp file does not survive the write"
        );
        drop(fs::remove_file(&path).await);
    }
}
