//! Read-only extrema derived from completed Scope bins, without allocation.
//! Min/max envelopes do not contain enough information to derive RMS or frequency.
use crate::{ScopePoint, ScopeSnapshot, SCOPE_BINS};

/// Raw-voltage extrema across valid lane-bin observations.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScopeExtrema {
    pub min: f64,
    pub max: f64,
    /// Computed in f64 so opposite finite f32 extremes do not overflow.
    pub peak_to_peak: f64,
    /// Number of finite, ordered lane-bin points, not raw samples.
    pub observations: usize,
}

/// Independent X/Y measurements; an unobserved axis is `None`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScopeMeasurements {
    pub x: Option<ScopeExtrema>,
    pub y: Option<ScopeExtrema>,
}

impl ScopeSnapshot {
    /// Measure all currently active lanes using raw completed-bin volts.
    ///
    /// The bounded scan visits at most 8192 points and allocates nothing. Empty,
    /// reversed and nonfinite points are ignored. Retired lanes are excluded,
    /// even when they retain old bins. Previously completed history remains
    /// eligible while a new capture progressively replaces it; private partial
    /// accumulation is absent from the snapshot. Display gain/offset do not
    /// affect these measurements.
    pub fn measurements(&self) -> ScopeMeasurements {
        ScopeMeasurements {
            x: extrema(&self.x[..usize::from(self.x_channels).min(16)]),
            y: extrema(&self.y[..usize::from(self.y_channels).min(16)]),
        }
    }

    /// Measure a zero-based voice independently on each active axis.
    /// An inactive or out-of-range voice returns `None` for that axis.
    pub fn voice_measurements(&self, voice: usize) -> ScopeMeasurements {
        let selected = |axis: &[[ScopePoint; SCOPE_BINS]; 16], count: u8| {
            if voice < usize::from(count).min(16) {
                extrema(&axis[voice..voice + 1])
            } else {
                None
            }
        };
        ScopeMeasurements {
            x: selected(&self.x, self.x_channels),
            y: selected(&self.y, self.y_channels),
        }
    }
}

fn extrema(lanes: &[[ScopePoint; SCOPE_BINS]]) -> Option<ScopeExtrema> {
    let mut result: Option<ScopeExtrema> = None;
    for point in lanes.iter().flatten() {
        if !point.min.is_finite() || !point.max.is_finite() || point.min > point.max {
            continue;
        }
        let min = f64::from(point.min);
        let max = f64::from(point.max);
        match &mut result {
            Some(result) => {
                result.min = result.min.min(min);
                result.max = result.max.max(max);
                result.observations += 1;
            }
            None => {
                result = Some(ScopeExtrema {
                    min,
                    max,
                    peak_to_peak: 0.0,
                    observations: 1,
                });
            }
        }
    }
    if let Some(result) = &mut result {
        result.peak_to_peak = result.max - result.min;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn point(min: f32, max: f32) -> ScopePoint {
        ScopePoint { min, max }
    }
    #[test]
    fn dc_positive_envelopes_and_independent_voices() {
        let mut s = ScopeSnapshot {
            x_channels: 2,
            y_channels: 1,
            ..Default::default()
        };
        s.x[0][0] = point(3.0, 3.0);
        s.x[0][1] = point(3.0, 3.0);
        s.x[1][0] = point(7.0, 9.0);
        s.y[0][0] = point(-2.0, -2.0);
        assert_eq!(
            s.measurements().x,
            Some(ScopeExtrema {
                min: 3.0,
                max: 9.0,
                peak_to_peak: 6.0,
                observations: 3
            })
        );
        assert_eq!(
            s.voice_measurements(0).x,
            Some(ScopeExtrema {
                min: 3.0,
                max: 3.0,
                peak_to_peak: 0.0,
                observations: 2
            })
        );
        assert_eq!(s.voice_measurements(1).x.unwrap().min, 7.0);
        assert_eq!(s.voice_measurements(1).y, None);
        assert_eq!(s.measurements().y.unwrap().peak_to_peak, 0.0);
        s.x_scale = 8;
        s.x_offset = 10.0;
        assert_eq!(s.measurements().x.unwrap().min, 3.0);
    }
    #[test]
    fn retired_and_zero_channel_axes_do_not_expose_stale_bins() {
        let mut s = ScopeSnapshot {
            x_channels: 1,
            ..Default::default()
        };
        s.x[0][0] = point(2.0, 4.0);
        s.x[15][0] = point(-100.0, 100.0);
        assert_eq!(s.measurements().x.unwrap().observations, 1);
        assert_eq!(s.voice_measurements(15), ScopeMeasurements::default());
        for voice in [16, usize::MAX] {
            assert_eq!(s.voice_measurements(voice), ScopeMeasurements::default());
        }
        s.x_channels = 0;
        assert_eq!(s.measurements(), ScopeMeasurements::default());
    }
    #[test]
    fn invalid_nonfinite_and_reversed_points_are_not_observations() {
        let mut s = ScopeSnapshot {
            x_channels: 1,
            ..Default::default()
        };
        for (i, p) in [
            point(f32::NAN, 1.0),
            point(0.0, f32::NAN),
            point(f32::NEG_INFINITY, 1.0),
            point(0.0, f32::INFINITY),
            point(5.0, 4.0),
        ]
        .into_iter()
        .enumerate()
        {
            s.x[0][i] = p;
        }
        assert_eq!(s.measurements(), ScopeMeasurements::default());
        s.x[0][6] = point(5.0, 6.0);
        assert_eq!(s.measurements().x.unwrap().observations, 1);
    }
    #[test]
    fn partial_capture_counts_only_existing_completed_points_and_retained_history() {
        let mut s = ScopeSnapshot {
            x_channels: 1,
            buffer_index: 1,
            sample_index: 3,
            ..Default::default()
        };
        s.x[0][0] = point(4.0, 8.0);
        assert_eq!(s.measurements().x.unwrap().observations, 1);
        // A prior capture's complete bin remains visible until replaced.
        s.x[0][200] = point(-5.0, -3.0);
        let m = s.measurements().x.unwrap();
        assert_eq!(m.observations, 2);
        assert_eq!(m.min, -5.0);
    }
    #[test]
    fn full_f32_range_subtraction_is_finite_in_f64() {
        let mut s = ScopeSnapshot {
            x_channels: 1,
            ..Default::default()
        };
        s.x[0][0] = point(-f32::MAX, f32::MAX);
        let m = s.measurements().x.unwrap();
        assert!(m.peak_to_peak.is_finite());
        assert_eq!(m.peak_to_peak, 2.0 * f64::from(f32::MAX));
    }
}
