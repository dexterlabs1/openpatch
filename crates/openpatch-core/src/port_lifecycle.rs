//! Bounded composition of Rack 2.6.6 output connection and bypass edges.
//!
//! This records host edges between processing frames, not module processing or
//! `Output::setChannels()`. Repeated assignments that do not change connection
//! or bypass state must not create edges. Consumer tracking and incarnation
//! fences belong to the caller. No raw voltage storage is owned here.

use crate::MAX_CHANNELS;
use std::fmt;

/// The effect of a sequence of host edges for one initial channel count.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OutputLifecycleEntry {
    pub final_count: u8,
    /// Lanes to overwrite with positive zero, independent of their old values.
    pub clear_mask: u16,
}

/// An external channel count outside the supported 0..=16 range.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputLifecycleCountError {
    pub count: u8,
}

impl fmt::Display for OutputLifecycleCountError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "output channel count {} exceeds 16", self.count)
    }
}

impl std::error::Error for OutputLifecycleCountError {}

/// A fixed-size transformation for every possible initial output count.
///
/// Connection forces count one without clearing storage. Disconnection forces
/// count zero without clearing storage. An actual bypass edge clears the old
/// active lanes and forces count one when connected; disconnected outputs are
/// unchanged. These are the bounded host transitions in Rack 2.6.6 Engine.cpp
/// and Port.hpp, expressed as an original count/clear-mask composition.
///
/// `first.then(second)` applies `first` before `second`. A pending transform
/// must be consumed once, or composed forward when publications are skipped;
/// irreversible clearing cannot be removed by subtracting a prior transform.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutputLifecycleTransform {
    entries: [OutputLifecycleEntry; MAX_CHANNELS + 1],
}

impl Default for OutputLifecycleTransform {
    fn default() -> Self {
        Self::identity()
    }
}

impl OutputLifecycleTransform {
    pub const fn identity() -> Self {
        let mut entries = [OutputLifecycleEntry {
            final_count: 0,
            clear_mask: 0,
        }; MAX_CHANNELS + 1];
        let mut count = 0;
        while count <= MAX_CHANNELS {
            entries[count].final_count = count as u8;
            count += 1;
        }
        Self { entries }
    }

    /// First consumer added. Adding another consumer is not a connection edge.
    pub const fn connect() -> Self {
        Self {
            entries: [OutputLifecycleEntry {
                final_count: 1,
                clear_mask: 0,
            }; MAX_CHANNELS + 1],
        }
    }

    /// Last consumer removed. Removing another consumer is not this edge.
    pub const fn disconnect() -> Self {
        Self {
            entries: [OutputLifecycleEntry {
                final_count: 0,
                clear_mask: 0,
            }; MAX_CHANNELS + 1],
        }
    }

    /// Entering or leaving bypass; an unchanged bypass assignment is no edge.
    pub const fn bypass_edge() -> Self {
        let mut result = Self::identity();
        let mut count = 1;
        while count <= MAX_CHANNELS {
            result.entries[count] = OutputLifecycleEntry {
                final_count: 1,
                clear_mask: ((1_u32 << count) - 1) as u16,
            };
            count += 1;
        }
        result
    }

    /// Compose `later` after this transformation, retaining all prior clears.
    pub const fn then(self, later: Self) -> Self {
        let mut result = Self::identity();
        let mut count = 0;
        while count <= MAX_CHANNELS {
            let first = self.entries[count];
            let second = later.entries[first.final_count as usize];
            result.entries[count] = OutputLifecycleEntry {
                final_count: second.final_count,
                clear_mask: first.clear_mask | second.clear_mask,
            };
            count += 1;
        }
        result
    }

    pub fn entries(&self) -> &[OutputLifecycleEntry; MAX_CHANNELS + 1] {
        &self.entries
    }

    pub fn entry(
        &self,
        initial_count: u8,
    ) -> Result<OutputLifecycleEntry, OutputLifecycleCountError> {
        self.entries
            .get(usize::from(initial_count))
            .copied()
            .ok_or(OutputLifecycleCountError {
                count: initial_count,
            })
    }

    /// Clear selected private raw lanes to exact +0.0, then store the count.
    /// Invalid counts return an error before either argument is changed.
    pub fn apply(
        &self,
        count: &mut u8,
        values: &mut [f32; MAX_CHANNELS],
    ) -> Result<(), OutputLifecycleCountError> {
        let entry = self.entry(*count)?;
        for (lane, value) in values.iter_mut().enumerate() {
            if entry.clear_mask & (1_u16 << lane) != 0 {
                *value = 0.0;
            }
        }
        *count = entry.final_count;
        Ok(())
    }
}
