//! Fixed-storage callback timing observations. The recorder has one writer;
//! readers take at most four attempts at a coherent atomic publication.
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::SeqCst},
    Arc,
};

/// Measurement fields describe the last coherent publication only when
/// `available` is true. A closed stream retains its last observations, but marks
/// them unavailable. Contention never substitutes a plausible zero measurement.
#[derive(Serialize, Clone, Debug, Default)]
pub struct Snapshot {
    pub available: bool,
    pub coherent: bool,
    pub closed: bool,
    pub requested_epoch: u64,
    pub epoch: u64,
    pub reset_pending: bool,
    pub callback_sequence: u64,
    pub sample_rate: u32,
    pub frames: usize,
    pub channels: usize,
    pub revision: u64,
    pub duration_ns: u64,
    pub utilization: Option<f64>,
    pub callbacks: u64,
    pub invalid_callbacks: u64,
    pub overruns: u64,
    pub peak_utilization: Option<f64>,
    pub max_duration_ns: u64,
    pub total_duration_ns: u64,
    pub total_frames: u64,
    pub adopted_graph: bool,
    pub crossfade: bool,
    pub transition_callbacks: u64,
}

#[derive(Default)]
struct Published {
    version: AtomicU64,
    observed: AtomicBool,
    epoch: AtomicU64,
    callback_sequence: AtomicU64,
    frames: AtomicUsize,
    channels: AtomicUsize,
    revision: AtomicU64,
    duration_ns: AtomicU64,
    callbacks: AtomicU64,
    invalid_callbacks: AtomicU64,
    overruns: AtomicU64,
    peak_utilization_bits: AtomicU64,
    max_duration_ns: AtomicU64,
    total_duration_ns: AtomicU64,
    total_frames: AtomicU64,
    adopted_graph: AtomicBool,
    crossfade: AtomicBool,
    transition_callbacks: AtomicU64,
}

pub(super) struct Diagnostics {
    rate: u32,
    requested_epoch: AtomicU64,
    closed: AtomicBool,
    published: Published,
}

#[derive(Default)]
struct Interval {
    epoch: u64,
    callbacks: u64,
    invalid_callbacks: u64,
    overruns: u64,
    peak_utilization: Option<f64>,
    max_duration_ns: u64,
    total_duration_ns: u64,
    total_frames: u64,
    transition_callbacks: u64,
}

/// Move-only ownership enforces a single callback publisher. All interval
/// bookkeeping stays local; publication uses fixed atomics without locks.
pub(super) struct Recorder {
    diagnostics: Arc<Diagnostics>,
    callback_sequence: u64,
    interval: Interval,
}

fn utilization(rate: u32, elapsed_ns: u64, frames: usize, channels: usize) -> Option<f64> {
    if rate == 0 || frames == 0 || channels == 0 {
        None
    } else {
        Some(elapsed_ns as f64 * f64::from(rate) / (frames as f64 * 1_000_000_000.0))
    }
}

impl Diagnostics {
    pub(super) fn new(rate: u32) -> (Arc<Self>, Recorder) {
        let diagnostics = Arc::new(Self {
            rate,
            requested_epoch: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            published: Published::default(),
        });
        let recorder = Recorder {
            diagnostics: Arc::clone(&diagnostics),
            callback_sequence: 0,
            interval: Interval::default(),
        };
        (diagnostics, recorder)
    }

    /// Request a new interval. The callback acknowledges the epoch captured by
    /// begin(), so a reset requested during a callback remains visibly pending.
    pub(super) fn request_reset(&self) -> u64 {
        if self.closed.load(SeqCst) {
            return self.requested_epoch.load(SeqCst);
        }
        self.requested_epoch
            .try_update(SeqCst, SeqCst, |epoch| Some(epoch.saturating_add(1)))
            .unwrap_or_else(|epoch| epoch)
            .saturating_add(1)
    }

