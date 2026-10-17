// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from RackMCP 2.0.0 src/core/sequencer.{hpp,cpp},
// Copyright Zachary Scheffler, licensed GPL-3.0-or-later.
// Pinned source: 285e84c5ff2d3ecef25890da07f578af6a0c5ca6.
// This is the fixed playback store and sample engine only; names, selected
// pattern, automation lanes, and host control remain outside the kernel.

pub(crate) const MAX_PATTERNS: usize = 16;
pub(crate) const MAX_STEPS: usize = 64;
pub(crate) const TRACKS: usize = 4;
pub(crate) const MAX_VOICES: usize = 4;

const RETRIGGER_SECONDS: f64 = 0.001;
const PULSE_SECONDS: f64 = 0.001;
const STALL_RATIO: f64 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Step {
    pub(crate) on: bool,
    pub(crate) note_count: u8,
    pub(crate) notes: [f32; MAX_VOICES],
    pub(crate) vel: f32,
    pub(crate) gate: f32,
    pub(crate) prob: f32,
    pub(crate) ratchet: u8,
    pub(crate) accent: bool,
    pub(crate) slide: bool,
}

impl Step {
    const EMPTY: Self = Self {
        on: false,
        note_count: 0,
        notes: [0.0; MAX_VOICES],
        vel: 0.0,
        gate: 0.0,
        prob: 0.0,
        ratchet: 0,
        accent: false,
        slide: false,
    };
}

