//! Bounded input first-connection and last-disconnection effects.
//!
//! These are host edges between processing frames, not cable transfer or module
//! processing. The caller tracks physical cable presence and emits edges only
//! when the first cable arrives or the last cable leaves. Removing one of
//! several stacked cables does not reset the prior combined input frame.

use crate::MAX_CHANNELS;
use std::fmt;

/// An external input channel count outside the supported 0..=16 range.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputLifecycleCountError {
    pub count: u8,
}

impl fmt::Display for InputLifecycleCountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "input channel count {} exceeds 16", self.count)
    }
}

impl std::error::Error for InputLifecycleCountError {}

/// An allocation-free composition of Rack 2.6.6 input connection edges.
///
/// First connection forces count one without changing storage. Last
/// disconnection clears all sixteen raw slots, including inactive slots and
/// storage with an already-zero count, then forces count zero. This differs
/// from the output bypass edge, which only clears the old active prefix.
///
/// `first.then(second)` applies `first` before `second`. Pending effects must
/// be consumed once or composed forward; an irreversible reset cannot be
/// removed by subtracting a previously published prefix. Incarnation fences
/// and the per-sample cable transfer remain the caller's responsibility.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputLifecycleTransform {
    reset_all: bool,
    forced_count: Option<u8>,
}

impl InputLifecycleTransform {
    pub const fn identity() -> Self {
        Self {
            reset_all: false,
            forced_count: None,
        }
    }

    /// First cable added; additional stacked cables do not create this edge.
    pub const fn connect() -> Self {
        Self {
            reset_all: false,
            forced_count: Some(1),
        }
    }

    /// Last cable removed; removing a stacked sibling is not this edge.
    pub const fn disconnect() -> Self {
        Self {
            reset_all: true,
            forced_count: Some(0),
        }
    }

    /// Compose `later` after this effect, retaining any earlier full reset.
    pub const fn then(self, later: Self) -> Self {
        Self {
            reset_all: self.reset_all || later.reset_all,
            forced_count: match later.forced_count {
                Some(count) => Some(count),
                None => self.forced_count,
            },
        }
    }

    /// Inspect `(reset_all, forced_count)` without permitting invalid effects.
    pub const fn effect(&self) -> (bool, Option<u8>) {
        (self.reset_all, self.forced_count)
    }

    /// Apply a full reset as exact +0.0, then any forced channel count.
    /// Invalid incoming counts return before either argument is changed, even
    /// when the effect would otherwise replace that count with zero or one.
    pub fn apply(
        &self,
        count: &mut u8,
        values: &mut [f32; MAX_CHANNELS],
    ) -> Result<(), InputLifecycleCountError> {
        if usize::from(*count) > MAX_CHANNELS {
            return Err(InputLifecycleCountError { count: *count });
        }
        if self.reset_all {
            values.fill(0.0);
        }
        if let Some(forced) = self.forced_count {
            *count = forced;
        }
        Ok(())
    }
}