    pub(super) fn close(&self) {
        self.closed.store(true, SeqCst);
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        let p = &self.published;
        for _ in 0..4 {
            let version = p.version.load(SeqCst);
            if version & 1 != 0 {
                continue;
            }
            let mut snapshot = Snapshot {
                coherent: true,
                epoch: p.epoch.load(SeqCst),
                callback_sequence: p.callback_sequence.load(SeqCst),
                sample_rate: self.rate,
                frames: p.frames.load(SeqCst),
                channels: p.channels.load(SeqCst),
                revision: p.revision.load(SeqCst),
                duration_ns: p.duration_ns.load(SeqCst),
                callbacks: p.callbacks.load(SeqCst),
                invalid_callbacks: p.invalid_callbacks.load(SeqCst),
                overruns: p.overruns.load(SeqCst),
                max_duration_ns: p.max_duration_ns.load(SeqCst),
                total_duration_ns: p.total_duration_ns.load(SeqCst),
                total_frames: p.total_frames.load(SeqCst),
                adopted_graph: p.adopted_graph.load(SeqCst),
                crossfade: p.crossfade.load(SeqCst),
                transition_callbacks: p.transition_callbacks.load(SeqCst),
                ..Snapshot::default()
            };
            let observed = p.observed.load(SeqCst);
            let peak = f64::from_bits(p.peak_utilization_bits.load(SeqCst));
            if p.version.load(SeqCst) != version {
                continue;
            }
            snapshot.closed = self.closed.load(SeqCst);
            // This follows the published epoch load: reset requests cannot be
            // older than an epoch already acknowledged by the sole writer.
            snapshot.requested_epoch = self.requested_epoch.load(SeqCst);
            snapshot.reset_pending = snapshot.epoch != snapshot.requested_epoch;
            snapshot.available = observed && !snapshot.closed;
            snapshot.utilization = utilization(
                self.rate,
                snapshot.duration_ns,
                snapshot.frames,
                snapshot.channels,
            );
            snapshot.peak_utilization = (snapshot.callbacks > 0).then_some(peak);
            return snapshot;
        }
        // Only independently atomic lifecycle metadata is meaningful here.
        // No unverified timing/counter combination escapes a failed read.
        let epoch = p.epoch.load(SeqCst);
        let requested_epoch = self.requested_epoch.load(SeqCst);
        Snapshot {
            sample_rate: self.rate,
            closed: self.closed.load(SeqCst),
            requested_epoch,
            epoch,
            reset_pending: epoch != requested_epoch,
            ..Snapshot::default()
        }
    }
}