impl Default for Step {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Track {
    pub(crate) voices: u8,
    pub(crate) glide_ms: f32,
    pub(crate) steps: [Step; MAX_STEPS],
}

impl Track {
    const EMPTY: Self = Self {
        voices: 1,
        glide_ms: 80.0,
        steps: [Step::EMPTY; MAX_STEPS],
    };
}

impl Default for Track {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Pattern {
    pub(crate) length: u8,
    pub(crate) steps_per_beat: u8,
    pub(crate) tracks: [Track; TRACKS],
}

impl Pattern {
    const EMPTY: Self = Self {
        length: 16,
        steps_per_beat: 4,
        tracks: [Track::EMPTY; TRACKS],
    };
}

impl Default for Pattern {
    fn default() -> Self {
        Self::EMPTY
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Data {
    pub(crate) seed: u32,
    pub(crate) pattern_count: u8,
    pub(crate) patterns: [Pattern; MAX_PATTERNS],
}

impl Default for Data {
    fn default() -> Self {
        // The repeated constants avoid constructing an intermediate default
        // pattern or invoking a per-element initializer for this large store.
        Self {
            seed: 1,
            pattern_count: 1,
            patterns: [Pattern::EMPTY; MAX_PATTERNS],
        }
    }
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

pub(crate) fn chance(seed: u32, pattern: usize, loop_count: i64, step: usize, track: usize) -> f32 {
    let mut h = splitmix64(u64::from(seed));
    h = splitmix64(h ^ pattern as u64);
    h = splitmix64(h ^ loop_count as u64);
    h = splitmix64(h ^ ((step as u64) << 8) ^ track as u64);
    ((h >> 40) & 0xFF_FFFF) as f32 / 16_777_216.0_f32
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Outputs {
    pub(crate) voices: [usize; TRACKS],
    pub(crate) pitch: [[f32; MAX_VOICES]; TRACKS],
    pub(crate) gate: [f32; TRACKS],
    pub(crate) vel: [f32; TRACKS],
    pub(crate) accent: [f32; TRACKS],
    pub(crate) clock: bool,
    pub(crate) reset: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct TrackRt {
    active: bool,
    hold_through: bool,
    legato_in: bool,
    ratchets: i32,
    sub_len: f64,
    gate_len: f64,
    pitch: [f32; MAX_VOICES],
    target: [f32; MAX_VOICES],
    glide: [f32; MAX_VOICES],
    vel: f32,
    accent: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct Engine {
    active: usize,
    pending: Option<usize>,
    pending_now: bool,
    step: usize,
    loop_count: i64,
    started: bool,
    step_sample: f64,
    step_len: f64,
    beats_at_step: f64,
    beats_per_step: f64,
    samples_since_clock: f64,
    ext_period: f64,
    last_raw_period: f64,
    ext_edges: i32,
    clock_pulse: f64,
    reset_pulse: f64,
    reset_pending: bool,
    seek_pending: bool,
    seek_beats: f64,
    rt: [TrackRt; TRACKS],
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}

impl Engine {
    pub(crate) fn new() -> Self {
        Self {
            active: 0,
            pending: None,
            pending_now: false,
            step: 0,
            loop_count: 0,
            started: false,
            step_sample: 0.0,
            step_len: 1.0,
            beats_at_step: 0.0,
            beats_per_step: 0.25,
            samples_since_clock: 0.0,
            ext_period: 0.0,
            last_raw_period: 0.0,
            ext_edges: 0,
            clock_pulse: 0.0,
            reset_pulse: 0.0,
            reset_pending: false,
            seek_pending: false,
            seek_beats: 0.0,
            rt: [TrackRt::default(); TRACKS],
        }
    }

    pub(crate) fn reset(&mut self) {
        self.reset_pending = true;
    }

    pub(crate) fn seek(&mut self, mut beats: f64) {
        if beats.is_nan() || beats < 0.0 {
            beats = 0.0;
        }
        if beats > 5.0e6 {
            beats = 5.0e6;
        }
        self.seek_beats = beats;
        self.seek_pending = true;
    }

    /// Queue for the next advance (`now`) or the old pattern's next wrap.
    /// Indices outside the fixed store are rejected without changing the queue;
    /// process also discards requests outside the currently loaded pattern count.
    pub(crate) fn request_pattern(&mut self, index: usize, now: bool) -> bool {
        if index >= MAX_PATTERNS {
            return false;
        }
        self.pending = Some(index);
        self.pending_now = now;
        true
    }

    pub(crate) fn active_pattern(&self) -> usize {
        self.active
    }

    pub(crate) fn pending_pattern(&self) -> Option<usize> {
        self.pending
    }

    pub(crate) fn step_index(&self) -> usize {
        self.step
    }

    pub(crate) fn loop_count(&self) -> i64 {
        self.loop_count
    }

    pub(crate) fn beat_position(&self) -> f64 {
        let frac = if self.step_len > 0.0 {
            (self.step_sample / self.step_len).min(1.0)
        } else {
            0.0
        };
        self.beats_at_step
            + if self.started {
                frac * self.beats_per_step
            } else {
                0.0
            }
    }

    // min/max also retain the source's NaN fallback, unlike float::clamp.
    #[allow(clippy::manual_clamp)]
    fn start_step(&mut self, data: &Data, rate: f32, bpm: f32, swing: f32, ext: bool) {
        let pattern = &data.patterns[self.active];
        self.beats_per_step = 1.0 / f64::from(pattern.steps_per_beat);
        let base =
            f64::from(rate) * 60.0 / (f64::from(bpm.max(1.0)) * f64::from(pattern.steps_per_beat));
        self.step_len = if ext && self.ext_period > 0.0 {
            self.ext_period
        } else if ext {
            base
        } else {
            let s = (f64::from(swing) / 100.0).min(0.75).max(0.5);
            base * 2.0
                * if self.step.is_multiple_of(2) {
                    s
                } else {
                    1.0 - s
                }
        };
        self.step_sample = 0.0;
        self.clock_pulse = PULSE_SECONDS * f64::from(rate);

        for t in 0..TRACKS {
            let rt = &mut self.rt[t];
            let track = &pattern.tracks[t];
            let step = &track.steps[self.step];
            let prev_hold = rt.active && rt.hold_through;
            let mut fire = step.on;
            if fire && step.prob < 1.0 {
                fire = chance(data.seed, self.active, self.loop_count, self.step, t) < step.prob;
            }
            if !fire {
                rt.active = false;
                rt.hold_through = false;
                continue;
            }
            rt.active = true;
            rt.legato_in = prev_hold;
            rt.hold_through = step.slide;
            rt.ratchets = i32::from(step.ratchet).max(1);
            rt.sub_len = self.step_len / f64::from(rt.ratchets);
            rt.gate_len = f64::from(step.gate).max(0.01) * rt.sub_len;
            rt.vel = step.vel;
            rt.accent = step.accent;
            for v in 0..usize::from(track.voices) {
                if step.note_count == 0 {
                    continue;
                }
                let target = (step.notes[v % usize::from(step.note_count)] / 12.0)
                    .min(10.0)
                    .max(-10.0);
                rt.target[v] = target;
                let glide_samples = f64::from(track.glide_ms) * f64::from(rate) / 1000.0;
                if prev_hold && glide_samples >= 1.0 {
                    rt.glide[v] = (f64::from(target - rt.pitch[v]) / glide_samples) as f32;
                } else {
                    rt.pitch[v] = target;
                    rt.glide[v] = 0.0;
                }
            }
        }
    }

    /// Process one sample. Data must have the source parser's bounded lengths,
    /// voices, and note counts. Invalid sample rates use 1 Hz at this native
    /// boundary; valid inputs follow the pinned source's arithmetic and order.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn process(
        &mut self,
        data: &Data,
        rate: f32,
        bpm: f32,
        swing: f32,
        running: bool,
        ext_connected: bool,
        ext_rising: bool,
    ) -> Outputs {
        let rate = if rate.is_finite() && rate > 0.0 {
            rate
        } else {
            1.0
        };
        if self.active >= usize::from(data.pattern_count) {
            self.active = 0;
        }
        if self
            .pending
            .is_some_and(|p| p >= usize::from(data.pattern_count))
        {
            self.pending = None;
        }
        if self.step >= usize::from(data.patterns[self.active].length) {
            self.step = 0;
        }

        self.samples_since_clock += 1.0;
        if !ext_connected {
            self.ext_edges = 0;
        }
        if ext_rising {
            let gap = self.samples_since_clock;
            self.samples_since_clock = 0.0;
            self.ext_edges = self.ext_edges.wrapping_add(1);
            if self.ext_edges == 1 {
                self.ext_period = 0.0;
            } else if self.ext_period <= 0.0
                || gap <= STALL_RATIO * self.ext_period
                || (gap - self.last_raw_period).abs() <= 0.05 * gap
            {
                self.ext_period = gap;
            }
            self.last_raw_period = gap;
        }

        if self.reset_pending {
            self.reset_pending = false;
            if let Some(pending) = self.pending.take() {
                self.active = pending;
            }
            self.step = 0;
            self.loop_count = 0;
            self.beats_at_step = 0.0;
            self.step_sample = 0.0;
            self.started = false;
            for rt in &mut self.rt {
                rt.active = false;
            }
            self.reset_pulse = PULSE_SECONDS * f64::from(rate);
        }

        if self.seek_pending {
            self.seek_pending = false;
            let pattern = &data.patterns[self.active];
            let bps = 1.0 / f64::from(pattern.steps_per_beat.max(1));
            let len = f64::from(pattern.length.max(1));
            let total = (self.seek_beats.max(0.0) / bps + 1e-9).floor();
            self.step = (total % len) as usize;
            self.loop_count = (total / len) as i64;
            self.beats_at_step = total * bps;
            self.beats_per_step = bps;
            self.step_sample = 0.0;
            self.started = false;
            self.reset_pulse = 0.0;
            self.clock_pulse = 0.0;
            for rt in &mut self.rt {
                rt.active = false;
                rt.hold_through = false;
                rt.legato_in = false;
            }
        }

        if !running {
            self.started = false;
            for rt in &mut self.rt {
                rt.active = false;
            }
        } else {
            let mut advance = false;
            let mut begin = false;
            if !self.started {
                begin = if ext_connected { ext_rising } else { true };
            } else if ext_connected {
                self.step_sample += 1.0;
                advance = ext_rising;
            } else {
                self.step_sample += 1.0;
                advance = self.step_sample >= self.step_len;
            }
            if advance {
                self.beats_at_step += self.beats_per_step;
                self.step += 1;
                if self.step >= usize::from(data.patterns[self.active].length) {
                    self.step = 0;
                    self.loop_count += 1;
                }
                if let Some(pending) = self.pending {
                    if self.pending_now || self.step == 0 {
                        if pending != self.active {
                            self.loop_count = 0;
                        }
                        self.active = pending;
                        self.pending = None;
                        if self.step >= usize::from(data.patterns[self.active].length) {
                            self.step = 0;
                        }
                    }
                }
                begin = true;
            }
            if begin {
                self.started = true;
                self.start_step(data, rate, bpm, swing, ext_connected);
            }
        }

        let mut out = Outputs::default();
        let pattern = &data.patterns[self.active];
        let retrig = RETRIGGER_SECONDS * f64::from(rate);
        for t in 0..TRACKS {
            let rt = &mut self.rt[t];
            let voices = usize::from(pattern.tracks[t].voices);
            out.voices[t] = voices;
            for v in 0..voices {
                if rt.glide[v] != 0.0 {
                    rt.pitch[v] += rt.glide[v];
                    if (rt.glide[v] > 0.0 && rt.pitch[v] >= rt.target[v])
                        || (rt.glide[v] < 0.0 && rt.pitch[v] <= rt.target[v])
                    {
                        rt.pitch[v] = rt.target[v];
                        rt.glide[v] = 0.0;
                    }
                }
                out.pitch[t][v] = rt.pitch[v];
            }
            let mut gate = false;
            if rt.active {
                let mut sub = (self.step_sample / rt.sub_len) as i32;
                if sub > rt.ratchets - 1 {
                    sub = rt.ratchets - 1;
                }
                let within = self.step_sample - f64::from(sub) * rt.sub_len;
                let gap = if sub == 0 && rt.legato_in {
                    0.0
                } else {
                    retrig
                };
                gate = within >= gap && within < rt.gate_len + gap;
                if rt.hold_through && sub == rt.ratchets - 1 && within >= gap {
                    gate = true;
                }
            }
            out.gate[t] = if gate { 10.0 } else { 0.0 };
            out.vel[t] = rt.vel * 10.0;
            out.accent[t] = if gate && rt.accent { 10.0 } else { 0.0 };
        }
        out.clock = self.clock_pulse > 0.0;
        if self.clock_pulse > 0.0 {
            self.clock_pulse -= 1.0;
        }
        out.reset = self.reset_pulse > 0.0;
        if self.reset_pulse > 0.0 {
            self.reset_pulse -= 1.0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(engine: &mut Engine, data: &Data, running: bool, edge: bool) -> Outputs {
        engine.process(data, 1000.0, 60.0, 50.0, running, true, edge)
    }

    fn note(semitones: f32) -> Step {
        Step {
            on: true,
            note_count: 1,
            notes: [semitones, 0.0, 0.0, 0.0],
            vel: 0.8,
            gate: 0.5,
            prob: 1.0,
            ratchet: 1,
            accent: true,
            slide: false,
        }
    }

    #[test]
    fn selection_waits_for_advance_or_wrap_and_reset_consumes_it() {
        let mut data = Data {
            pattern_count: 2,
            ..Data::default()
        };
        data.patterns[0].length = 2;
        let mut engine = Engine::new();
        assert!(engine.request_pattern(1, true));
        assert!(!engine.request_pattern(MAX_PATTERNS, false));
        tick(&mut engine, &data, true, true);
        assert_eq!(engine.active_pattern(), 0);
        assert_eq!(engine.pending_pattern(), Some(1));
        tick(&mut engine, &data, true, true);
        assert_eq!((engine.active_pattern(), engine.step_index()), (1, 1));
        assert_eq!(engine.beat_position(), 0.25);

        engine.request_pattern(0, false);
        for _ in 0..14 {
            tick(&mut engine, &data, true, true);
        }
        assert_eq!((engine.active_pattern(), engine.step_index()), (1, 15));
        tick(&mut engine, &data, true, true);
        assert_eq!((engine.active_pattern(), engine.step_index()), (0, 0));
        assert_eq!(engine.loop_count(), 0);
        engine.request_pattern(1, false);
        engine.reset();
        let out = tick(&mut engine, &data, false, false);
        assert!(out.reset);
        assert_eq!((engine.active_pattern(), engine.step_index()), (1, 0));
        assert_eq!(engine.pending_pattern(), None);
        assert_eq!(engine.beat_position(), 0.0);
    }

    #[test]
    fn stop_holds_position_and_velocity_resume_replays_current_step() {
        let mut data = Data::default();
        data.patterns[0].tracks[0].steps[0] = note(12.0);
        let mut engine = Engine::new();
        tick(&mut engine, &data, true, true);
        assert_eq!(tick(&mut engine, &data, true, false).gate[0], 10.0);
        assert!(engine.beat_position() > 0.0);
        let stopped = tick(&mut engine, &data, false, false);
        assert_eq!(
            (stopped.gate[0], stopped.pitch[0][0], stopped.vel[0]),
            (0.0, 1.0, 8.0)
        );
        assert_eq!(engine.beat_position(), 0.0);
        tick(&mut engine, &data, true, false);
        assert!(!engine.started);
        let resumed = tick(&mut engine, &data, true, true);
        assert_eq!(engine.step_index(), 0);
        assert_eq!(resumed.gate[0], 0.0);
        assert!(resumed.clock);
    }

    #[test]
    fn successful_slide_glides_while_stopped_and_failed_step_breaks_tie() {
        let mut data = Data::default();
        let track = &mut data.patterns[0].tracks[0];
        track.glide_ms = 4.0;
        track.steps[0] = Step {
            slide: true,
            ..note(0.0)
        };
        track.steps[1] = Step {
            slide: true,
            ..note(12.0)
        };
        track.steps[2] = Step {
            prob: 0.0,
            ..note(24.0)
        };
        track.steps[3] = note(36.0);
        let mut engine = Engine::new();
        tick(&mut engine, &data, true, true);
        tick(&mut engine, &data, true, false);
        let slid = tick(&mut engine, &data, true, true);
        assert_eq!((slid.pitch[0][0], slid.gate[0]), (0.25, 10.0));
        let stopped = tick(&mut engine, &data, false, false);
        assert_eq!((stopped.pitch[0][0], stopped.gate[0]), (0.5, 0.0));
        tick(&mut engine, &data, false, false);
        assert_eq!(tick(&mut engine, &data, false, false).pitch[0][0], 1.0);

        // Seek rearms step 1, then let the failed probability step break its tie.
        engine.seek(0.25);
        tick(&mut engine, &data, true, true);
        assert_eq!(tick(&mut engine, &data, true, true).gate[0], 0.0);
        let after_rest = tick(&mut engine, &data, true, true);
        assert_eq!((after_rest.pitch[0][0], after_rest.gate[0]), (3.0, 0.0));
    }

    #[test]
    fn external_clock_ignores_first_gap_and_requires_two_agreeing_stall_gaps() {
        let data = Data::default();
        let mut engine = Engine::new();
        for _ in 0..300 {
            tick(&mut engine, &data, false, false);
        }
        tick(&mut engine, &data, true, true);
        assert_eq!(engine.step_len, 250.0);
        for _ in 0..9 {
            tick(&mut engine, &data, true, false);
        }
        tick(&mut engine, &data, true, true);
        assert_eq!(engine.step_len, 10.0);
        for _ in 0..99 {
            tick(&mut engine, &data, true, false);
        }
        assert_eq!(engine.beat_position(), 0.5);
        tick(&mut engine, &data, true, true);
        assert_eq!(engine.step_len, 10.0);
        for _ in 0..99 {
            tick(&mut engine, &data, true, false);
        }
        tick(&mut engine, &data, true, true);
        assert_eq!(engine.step_len, 100.0);

        // Clock measurement continues stopped and survives reset and seek.
        for _ in 0..99 {
            tick(&mut engine, &data, false, false);
        }
        tick(&mut engine, &data, false, true);
        engine.reset();
        engine.seek(0.75);
        let out = tick(&mut engine, &data, true, true);
        assert!(!out.reset);
        assert_eq!(engine.step_index(), 3);
        assert_eq!(engine.step_len, 1.0);
        assert_eq!(engine.beat_position(), 0.75);
    }
}
