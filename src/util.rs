//! Small shared helpers used across the crate.

use miette::Report;
use strip_ansi_escapes::strip_str;

/// Renders an error as a full miette diagnostic (code, message, help, and source
/// chain), with terminal colors stripped, for inclusion in a tracing log line.
///
/// The command error handler already surfaces user-facing errors through a
/// [`miette::Report`]; this gives the bot's background tasks (chat replies,
/// interactions, autocomplete, image description) the same rich rendering in the
/// logs, where a bare `Display`/`Debug` would drop the diagnostic code, help
/// text, and source chain.
pub fn render_diagnostic<E: Into<Report>>(error: E) -> String {
    strip_str(format!("{:?}", error.into()))
}

/// Returns the previous index in a cyclic sequence of `len` elements, wrapping from the first
/// element back to the last.
///
/// `len` must be non-zero; both call sites cycle over a [`nonempty::NonEmpty`], so the length is
/// always at least one.
#[must_use]
pub const fn wrapping_previous(index: usize, len: usize) -> usize {
    index.saturating_add(len).saturating_sub(1).strict_rem(len)
}

/// Tests for the shared helpers.
#[cfg(test)]
mod tests {
    use super::wrapping_previous;

    /// Stepping back from the first element wraps around to the last, and other positions just
    /// decrement.
    #[test]
    fn wrapping_previous_cycles_backward_and_wraps() {
        assert_eq!(
            wrapping_previous(0, 3),
            2,
            "the first element wraps to the last"
        );
        assert_eq!(wrapping_previous(2, 3), 1, "a middle element decrements");
        assert_eq!(
            wrapping_previous(1, 3),
            0,
            "the second element steps to the first"
        );
        assert_eq!(wrapping_previous(0, 1), 0, "a single element stays put");
    }
}
