//! The shared [`reqwest`] client every hand-rolled REST call goes through.
//!
//! One process-wide client, built with a connect and a total-request timeout so
//! no call can hang indefinitely, and sharing its connection pool across the
//! `OpenRouter` and `ElevenLabs` calls.

use core::time::Duration;
use reqwest::Client;
use std::sync::LazyLock;

/// How long to wait for a connection to be established before giving up.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a whole request (connect, headers, body) may take before giving up.
///
/// Generous, since it also covers speech synthesis of a long reply, but finite:
/// no call may hang indefinitely.
const REQUEST_TIMEOUT: Duration = Duration::from_mins(2);

/// The shared client every REST call goes through.
static HTTP: LazyLock<Client> = LazyLock::new(|| {
    Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        // building only fails if the TLS backend cannot be initialised; a
        // timeout-less client still beats no client at all
        .unwrap_or_else(|_| Client::new())
});

/// Returns the shared HTTP client.
pub fn http() -> &'static Client {
    &HTTP
}
