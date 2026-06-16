//! A self-describing JSON codec for `native_model`.
//!
//! `native_model` defaults to bincode, which is not self-describing and cannot
//! round-trip serenity types whose `Serialize` and `Deserialize` implementations
//! use different serde data models (for example `ReactionType` serializes as a
//! map but deserializes as a struct). JSON round-trips all of them, matching how
//! these types were previously stored in `SurrealDB`.

use native_model::{Decode, Encode};
use serde::{Deserialize, Serialize};
use serde_json::Error;

/// A `native_model` codec backed by `serde_json`.
#[derive(Default)]
pub struct Json;

impl<T: Serialize> Encode<T> for Json {
    /// The error type returned when encoding fails.
    type Error = Error;
    /// Encodes a value to a JSON byte vector.
    fn encode(obj: &T) -> Result<Vec<u8>, Error> {
        serde_json::to_vec(obj)
    }
}

impl<T: for<'a> Deserialize<'a>> Decode<T> for Json {
    /// The error type returned when decoding fails.
    type Error = Error;
    /// Decodes a value from a JSON byte vector.
    fn decode(data: Vec<u8>) -> Result<T, Error> {
        serde_json::from_slice(&data)
    }
}
