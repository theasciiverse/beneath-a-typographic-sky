//! The CD intro. Not ported yet: it counts as skipped.

use super::sky::Sky;

impl Sky {
    /// Returns true when the intro played to the end, false when skipped.
    pub(crate) fn do_intro(&mut self) -> bool {
        false
    }
}
