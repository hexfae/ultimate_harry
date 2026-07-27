//! Small shared helpers used across the crate.

use miette::Report;

/// Prints the full miette diagnostic (code, message, help, and source chain) to
/// stderr, where miette renders it graphically and in colour on an interactive
/// terminal, and plainly when stderr is not a terminal (so journald/file logs
/// stay free of escape codes).
///
/// Pair it with a concise `tracing` event that carries the structured context
/// fields: the rich report is kept off tracing's formatter, which mangles the
/// colours and graphical layout (a bare `Display`/`Debug` in a log line would
/// also drop the diagnostic code, help text, and source chain).
#[expect(
    clippy::print_stderr,
    reason = "miette only renders its coloured graphical report when written straight to stderr, not through tracing"
)]
pub fn report_error<E: Into<Report>>(error: E) {
    // build the graphical report with `format!` (where Debug formatting is
    // allowed) and print its Display, so miette's colours survive to stderr.
    let report = format!("{:?}", error.into());
    eprintln!("{report}");
}

/// Returns the previous index in a cyclic sequence of `len` elements, wrapping from the first
/// element back to the last.
///
/// An empty sequence has no indices to step through, so a `len` of zero yields zero.
#[must_use]
pub const fn wrapping_previous(index: usize, len: usize) -> usize {
    match index.saturating_add(len).saturating_sub(1).checked_rem(len) {
        Some(previous) => previous,
        None => 0,
    }
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
        assert_eq!(
            wrapping_previous(0, 0),
            0,
            "an empty sequence has no index to step to"
        );
    }
}