impl Recorder {
    pub(super) fn begin(&self) -> u64 {
        self.diagnostics.requested_epoch.load(SeqCst)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn record(
        &mut self,
        epoch: u64,
        elapsed_ns: u64,
        frames: usize,
        channels: usize,
        revision: u64,
        adopted_graph: bool,
        crossfade: bool,
    ) {
        if self.diagnostics.closed.load(SeqCst) {
            return;
        }
        if epoch != self.interval.epoch {
            self.interval = Interval {
                epoch,
                ..Interval::default()
            };
        }
        self.callback_sequence = self.callback_sequence.saturating_add(1);
        let interval = &mut self.interval;
        if let Some(value) = utilization(self.diagnostics.rate, elapsed_ns, frames, channels) {
            interval.callbacks = interval.callbacks.saturating_add(1);
            // Integer cross multiplication makes equality an exact non-overrun,
            // even for durations/rates whose products overflow u64.
            if u128::from(elapsed_ns) * u128::from(self.diagnostics.rate)
                > frames as u128 * 1_000_000_000
            {
                interval.overruns = interval.overruns.saturating_add(1);
            }
            interval.peak_utilization = Some(interval.peak_utilization.unwrap_or(0.0).max(value));
            interval.max_duration_ns = interval.max_duration_ns.max(elapsed_ns);
            interval.total_duration_ns = interval.total_duration_ns.saturating_add(elapsed_ns);
            interval.total_frames = interval
                .total_frames
                .saturating_add(u64::try_from(frames).unwrap_or(u64::MAX));
            if adopted_graph || crossfade {
                interval.transition_callbacks = interval.transition_callbacks.saturating_add(1);
            }
        } else {
            interval.invalid_callbacks = interval.invalid_callbacks.saturating_add(1);
        }
        let p = &self.diagnostics.published;
        // SeqCst ordering gives both begin/end markers and every field one
        // shared ordering. The private seqlock marker wraps; public counters
        // saturate. No other writer can interleave publications.
        p.version.fetch_add(1, SeqCst);
        p.epoch.store(interval.epoch, SeqCst);
        p.callback_sequence.store(self.callback_sequence, SeqCst);
        p.frames.store(frames, SeqCst);
        p.channels.store(channels, SeqCst);
        p.revision.store(revision, SeqCst);
        p.duration_ns.store(elapsed_ns, SeqCst);
        p.callbacks.store(interval.callbacks, SeqCst);
        p.invalid_callbacks
            .store(interval.invalid_callbacks, SeqCst);
        p.overruns.store(interval.overruns, SeqCst);
        p.peak_utilization_bits
            .store(interval.peak_utilization.unwrap_or(0.0).to_bits(), SeqCst);
        p.max_duration_ns.store(interval.max_duration_ns, SeqCst);
        p.total_duration_ns
            .store(interval.total_duration_ns, SeqCst);
        p.total_frames.store(interval.total_frames, SeqCst);
        p.adopted_graph.store(adopted_graph, SeqCst);
        p.crossfade.store(crossfade, SeqCst);
        p.transition_callbacks
            .store(interval.transition_callbacks, SeqCst);
        p.observed.store(true, SeqCst);
        p.version.fetch_add(1, SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::thread;

    fn record(recorder: &mut Recorder, elapsed: u64, frames: usize) {
        let epoch = recorder.begin();
        recorder.record(epoch, elapsed, frames, 2, 7, false, false);
    }

    #[test]
    fn pre_first_contention_and_closed_streams_never_claim_an_available_measurement() {
        let (diagnostics, mut recorder) = Diagnostics::new(48_000);
        let first = diagnostics.snapshot();
        assert!(first.coherent);
        assert!(!first.available);
        assert!(!first.closed);
        assert_eq!(first.sample_rate, 48_000);
        assert_eq!(first.utilization, None);
        assert_eq!(first.peak_utilization, None);
        record(&mut recorder, 100, 48);
        diagnostics.published.version.store(3, SeqCst);
        let contended = diagnostics.snapshot();
        assert!(!contended.available);
        assert!(!contended.coherent);
        assert_eq!(contended.utilization, None);
        assert_eq!(contended.peak_utilization, None);
        diagnostics.published.version.store(4, SeqCst);
        assert!(diagnostics.snapshot().available);
        diagnostics.close();
        record(&mut recorder, 9000, 48);
        let closed = diagnostics.snapshot();
        assert!(closed.coherent);
        assert!(closed.closed);
        assert!(!closed.available);
        assert_eq!(closed.duration_ns, 100);
        assert_eq!(closed.callbacks, 1);
        assert_eq!(closed.callback_sequence, 1);
        assert_eq!(diagnostics.request_reset(), 0);
        let (empty, _) = Diagnostics::new(48_000);
        empty.close();
        assert!(!empty.snapshot().available);
        assert!(empty.snapshot().closed);
    }

    #[test]
    fn exact_budget_boundary_and_wide_integer_products_classify_overruns() {
        let (diagnostics, mut recorder) = Diagnostics::new(48_000);
        record(&mut recorder, 1_000_000, 48);
        let exact = diagnostics.snapshot();
        assert_eq!(exact.utilization, Some(1.0));
        assert_eq!(exact.overruns, 0);
        record(&mut recorder, 1_000_001, 48);
        assert_eq!(diagnostics.snapshot().overruns, 1);
        record(&mut recorder, 999_999, 48);
        let final_snapshot = diagnostics.snapshot();
        assert_eq!(final_snapshot.callbacks, 3);
        assert_eq!(final_snapshot.overruns, 1);
        assert_eq!(final_snapshot.max_duration_ns, 1_000_001);
        assert_eq!(final_snapshot.total_duration_ns, 3_000_000);
        assert_eq!(final_snapshot.total_frames, 144);
        assert_eq!(final_snapshot.peak_utilization, Some(1.000_001));

        let (wide, mut writer) = Diagnostics::new(u32::MAX);
        record(&mut writer, u64::MAX, usize::MAX);
        let snapshot = wide.snapshot();
        let expected =
            u128::from(u64::MAX) * u128::from(u32::MAX) > usize::MAX as u128 * 1_000_000_000;
        assert_eq!(snapshot.overruns, u64::from(expected));
        assert!(snapshot.utilization.unwrap().is_finite());
        assert_eq!(snapshot.max_duration_ns, u64::MAX);
    }

    #[test]
    fn variable_buffer_metadata_and_transition_counts_share_one_publication() {
        let (diagnostics, mut recorder) = Diagnostics::new(96_000);
        recorder.record(recorder.begin(), 500_000, 96, 1, 20, true, false);
        recorder.record(recorder.begin(), 1_000_000, 192, 6, 21, false, true);
        recorder.record(recorder.begin(), 1_500_000, 192, 2, 22, true, true);
        recorder.record(recorder.begin(), 0, 24, 2, 23, false, false);
        let snapshot = diagnostics.snapshot();
        assert!(snapshot.available && snapshot.coherent);
        assert_eq!(snapshot.callback_sequence, 4);
        assert_eq!(snapshot.frames, 24);
        assert_eq!(snapshot.channels, 2);
        assert_eq!(snapshot.revision, 23);
        assert_eq!(snapshot.duration_ns, 0);
        assert_eq!(snapshot.utilization, Some(0.0));
        assert_eq!(snapshot.peak_utilization, Some(0.75));
        assert_eq!(snapshot.total_duration_ns, 3_000_000);
        assert_eq!(snapshot.total_frames, 504);
        assert_eq!(snapshot.transition_callbacks, 3);
        assert!(!snapshot.adopted_graph && !snapshot.crossfade);
        let json = serde_json::to_value(snapshot).unwrap();
        assert_eq!(json["sample_rate"], 96_000);
        assert_eq!(json["coherent"], true);
        assert_eq!(json["transition_callbacks"], 3);
    }

    #[test]
    fn invalid_callbacks_remain_observable_but_do_not_pollute_valid_aggregates() {
        let (diagnostics, mut recorder) = Diagnostics::new(48_000);
        record(&mut recorder, 400_000, 48);
        recorder.record(recorder.begin(), u64::MAX, 0, 2, 8, true, true);
        recorder.record(recorder.begin(), u64::MAX, 48, 0, 9, true, true);
        let snapshot = diagnostics.snapshot();
        assert!(snapshot.available);
        assert_eq!(snapshot.callback_sequence, 3);
        assert_eq!(snapshot.callbacks, 1);
        assert_eq!(snapshot.invalid_callbacks, 2);
        assert_eq!(snapshot.utilization, None);
        assert_eq!(snapshot.peak_utilization, Some(0.4));
        assert_eq!(snapshot.max_duration_ns, 400_000);
        assert_eq!(snapshot.total_duration_ns, 400_000);
        assert_eq!(snapshot.total_frames, 48);
        assert_eq!(snapshot.overruns, 0);
        assert_eq!(snapshot.transition_callbacks, 0);
        assert!(snapshot.adopted_graph && snapshot.crossfade);
        let (zero_rate, mut writer) = Diagnostics::new(0);
        record(&mut writer, 1, 1);
        let snapshot = zero_rate.snapshot();
        assert!(snapshot.available);
        assert_eq!(snapshot.callbacks, 0);
        assert_eq!(snapshot.invalid_callbacks, 1);
        assert_eq!(snapshot.utilization, None);
        assert_eq!(snapshot.peak_utilization, None);
        assert_eq!(snapshot.total_frames, 0);
    }

    #[test]
    fn reset_acknowledges_entry_epoch_without_resetting_monotonic_sequence() {
        let (diagnostics, mut recorder) = Diagnostics::new(48_000);
        record(&mut recorder, 2_000_000, 48);
        let entry_epoch = recorder.begin();
        assert_eq!(diagnostics.request_reset(), 1);
        let pending = diagnostics.snapshot();
        assert!(pending.reset_pending);
        assert_eq!(pending.epoch, 0);
        assert_eq!(pending.callbacks, 1);
        recorder.record(entry_epoch, 1_000_000, 48, 2, 8, true, false);
        let pending = diagnostics.snapshot();
        assert!(pending.reset_pending);
        assert_eq!(pending.epoch, 0);
        assert_eq!(pending.callbacks, 2);
        assert_eq!(pending.callback_sequence, 2);
        record(&mut recorder, 500_000, 24);
        let reset = diagnostics.snapshot();
        assert!(!reset.reset_pending);
        assert_eq!(reset.epoch, 1);
        assert_eq!(reset.requested_epoch, 1);
        assert_eq!(reset.callback_sequence, 3);
        assert_eq!(reset.callbacks, 1);
        assert_eq!(reset.overruns, 0);
        assert_eq!(reset.total_duration_ns, 500_000);
        assert_eq!(reset.total_frames, 24);
        assert_eq!(reset.transition_callbacks, 0);
        assert_eq!(diagnostics.request_reset(), 2);
        assert_eq!(diagnostics.request_reset(), 3);
        recorder.record(recorder.begin(), 100, 0, 2, 9, false, false);
        let reset = diagnostics.snapshot();
        assert_eq!(reset.epoch, 3);
        assert!(!reset.reset_pending);
        assert_eq!(reset.callbacks, 0);
        assert_eq!(reset.invalid_callbacks, 1);
        assert_eq!(reset.callback_sequence, 4);
        assert_eq!(reset.peak_utilization, None);
    }

    #[test]
    fn reset_between_begin_and_record_stays_pending_across_threads() {
        let (diagnostics, mut recorder) = Diagnostics::new(48_000);
        let barrier = Arc::new(Barrier::new(2));
        let worker_barrier = Arc::clone(&barrier);
        let writer = thread::spawn(move || {
            let epoch = recorder.begin();
            worker_barrier.wait();
            worker_barrier.wait();
            recorder.record(epoch, 10, 1, 2, 1, false, false);
            recorder
        });
        barrier.wait();
        assert_eq!(diagnostics.request_reset(), 1);
        barrier.wait();
        let mut recorder = writer.join().unwrap();
        assert!(diagnostics.snapshot().reset_pending);
        assert_eq!(diagnostics.snapshot().epoch, 0);
        record(&mut recorder, 20, 1);
        let acknowledged = diagnostics.snapshot();
        assert!(!acknowledged.reset_pending);
        assert_eq!(acknowledged.epoch, 1);
        assert_eq!(acknowledged.callbacks, 1);
        assert_eq!(acknowledged.total_duration_ns, 20);
        assert_eq!(acknowledged.callback_sequence, 2);
    }

    #[test]
    fn public_counters_saturate_while_private_publication_marker_can_wrap() {
        let (diagnostics, mut recorder) = Diagnostics::new(48_000);
        recorder.callback_sequence = u64::MAX - 1;
        recorder.interval.callbacks = u64::MAX - 1;
        recorder.interval.invalid_callbacks = u64::MAX - 1;
        recorder.interval.overruns = u64::MAX - 1;
        recorder.interval.transition_callbacks = u64::MAX - 1;
        recorder.interval.total_frames = u64::MAX - 1;
        recorder.interval.total_duration_ns = u64::MAX - 1;
        diagnostics.published.version.store(u64::MAX - 1, SeqCst);
        for _ in 0..2 {
            recorder.record(0, u64::MAX, 48, 2, 1, true, true);
        }
        let snapshot = diagnostics.snapshot();
        assert!(snapshot.available && snapshot.coherent);
        assert_eq!(snapshot.callback_sequence, u64::MAX);
        assert_eq!(snapshot.callbacks, u64::MAX);
        assert_eq!(snapshot.overruns, u64::MAX);
        assert_eq!(snapshot.transition_callbacks, u64::MAX);
        assert_eq!(snapshot.total_frames, u64::MAX);
        assert_eq!(snapshot.total_duration_ns, u64::MAX);
        for _ in 0..2 {
            recorder.record(0, 1, 0, 2, 1, false, false);
        }
        assert_eq!(diagnostics.snapshot().invalid_callbacks, u64::MAX);
        diagnostics.requested_epoch.store(u64::MAX - 1, SeqCst);
        assert_eq!(diagnostics.request_reset(), u64::MAX);
        assert_eq!(diagnostics.request_reset(), u64::MAX);
        record(&mut recorder, 1, 1);
        let reset = diagnostics.snapshot();
        assert_eq!(reset.epoch, u64::MAX);
        assert!(!reset.reset_pending);
        assert_eq!(reset.callbacks, 1);
        assert_eq!(reset.callback_sequence, u64::MAX);
    }

    #[test]
    fn concurrent_snapshots_never_mix_callback_fields_or_interval_totals() {
        let (diagnostics, mut recorder) = Diagnostics::new(48_000);
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = Arc::clone(&finished);
        let writer = thread::spawn(move || {
            for sequence in 1..=10_000_u64 {
                recorder.record(
                    recorder.begin(),
                    sequence * 100,
                    sequence as usize,
                    2,
                    sequence + 100,
                    sequence.is_multiple_of(2),
                    sequence.is_multiple_of(3),
                );
            }
            worker_finished.store(true, SeqCst);
        });
        let verify = |snapshot: Snapshot| {
            if !snapshot.available {
                return;
            }
            assert!(snapshot.coherent);
            let sequence = snapshot.callback_sequence;
            assert_eq!(snapshot.frames as u64, sequence);
            assert_eq!(snapshot.duration_ns, sequence * 100);
            assert_eq!(snapshot.revision, sequence + 100);
            assert_eq!(snapshot.callbacks, sequence);
            assert_eq!(snapshot.invalid_callbacks, 0);
            assert_eq!(snapshot.total_frames, sequence * (sequence + 1) / 2);
            assert_eq!(snapshot.total_duration_ns, snapshot.total_frames * 100);
            assert_eq!(snapshot.adopted_graph, sequence.is_multiple_of(2));
            assert_eq!(snapshot.crossfade, sequence.is_multiple_of(3));
            assert_eq!(
                snapshot.transition_callbacks,
                sequence / 2 + sequence / 3 - sequence / 6
            );
            assert_eq!(snapshot.overruns, 0);
        };
        while !finished.load(SeqCst) {
            verify(diagnostics.snapshot());
        }
        writer.join().unwrap();
        verify(diagnostics.snapshot());
        assert_eq!(diagnostics.snapshot().callback_sequence, 10_000);
    }
}
