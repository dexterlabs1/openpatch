//! Native output with control-thread compilation and bounded ownership transfer.
//! The running callback never allocates, locks, formats errors, or drops engines.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use openpatch_core::{Engine, MidiEvent, Patch, BLOCK_SIZE};
use rtrb::{Consumer, Producer, PushError, RingBuffer};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

pub const WAVEFORM_SIZE: usize = 256;
const QUEUE_SIZE: usize = 2;
const MIDI_QUEUE_SIZE: usize = 512;
const DEFAULT_VOLUME: f32 = 0.65;

struct Shared {
    playing: AtomicBool,
    midi_epoch: AtomicU64,
    midi_overflows: AtomicU64,
    frame_clock: AtomicU64,
    volume: AtomicU32,
    peak: AtomicU32,
    stream_error: AtomicBool,
    solver_failures: AtomicUsize,
    waveform: [AtomicU32; WAVEFORM_SIZE],
    waveform_cursor: AtomicUsize,
}

impl Shared {
    fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            midi_epoch: AtomicU64::new(1),
            midi_overflows: AtomicU64::new(0),
            frame_clock: AtomicU64::new(0),
            volume: AtomicU32::new(DEFAULT_VOLUME.to_bits()),
            peak: AtomicU32::new(0),
            stream_error: AtomicBool::new(false),
            solver_failures: AtomicUsize::new(0),
            waveform: std::array::from_fn(|_| AtomicU32::new(0)),
            waveform_cursor: AtomicUsize::new(0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ScheduledMidi {
    frame: u64,
    epoch: u64,
    event: MidiEvent,
}

struct MidiSender {
    queue: Producer<ScheduledMidi>,
    shared: Arc<Shared>,
}
impl MidiSender {
    #[cfg(test)]
    fn send(&mut self, frame: u64, event: MidiEvent) -> bool {
        let epoch = self.shared.midi_epoch.load(Ordering::SeqCst);
        self.send_in_epoch(frame, event, epoch)
    }
    fn send_observed(&mut self, frame: u64, event: MidiEvent, epoch: u64) -> bool {
        // Keyboard held-state belongs to the generation the UI observed. Never
        // retag that state with a reset generation it has not yet processed.
        if epoch != self.shared.midi_epoch.load(Ordering::SeqCst) {
            return false;
        }
        self.send_in_epoch(frame, event, epoch)
    }
    fn send_in_epoch(&mut self, frame: u64, event: MidiEvent, epoch: u64) -> bool {
        if !event.is_valid() {
            return false;
        }
        if event == MidiEvent::Panic {
            self.panic();
            return true;
        }
        if !self.shared.playing.load(Ordering::SeqCst) {
            return false;
        }
        let event = ScheduledMidi {
            frame,
            epoch,
            event,
        };
        if self.queue.push(event).is_err() {
            self.shared.midi_overflows.fetch_add(1, Ordering::Relaxed);
            self.panic();
            false
        } else {
            true
        }
    }
    fn panic(&self) {
        self.shared.midi_epoch.fetch_add(1, Ordering::SeqCst);
    }
}

/// Sole producer for hardware MIDI. Owned by the connection callback, never
/// shared with the virtual keyboard. Dropping/switching a connection resets BOTH
/// sources via its manager's reset handle.
pub struct HardwareMidiSender {
    sender: MidiSender,
    timestamp: TimestampMapper,
    rate: u32,
}
impl HardwareMidiSender {
    pub fn send_bytes(&mut self, timestamp_us: u64, bytes: &[u8]) -> bool {
        let Some(event) = openpatch_core::parse_midi(bytes) else {
            return false;
        };
        let epoch = self.sender.shared.midi_epoch.load(Ordering::SeqCst);
        let now = self.sender.shared.frame_clock.load(Ordering::Acquire);
        let frame = self.timestamp.map(timestamp_us, now, self.rate, epoch);
        self.sender.send_in_epoch(frame, event, epoch)
    }
    pub(crate) fn reset_handle(&self) -> MidiReset {
        MidiReset(self.sender.shared.clone())
    }
}

#[derive(Clone)]
pub(crate) struct MidiReset(Arc<Shared>);
impl MidiReset {
    pub(crate) fn panic(&self) {
        self.0.midi_epoch.fetch_add(1, Ordering::SeqCst);
    }
}

/// Input timestamps are relative to a connection, not to the audio device.
/// Anchor the first packet to the current completed sample frame; reconnect,
/// epoch change or a backwards timestamp reanchors. Late events play at the
/// next rendered frame; timestamps more than 100ms ahead are capped. Targets
/// remain FIFO within each source. Audio clock resets with every new stream.
#[derive(Default)]
struct TimestampMapper {
    anchor: Option<(u64, u64, u64)>,
    last: u64,
    last_us: u64,
}
impl TimestampMapper {
    fn map(&mut self, us: u64, now: u64, rate: u32, epoch: u64) -> u64 {
        if self
            .anchor
            .is_none_or(|(_, _, generation)| generation != epoch || us < self.last_us)
        {
            self.last = if self
                .anchor
                .is_some_and(|(_, _, generation)| generation == epoch)
            {
                self.last.max(now)
            } else {
                now
            };
            self.anchor = Some((us, now, epoch));
        }
        self.last_us = us;
        let (start, frame, _) = self.anchor.unwrap();
        let delta = (u128::from(us - start) * u128::from(rate) / 1_000_000)
            .min(u128::from(u64::MAX)) as u64;
        let target = frame
            .saturating_add(delta)
            .clamp(now, now.saturating_add(u64::from(rate) / 10));
        self.last = target.max(self.last);
        self.last
    }
}

struct MidiQueue {
    queue: Consumer<ScheduledMidi>,
    head: Option<ScheduledMidi>,
}
impl MidiQueue {
    fn fill(&mut self, epoch: u64, budget: &mut usize) {
        if self.head.is_some_and(|v| v.epoch < epoch) {
            self.head = None;
        }
        for _ in 0..MIDI_QUEUE_SIZE {
            if self.head.is_some() || *budget == 0 {
                break;
            }
            match self.queue.pop() {
                Ok(v) => {
                    *budget -= 1;
                    if v.epoch >= epoch {
                        self.head = Some(v);
                    }
                }
                Err(_) => break,
            }
        }
    }
}
struct MidiQueues {
    hardware: MidiQueue,
    ui: MidiQueue,
    hardware_sender: Option<MidiSender>,
    ui_sender: Option<MidiSender>,
    shared: Arc<Shared>,
    epoch: u64,
    refill_budget: usize,
}
impl MidiQueues {
    fn new(shared: Arc<Shared>) -> Self {
        let (hardware, hardware_queue) = RingBuffer::new(MIDI_QUEUE_SIZE);
        let (ui, ui_queue) = RingBuffer::new(MIDI_QUEUE_SIZE);
        Self {
            hardware: MidiQueue {
                queue: hardware_queue,
                head: None,
            },
            ui: MidiQueue {
                queue: ui_queue,
                head: None,
            },
            hardware_sender: Some(MidiSender {
                queue: hardware,
                shared: shared.clone(),
            }),
            ui_sender: Some(MidiSender {
                queue: ui,
                shared: shared.clone(),
            }),
            epoch: shared.midi_epoch.load(Ordering::SeqCst),
            refill_budget: MIDI_QUEUE_SIZE * 4,
            shared,
        }
    }
    fn take_ui(&mut self) -> Option<MidiSender> {
        self.ui_sender.take()
    }
    fn take_hardware(&mut self) -> Option<MidiSender> {
        self.hardware_sender.take()
    }
    fn sync_epoch(&mut self) -> bool {
        let epoch = self.shared.midi_epoch.load(Ordering::SeqCst);
        if epoch == self.epoch {
            return false;
        }
        self.epoch = epoch;
        // A producer can publish the NEW epoch between our previous load and
        // queue fill. Preserve that head; only discard older generations.
        if self.ui.head.is_some_and(|v| v.epoch < epoch) {
            self.ui.head = None;
        }
        if self.hardware.head.is_some_and(|v| v.epoch < epoch) {
            self.hardware.head = None;
        }
        true
    }
    fn fill(&mut self) {
        self.hardware.fill(self.epoch, &mut self.refill_budget);
        self.ui.fill(self.epoch, &mut self.refill_budget);
    }
    fn next_frame(&mut self) -> Option<u64> {
        self.fill();
        match (
            self.hardware.head.filter(|v| v.epoch == self.epoch),
            self.ui.head.filter(|v| v.epoch == self.epoch),
        ) {
            (Some(a), Some(b)) => Some(a.frame.min(b.frame)),
            (Some(a), None) | (None, Some(a)) => Some(a.frame),
            _ => None,
        }
    }
    fn pop_due(&mut self, now: u64) -> Option<MidiEvent> {
        self.fill();
        let queue = match (
            self.hardware.head.filter(|v| v.epoch == self.epoch),
            self.ui.head.filter(|v| v.epoch == self.epoch),
        ) {
            (Some(a), Some(b)) if a.frame <= b.frame => &mut self.hardware,
            (Some(_), Some(_)) => &mut self.ui,
            (Some(_), None) => &mut self.hardware,
            (None, Some(_)) => &mut self.ui,
            _ => return None,
        };
        if queue.head.is_some_and(|v| v.frame <= now) {
            queue.head.take().map(|v| v.event)
        } else {
            None
        }
    }
}

/// The stream keeps running during transport pause so gain can fade to silence.
/// Call `service` on each UI update to dispose of retired graphs and publish edits.
pub struct Audio {
    // Drop the stream before its control-side queue endpoints. CPAL shuts down the
    // callback before returning from Stream::drop; graph teardown is not rendering.
    _stream: cpal::Stream,
    incoming: Producer<Engine>,
    retired: Consumer<Engine>,
    pending: Option<Engine>,
    shared: Arc<Shared>,
    pub sample_rate: u32,
    pub device_name: String,
    pub device_channels: u16,
    midi_ui: MidiSender,
    midi_hardware: Option<HardwareMidiSender>,
}

impl Audio {
    pub fn new(patch: &Patch) -> Result<Self, String> {
        Self::new_with_device(patch, None)
    }

    pub fn output_devices() -> Result<Vec<String>, String> {
        let host = cpal::default_host();
        let devices = host.output_devices().map_err(|e| e.to_string())?;
        Ok(devices.filter_map(|d| d.name().ok()).collect())
    }

    pub fn new_with_device(patch: &Patch, selected: Option<&str>) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = if let Some(name) = selected {
            host.output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.name().ok().as_deref() == Some(name))
                .ok_or_else(|| format!("Output device '{name}' is unavailable"))?
        } else {
            host.default_output_device()
                .ok_or("No output device found. Connect an audio device and retry.")?
        };
        let device_name = device.name().unwrap_or_else(|_| "Default output".into());
        let explicit_stereo = patch.output.as_ref().is_some_and(|id| {
            patch
                .nodes
                .iter()
                .any(|n| &n.id == id && n.kind == "audio_stereo")
        });
        let supported = choose_config(&device, patch.sample_rate, explicit_stereo)?;
        let sample_rate = supported.sample_rate().0;
        let config = supported.config();
        if config.channels == 0 {
            return Err("The output device reported zero audio channels.".into());
        }
        let mut device_patch = patch.clone();
        device_patch.sample_rate = sample_rate;
        let engine = device_patch.compile()?;
        let (incoming, updates) = RingBuffer::new(QUEUE_SIZE);
        let (recycle, retired) = RingBuffer::new(QUEUE_SIZE);
        let shared = Arc::new(Shared::new());
        let mut callback = Callback::new(engine, updates, recycle, shared.clone(), sample_rate);
        let midi_ui = callback.midi.take_ui().unwrap();
        let midi_hardware = Some(HardwareMidiSender {
            sender: callback.midi.take_hardware().unwrap(),
            timestamp: TimestampMapper::default(),
            rate: sample_rate,
        });
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, callback),
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, callback),
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, callback),
            _ => unreachable!("choose_config only selects supported sample formats"),
        }
        .map_err(|error| format!("Could not create audio stream: {error}"))?;
        stream
            .play()
            .map_err(|error| format!("Could not start audio stream: {error}"))?;
        Ok(Self {
            _stream: stream,
            incoming,
            retired,
            pending: None,
            shared,
            sample_rate,
            device_name,
            device_channels: config.channels,
            midi_ui,
            midi_hardware,
        })
    }

    /// Compilation and superseded-edit disposal happen here, on the UI thread.
    /// If the callback is busy, the latest successfully compiled edit is retained.
    pub fn submit_patch(&mut self, patch: &Patch) -> Result<(), String> {
        let mut device_patch = patch.clone();
        device_patch.sample_rate = self.sample_rate;
        let engine = device_patch.compile()?;
        self.pending = Some(engine);
        self.service();
        Ok(())
    }

    pub fn service(&mut self) {
        while let Ok(engine) = self.retired.pop() {
            drop(engine);
        }
        if let Some(engine) = self.pending.take() {
            if let Err(PushError::Full(engine)) = self.incoming.push(engine) {
                self.pending = Some(engine);
            }
        }
    }

    pub fn set_playing(&self, playing: bool) {
        if self.shared.playing.swap(playing, Ordering::SeqCst) != playing {
            self.panic_midi();
        }
    }

    pub fn take_midi_sender(&mut self) -> Option<HardwareMidiSender> {
        self.midi_hardware.take()
    }
    /// Submit UI performance state using the generation last observed when
    /// synchronizing its held keys. Stale generations are rejected; a concurrent
    /// reset after this check still invalidates the event's original generation.
    pub fn send_midi_in_epoch(&mut self, event: MidiEvent, observed_epoch: u64) -> bool {
        let frame = self.shared.frame_clock.load(Ordering::Acquire);
        self.midi_ui.send_observed(frame, event, observed_epoch)
    }
    /// Global performance reset: invalidates pending UI and hardware events.
    pub fn panic_midi(&self) {
        self.shared.midi_epoch.fetch_add(1, Ordering::SeqCst);
    }
    pub fn midi_epoch(&self) -> u64 {
        self.shared.midi_epoch.load(Ordering::SeqCst)
    }
    pub fn midi_overflows(&self) -> u64 {
        self.shared.midi_overflows.load(Ordering::Relaxed)
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Ordering::Relaxed)
    }

    pub fn set_volume(&self, volume: f32) {
        let volume = if volume.is_finite() {
            volume.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.shared
            .volume
            .store(volume.to_bits(), Ordering::Relaxed);
    }

    pub fn volume(&self) -> f32 {
        f32::from_bits(self.shared.volume.load(Ordering::Relaxed))
    }

    pub fn peak(&self) -> f32 {
        f32::from_bits(self.shared.peak.load(Ordering::Relaxed))
    }

    /// A best-effort scope snapshot, oldest sample first; no shared mutable floats.
    pub fn waveform(&self) -> [f32; WAVEFORM_SIZE] {
        let cursor = self.shared.waveform_cursor.load(Ordering::Acquire);
        std::array::from_fn(|offset| {
            let index = (cursor + offset) % WAVEFORM_SIZE;
            f32::from_bits(self.shared.waveform[index].load(Ordering::Relaxed))
        })
    }

    pub fn take_stream_error(&self) -> bool {
        self.shared.stream_error.swap(false, Ordering::Relaxed)
    }

    pub fn solver_failures(&self) -> usize {
        self.shared.solver_failures.load(Ordering::Relaxed)
    }
}

fn supported_format(format: cpal::SampleFormat) -> bool {
    matches!(
        format,
        cpal::SampleFormat::F32 | cpal::SampleFormat::I16 | cpal::SampleFormat::U16
    )
}

fn choose_config(
    device: &cpal::Device,
    preferred_rate: u32,
    stereo: bool,
) -> Result<cpal::SupportedStreamConfig, String> {
    if let Ok(config) = device.default_output_config() {
        if supported_format(config.sample_format())
            && (!stereo || config.channels() >= 2)
            && (8000..=192000).contains(&config.sample_rate().0)
        {
            return Ok(config);
        }
    }
    let ranges = device
        .supported_output_configs()
        .map_err(|error| format!("Could not inspect audio output formats: {error}"))?;
    let mut best: Option<(u32, cpal::SupportedStreamConfig)> = None;
    for range in ranges {
        if !supported_format(range.sample_format()) || range.channels() == 0 {
            continue;
        }
        let min = range.min_sample_rate().0.max(8000);
        let max = range.max_sample_rate().0.min(192000);
        if min > max {
            continue;
        }
        let rate = preferred_rate.clamp(min, max);
        let distance = rate.abs_diff(preferred_rate)
            + if stereo && range.channels() < 2 {
                1_000_000
            } else {
                0
            };
        if best.as_ref().is_none_or(|(score, _)| distance < *score) {
            best = Some((distance, range.with_sample_rate(cpal::SampleRate(rate))));
        }
    }
    best.map(|(_, config)| config)
        .ok_or_else(|| "Output device needs f32, i16, or u16 audio at 8–192 kHz.".into())
}

/// Holds the previous graph until its crossfade ends, then retains it locally if
/// the recycle queue is full. Never discard a Full error: it owns the graph.
struct Exchange<T> {
    active: T,
    previous: Option<T>,
    waiting_retirement: Option<T>,
    incoming: Consumer<T>,
    retired: Producer<T>,
}

impl<T> Exchange<T> {
    fn poll(&mut self) -> bool {
        self.recycle();
        if self.waiting_retirement.is_some() || self.previous.is_some() {
            return false;
        }
        if let Ok(next) = self.incoming.pop() {
            self.previous = Some(std::mem::replace(&mut self.active, next));
            true
        } else {
            false
        }
    }

    fn finish_transition(&mut self) {
        // poll never starts another transition while retirement is blocked.
        debug_assert!(self.waiting_retirement.is_none());
        self.waiting_retirement = self.previous.take();
        self.recycle();
    }

    fn recycle(&mut self) {
        if let Some(previous) = self.waiting_retirement.take() {
            if let Err(PushError::Full(previous)) = self.retired.push(previous) {
                self.waiting_retirement = Some(previous);
            }
        }
    }
}

struct Gain {
    value: f32,
    coefficient: f32,
}

impl Gain {
    fn new(sample_rate: u32) -> Self {
        Self {
            value: 0.0,
            coefficient: 1.0 - (-1.0 / (sample_rate as f32 * 0.010)).exp(),
        }
    }

    fn next(&mut self, target: f32) -> f32 {
        self.value += self.coefficient * (target - self.value);
        if (target - self.value).abs() < 0.000_001 {
            self.value = target;
        }
        self.value
    }
}

struct Callback {
    graphs: Exchange<Engine>,
    shared: Arc<Shared>,
    gain: Gain,
    fade_length: usize,
    fade_position: usize,
    scope_cursor: usize,
    scope_decimation: usize,
    midi: MidiQueues,
    frame_clock: u64,
}

impl Callback {
    fn new(
        engine: Engine,
        incoming: Consumer<Engine>,
        retired: Producer<Engine>,
        shared: Arc<Shared>,
        rate: u32,
    ) -> Self {
        let midi = MidiQueues::new(shared.clone());
        Self {
            midi,
            frame_clock: 0,
            graphs: Exchange {
                active: engine,
                previous: None,
                waiting_retirement: None,
                incoming,
                retired,
            },
            shared,
            gain: Gain::new(rate),
            fade_length: (rate as usize / 200).max(1),
            fade_position: 0,
            scope_cursor: 0,
            scope_decimation: 0,
        }
    }

    fn render<T: OutputSample>(&mut self, output: &mut [T], channels: usize) {
        if self.graphs.poll() {
            if let Some(previous) = self.graphs.previous.as_ref() {
                self.graphs.active.adopt_state_from(previous);
            }
            self.fade_position = 0;
        }
        let target = if self.shared.playing.load(Ordering::Relaxed) {
            f32::from_bits(self.shared.volume.load(Ordering::Relaxed))
        } else {
            0.0
        };
        let mut peak = 0.0_f32;
        let mut midi_budget = MIDI_QUEUE_SIZE * 2;
        self.midi.refill_budget = MIDI_QUEUE_SIZE * 4;
        let mut midi_flooded = false;
        // Fixed stack buffers handle any device callback length and channel count.
        let mut current = [[0.0_f32; 2]; BLOCK_SIZE];
        let mut previous = [[0.0_f32; 2]; BLOCK_SIZE];
        if channels == 0 {
            return;
        }
        for chunk in output.chunks_mut(BLOCK_SIZE * channels) {
            let frames = chunk.len() / channels;
            let mut offset = 0;
            while offset < frames {
                self.sync_midi_reset();
                let playing = self.shared.playing.load(Ordering::SeqCst);
                // Each producer is FIFO; equal-time hardware precedes UI events.
                // Bounded draining also protects the callback from a busy producer.
                while !midi_flooded && midi_budget > 0 {
                    let Some(event) = self.midi.pop_due(self.frame_clock) else {
                        break;
                    };
                    midi_budget -= 1;
                    if playing {
                        self.graphs.active.handle_midi(event);
                        if let Some(old) = self.graphs.previous.as_mut() {
                            old.handle_midi(event);
                        }
                    }
                }
                // Never extend the event budget by splitting audio into more
                // spans. Flooding resets both sources and defers all further
                // input until the next callback, while audio keeps rendering.
                if !midi_flooded && (midi_budget == 0 || self.midi.refill_budget == 0) {
                    self.shared.midi_overflows.fetch_add(1, Ordering::Relaxed);
                    self.shared.midi_epoch.fetch_add(1, Ordering::SeqCst);
                    midi_flooded = true;
                }
                self.sync_midi_reset();
                let remaining = frames - offset;
                let next_frame = if midi_flooded {
                    None
                } else {
                    self.midi.next_frame()
                };
                let count = next_frame.map_or(remaining, |at| {
                    (at.saturating_sub(self.frame_clock) as usize).clamp(1, remaining)
                });
                self.graphs
                    .active
                    .process_audio_block(&mut current[..count]);
                let active_stereo = self.graphs.active.output_channels() == 2;
                let previous_stereo = self
                    .graphs
                    .previous
                    .as_ref()
                    .is_some_and(|g| g.output_channels() == 2);
                if let Some(engine) = self.graphs.previous.as_mut() {
                    engine.process_audio_block(&mut previous[..count]);
                }
                for index in 0..count {
                    let old = self.graphs.previous.as_ref().map(|_| previous[index]);
                    let mix = if old.is_some() {
                        let mix = (self.fade_position as f32 / self.fade_length as f32).min(1.0);
                        self.fade_position += 1;
                        mix
                    } else {
                        1.0
                    };
                    let gain = self.gain.next(target);
                    let sample = std::array::from_fn::<_, 2, _>(|channel| {
                        finite_sample(
                            (old.map_or(0.0, |v| v[channel] * (1.0 - mix))
                                + current[index][channel] * mix)
                                * gain,
                        )
                    });
                    peak = peak.max(sample[0].abs()).max(sample[1].abs());
                    let start = (offset + index) * channels;
                    // Map each graph before blending: mono extra channels fade
                    // smoothly when an edit selects an explicit stereo sink.
                    for (channel, value) in chunk[start..start + channels].iter_mut().enumerate() {
                        let new = device_sample(current[index], active_stereo, channel, channels);
                        let old = old.map_or(0.0, |v| {
                            device_sample(v, previous_stereo, channel, channels)
                        });
                        *value = T::from_mono((old * (1.0 - mix) + new * mix) * gain);
                    }
                    // Scope is the L/R average for an explicit stereo sink.
                    let mono = sample[0] * 0.5 + sample[1] * 0.5;
                    if self.scope_decimation == 0 {
                        self.shared.waveform[self.scope_cursor]
                            .store(mono.to_bits(), Ordering::Relaxed);
                        self.scope_cursor = (self.scope_cursor + 1) % WAVEFORM_SIZE;
                    }
                    self.scope_decimation = (self.scope_decimation + 1) % 4;
                }
                offset += count;
                self.frame_clock += count as u64;
                self.shared
                    .frame_clock
                    .store(self.frame_clock, Ordering::Release);
                if self.graphs.previous.is_some() && self.fade_position >= self.fade_length {
                    self.graphs.finish_transition();
                }
            }
            // CPAL normally supplies complete frames; incomplete tails are silence.
            chunk[frames * channels..].fill(T::from_mono(0.0));
        }
        self.shared
            .waveform_cursor
            .store(self.scope_cursor, Ordering::Release);
        let previous_peak = f32::from_bits(self.shared.peak.load(Ordering::Relaxed));
        self.shared.solver_failures.store(
            self.graphs.active.solver_failures().min(usize::MAX as u64) as usize,
            Ordering::Relaxed,
        );
        self.shared
            .peak
            .store(peak.max(previous_peak * 0.92).to_bits(), Ordering::Relaxed);
    }
    fn sync_midi_reset(&mut self) {
        if self.midi.sync_epoch() {
            self.graphs.active.handle_midi(MidiEvent::Panic);
            if let Some(old) = self.graphs.previous.as_mut() {
                old.handle_midi(MidiEvent::Panic);
            }
        }
    }
}

/// Legacy mono feeds every hardware channel. Explicit stereo feeds L/R only;
/// a one-channel device receives their arithmetic mean (no amplitude doubling).
fn device_sample(sample: [f32; 2], stereo: bool, channel: usize, channels: usize) -> f32 {
    if !stereo {
        sample[0]
    } else if channels == 1 {
        sample[0] * 0.5 + sample[1] * 0.5
    } else if channel < 2 {
        sample[channel]
    } else {
        0.0
    }
}

fn finite_sample(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(-1.0, 1.0)
    } else {
        0.0
    }
}

trait OutputSample: cpal::SizedSample + Copy {
    fn from_mono(value: f32) -> Self;
}

impl OutputSample for f32 {
    fn from_mono(value: f32) -> Self {
        finite_sample(value)
    }
}

impl OutputSample for i16 {
    fn from_mono(value: f32) -> Self {
        let value = finite_sample(value);
        (value * if value < 0.0 { 32768.0 } else { 32767.0 }).round() as i16
    }
}

impl OutputSample for u16 {
    fn from_mono(value: f32) -> Self {
        // The exact unsigned silence midpoint is 32768, not 32767.
        (i16::from_mono(value) as i32 + 32768) as u16
    }
}

fn build_stream<T: OutputSample>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut callback: Callback,
) -> Result<cpal::Stream, cpal::BuildStreamError> {
    let channels = config.channels as usize;
    let errors = callback.shared.clone();
    device.build_output_stream(
        config,
        move |output: &mut [T], _| callback.render(output, channels),
        move |_| {
            errors.stream_error.store(true, Ordering::Relaxed);
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::cell::Cell;

    struct TrackedAllocator;

    thread_local! {
        static TRACKING: Cell<bool> = const { Cell::new(false) };
        static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
        static DEALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }

    #[global_allocator]
    static ALLOCATOR: TrackedAllocator = TrackedAllocator;

    unsafe impl GlobalAlloc for TrackedAllocator {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let _ = TRACKING.try_with(|tracking| {
                if tracking.get() {
                    ALLOCATIONS.with(|count| count.set(count.get() + 1));
                }
            });
            unsafe { System.alloc(layout) }
        }

        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            let _ = TRACKING.try_with(|tracking| {
                if tracking.get() {
                    DEALLOCATIONS.with(|count| count.set(count.get() + 1));
                }
            });
            unsafe { System.dealloc(pointer, layout) }
        }
    }

    fn without_heap_activity(operation: impl FnOnce()) {
        ALLOCATIONS.with(|count| count.set(0));
        DEALLOCATIONS.with(|count| count.set(0));
        TRACKING.with(|tracking| tracking.set(true));
        operation();
        TRACKING.with(|tracking| tracking.set(false));
        assert_eq!(ALLOCATIONS.with(Cell::get), 0, "callback allocated");
        assert_eq!(DEALLOCATIONS.with(Cell::get), 0, "callback deallocated");
    }

    #[test]
    fn sample_formats_have_correct_silence_endpoints_and_clipping() {
        assert_eq!(i16::from_mono(-1.0), i16::MIN);
        assert_eq!(i16::from_mono(1.0), i16::MAX);
        assert_eq!(i16::from_mono(0.0), 0);
        assert_eq!(u16::from_mono(-1.0), 0);
        assert_eq!(u16::from_mono(1.0), u16::MAX);
        assert_eq!(u16::from_mono(0.0), 32768);
        assert_eq!(f32::from_mono(8.0), 1.0);
        assert_eq!(f32::from_mono(f32::NAN), 0.0);
        assert_eq!(u16::from_mono(f32::INFINITY), 32768);
    }

    #[test]
    fn transport_gain_fades_in_and_out_monotonically() {
        let mut gain = Gain::new(48000);
        let first = gain.next(1.0);
        assert!(first > 0.0 && first < 0.01);
        for _ in 0..4800 {
            let before = gain.value;
            assert!(gain.next(1.0) >= before);
        }
        assert!((gain.value - 1.0).abs() < 0.0001);
        for _ in 0..9600 {
            let before = gain.value;
            assert!(gain.next(0.0) <= before);
        }
        assert_eq!(gain.value, 0.0);
    }

    #[test]
    fn full_retirement_queue_preserves_every_graph_without_callback_drops() {
        struct Graph(usize, Arc<AtomicUsize>);
        impl Drop for Graph {
            fn drop(&mut self) {
                self.1.fetch_add(1, Ordering::Relaxed);
            }
        }
        let drops = Arc::new(AtomicUsize::new(0));
        let graph = |id| Graph(id, drops.clone());
        let (mut updates, incoming) = RingBuffer::new(2);
        let (retired, mut disposal) = RingBuffer::new(1);
        let mut exchange = Exchange {
            active: graph(0),
            previous: None,
            waiting_retirement: None,
            incoming,
            retired,
        };
        assert!(updates.push(graph(1)).is_ok());
        assert!(exchange.poll());
        exchange.finish_transition();
        assert!(updates.push(graph(2)).is_ok());
        assert!(exchange.poll());
        exchange.finish_transition();
        assert!(exchange.waiting_retirement.is_some());
        assert!(updates.push(graph(3)).is_ok());
        assert!(!exchange.poll());
        assert_eq!(exchange.active.0, 2);
        assert_eq!(drops.load(Ordering::Relaxed), 0);
        drop(disposal.pop().unwrap());
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        assert!(exchange.poll());
        assert_eq!(exchange.active.0, 3);
        exchange.finish_transition();
        // Even without a UI consumer, callback retains ownership safely.
        drop(disposal);
        assert!(!exchange.poll());
        assert_eq!(drops.load(Ordering::Relaxed), 1);
        drop(exchange);
        drop(updates);
        assert_eq!(drops.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn arbitrary_callback_size_duplicates_mono_and_crossfades_edits() {
        let mut patch = Patch::default();
        patch.nodes.push(openpatch_core::Node {
            id: "dc".into(),
            kind: "constant".into(),
            params: [("value".into(), 1.0)].into(),
        });
        patch.output = Some("dc".into());
        let (mut updates, incoming) = RingBuffer::new(2);
        let (retired, mut disposal) = RingBuffer::new(2);
        let shared = Arc::new(Shared::new());
        shared.playing.store(true, Ordering::Relaxed);
        shared.volume.store(1.0_f32.to_bits(), Ordering::Relaxed);
        let mut callback =
            Callback::new(patch.compile().unwrap(), incoming, retired, shared, 48000);
        let mut stereo = [0.0_f32; 2034];
        without_heap_activity(|| callback.render(&mut stereo, 2));
        for frame in stereo.as_chunks::<2>().0 {
            assert_eq!(frame[0], frame[1]);
        }
        callback.gain.value = 1.0;
        patch.nodes[0].params.insert("value".into(), -1.0);
        assert!(updates.push(patch.compile().unwrap()).is_ok());
        without_heap_activity(|| callback.render(&mut stereo, 2));
        assert_eq!(stereo[0], 1.0);
        assert_eq!(*stereo.last().unwrap(), -1.0);
        for frames in stereo.as_chunks::<2>().0.windows(2) {
            assert!((frames[0][0] - frames[1][0]).abs() < 0.01);
        }
        assert!(disposal.pop().is_ok());
    }

    #[test]
    fn live_stateful_edit_adopts_phase_and_filter_memory_without_heap_activity() {
        let polyphonic: Patch =
            serde_json::from_str(include_str!("../../../examples/polyphonic-voice.json")).unwrap();
        for original in [
            openpatch_core::demo_patch(),
            openpatch_core::analog_demo_patch(),
            polyphonic,
        ] {
            let (mut updates, incoming) = RingBuffer::new(2);
            let (retired, _disposal) = RingBuffer::new(2);
            let shared = Arc::new(Shared::new());
            shared.playing.store(true, Ordering::Relaxed);
            shared.volume.store(1.0_f32.to_bits(), Ordering::Relaxed);
            let mut callback = Callback::new(
                original.compile().unwrap(),
                incoming,
                retired,
                shared,
                48000,
            );
            callback.gain.value = 1.0;
            let mut warmup = [0.0_f32; 129];
            callback.render(&mut warmup, 1);

            let mut edited = original.clone();
            edited
                .nodes
                .iter_mut()
                .find(|node| matches!(node.kind.as_str(), "oscillator" | "analog_vco"))
                .unwrap()
                .params
                .insert("frequency".into(), 440.0);
            let mut new_reference = edited.compile().unwrap();
            new_reference.adopt_state_from(&callback.graphs.active);
            let mut old_reference = original.compile().unwrap();
            old_reference.process_block(&mut warmup);
            let mut current = [0.0_f32; 1025];
            let mut previous = [0.0_f32; 1025];
            new_reference.process_block(&mut current);
            old_reference.process_block(&mut previous);
            assert!(updates.push(edited.compile().unwrap()).is_ok());
            let mut actual = [0.0_f32; 1025];
            without_heap_activity(|| callback.render(&mut actual, 1));
            for (index, sample) in actual.iter().enumerate() {
                let mix = (index as f32 / callback.fade_length as f32).min(1.0);
                let expected = previous[index] * (1.0 - mix) + current[index] * mix;
                assert!(
                    (sample - expected).abs() < 0.000_001,
                    "sample {index}: {sample} vs {expected}"
                );
            }
        }
    }
    fn midi_patch() -> Patch {
        serde_json::from_value(serde_json::json!({
            "version":2,"sample_rate":48000,"nodes":[{"id":"midi","kind":"midi_cv","params":{"channels":1.0}}],
            "cables":[],"output":"midi","output_port":"gate"
        })).unwrap()
    }
    fn callback_for(patch: &Patch) -> (Callback, Producer<Engine>, Consumer<Engine>) {
        let (updates, incoming) = RingBuffer::new(2);
        let (retired, disposal) = RingBuffer::new(2);
        let shared = Arc::new(Shared::new());
        shared.playing.store(true, Ordering::SeqCst);
        shared.volume.store(1.0_f32.to_bits(), Ordering::Relaxed);
        let mut callback =
            Callback::new(patch.compile().unwrap(), incoming, retired, shared, 48000);
        callback.gain.value = 1.0;
        (callback, updates, disposal)
    }
    fn note_on(note: u8) -> MidiEvent {
        MidiEvent::NoteOn {
            channel: 0,
            note,
            velocity: 100,
        }
    }
    fn note_off(note: u8) -> MidiEvent {
        MidiEvent::NoteOff {
            channel: 0,
            note,
            velocity: 0,
        }
    }

    #[test]
    fn stereo_sink_mapping_is_explicit_and_mono_compatibility_is_unchanged() {
        assert_eq!(device_sample([0.25, -0.75], false, 4, 6), 0.25);
        assert_eq!(device_sample([0.25, -0.75], true, 0, 6), 0.25);
        assert_eq!(device_sample([0.25, -0.75], true, 1, 6), -0.75);
        assert_eq!(device_sample([0.25, -0.75], true, 2, 6), 0.0);
        assert_eq!(device_sample([0.25, -0.75], true, 0, 1), -0.25);
        let patch: Patch = serde_json::from_value(serde_json::json!({
            "version":2,"sample_rate":48000,
            "nodes":[{"id":"l","kind":"constant","params":{"value":0.5}},
            {"id":"r","kind":"constant","params":{"value":-0.25}},
            {"id":"out","kind":"audio_stereo","params":{}}],
            "cables":[{"from":"l","to":"out","input":"left"},{"from":"r","to":"out","input":"right"}],
            "output":"out"
        })).unwrap();
        let (mut callback, _, _) = callback_for(&patch);
        let mut output = [0.0_f32; 6 * 259];
        without_heap_activity(|| callback.render(&mut output, 6));
        for frame in output.as_chunks::<6>().0.iter().skip(2) {
            assert_eq!(*frame, [0.05, -0.025, 0.0, 0.0, 0.0, 0.0]);
        }
        let mut mono = [0.0_f32; 2];
        callback.render(&mut mono, 1);
        assert_eq!(mono, [0.0125; 2]);
    }

    #[test]
    fn timestamp_mapping_reanchors_and_caps_future_packets() {
        let mut mapper = TimestampMapper::default();
        assert_eq!(mapper.map(50_000, 100, 48000, 1), 100);
        assert_eq!(mapper.map(51_000, 110, 48000, 1), 148);
        assert_eq!(mapper.map(10_000_000, 120, 48000, 1), 4920);
        // Reversal reanchors timestamps but must not overtake an older queued packet.
        assert_eq!(mapper.map(1, 130, 48000, 1), 4920);
        assert_eq!(mapper.map(50_000, 140, 48000, 2), 140);
    }

    #[test]
    fn event_sample_order_and_callback_partitions_are_equivalent_without_heap() {
        fn run(partitions: &[usize]) -> [f32; 23] {
            let (mut callback, _, _) = callback_for(&midi_patch());
            let mut hardware = callback.midi.take_hardware().unwrap();
            let mut ui = callback.midi.take_ui().unwrap();
            assert!(hardware.send(3, note_on(60)));
            assert!(ui.send(7, note_off(60)));
            assert!(ui.send(12, note_on(64)));
            assert!(hardware.send(19, note_off(64)));
            let mut output = [0.0_f32; 23];
            let mut offset = 0;
            without_heap_activity(|| {
                for count in partitions {
                    callback.render(&mut output[offset..offset + count], 1);
                    offset += count;
                }
            });
            output
        }
        let whole = run(&[23]);
        assert_eq!(whole, run(&[1, 2, 4, 1, 9, 6]));
        for (frame, sample) in whole.iter().enumerate() {
            assert_eq!(
                *sample,
                if (3..7).contains(&frame) || (12..19).contains(&frame) {
                    1.0
                } else {
                    0.0
                }
            );
        }
    }

    #[test]
    fn panic_overflow_and_disconnect_epoch_invalidate_both_queues_and_pending_heads() {
        let (mut callback, _, _) = callback_for(&midi_patch());
        let mut hardware = callback.midi.take_hardware().unwrap();
        let mut ui = callback.midi.take_ui().unwrap();
        assert!(hardware.send(0, note_on(60)));
        callback.render(&mut [0.0_f32; 1], 1);
        assert!(ui.send(10, note_on(61)));
        assert_eq!(callback.midi.next_frame(), Some(10)); // Pending head must also be discarded.
        assert!(hardware.send(11, note_on(62)));
        hardware.panic(); // Same reset mechanism used by disconnect/switch.
        let mut output = [99.0_f32; 20];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [0.0; 20]);
        for _ in 0..MIDI_QUEUE_SIZE {
            assert!(hardware.send(100, note_on(63)));
        }
        let before = callback.shared.midi_epoch.load(Ordering::SeqCst);
        assert!(!hardware.send(100, note_off(63)));
        assert!(callback.shared.midi_epoch.load(Ordering::SeqCst) > before);
        assert_eq!(callback.shared.midi_overflows.load(Ordering::Relaxed), 1);
        assert!(ui.send(21, note_on(65))); // New-generation notes remain playable.
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [1.0; 20]);
        ui.panic();
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [0.0; 20]);
    }

    #[test]
    fn held_notes_survive_graph_edits_and_release_in_both_crossfade_branches() {
        let patch = midi_patch();
        let (mut callback, mut updates, _disposal) = callback_for(&patch);
        let mut ui = callback.midi.take_ui().unwrap();
        assert!(ui.send(0, note_on(60)));
        callback.render(&mut [0.0_f32; 4], 1);
        assert!(updates.push(patch.compile().unwrap()).is_ok());
        assert!(ui.send(8, note_off(60)));
        let mut output = [0.0_f32; 16];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output[..4], [1.0; 4]);
        assert_eq!(output[4..], [0.0; 12]);
        assert!(callback.graphs.previous.is_some());
    }

    #[test]
    fn pause_and_resume_reset_held_notes_and_paused_input_is_discarded() {
        let (mut callback, _, _) = callback_for(&midi_patch());
        let mut ui = callback.midi.take_ui().unwrap();
        assert!(ui.send(0, note_on(60)));
        callback.render(&mut [0.0_f32; 4], 1);
        callback.shared.playing.store(false, Ordering::SeqCst);
        ui.panic();
        assert!(!ui.send(4, note_on(64)));
        callback.render(&mut [0.0_f32; 4], 1);
        callback.shared.playing.store(true, Ordering::SeqCst);
        ui.panic();
        let mut output = [1.0_f32; 8];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [0.0; 8]);
    }

    #[test]
    fn hardware_parser_rejects_malformed_messages_without_queueing() {
        let shared = Arc::new(Shared::new());
        shared.playing.store(true, Ordering::SeqCst);
        let mut queues = MidiQueues::new(shared);
        let mut hardware = HardwareMidiSender {
            sender: queues.take_hardware().unwrap(),
            timestamp: TimestampMapper::default(),
            rate: 48000,
        };
        for bytes in [
            &[][..],
            &[0x90, 60],
            &[0x90, 128, 40],
            &[0x90, 60, 40, 0],
            &[0xf0, 0xf7],
        ] {
            assert!(!hardware.send_bytes(0, bytes));
        }
        assert_eq!(queues.pop_due(0), None);
        assert!(hardware.send_bytes(0, &[0x90, 60, 0]));
        assert_eq!(queues.pop_due(0), Some(note_off(60)));
    }
    #[test]
    fn panic_between_timestamp_mapping_and_enqueue_never_retags_old_note() {
        let shared = Arc::new(Shared::new());
        shared.playing.store(true, Ordering::SeqCst);
        let mut queues = MidiQueues::new(shared.clone());
        let mut sender = queues.take_hardware().unwrap();
        let captured_epoch = shared.midi_epoch.load(Ordering::SeqCst);
        let frame = TimestampMapper::default().map(1000, 0, 48000, captured_epoch);
        sender.panic(); // Inject reset between producer capture/map and push.
        assert!(sender.send_in_epoch(frame, note_on(60), captured_epoch));
        assert!(queues.sync_epoch());
        assert_eq!(queues.pop_due(0), None);
    }

    #[test]
    fn newly_published_epoch_is_preserved_when_callback_still_has_old_epoch() {
        let shared = Arc::new(Shared::new());
        shared.playing.store(true, Ordering::SeqCst);
        let mut queues = MidiQueues::new(shared);
        let mut sender = queues.take_ui().unwrap();
        sender.panic();
        assert!(sender.send(0, note_on(60)));
        assert_eq!(queues.pop_due(0), None); // Old callback epoch cannot consume it.
        assert!(queues.ui.head.is_some());
        assert!(queues.sync_epoch());
        assert_eq!(queues.pop_due(0), Some(note_on(60)));
    }
    #[test]
    fn due_event_flood_resets_once_and_keeps_rendering_with_fixed_callback_budget() {
        let (mut callback, _, _) = callback_for(&midi_patch());
        let mut ui = callback.midi.take_ui().unwrap();
        let mut hardware = callback.midi.take_hardware().unwrap();
        for _ in 0..MIDI_QUEUE_SIZE {
            assert!(ui.send(0, note_on(60)));
            assert!(hardware.send(0, note_on(61)));
        }
        let mut output = [1.0_f32; 1025];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [0.0; 1025]);
        assert_eq!(callback.frame_clock, 1025);
        assert_eq!(callback.shared.midi_overflows.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn mono_to_stereo_edit_crossfades_each_graphs_device_layout() {
        let mono: Patch = serde_json::from_value(serde_json::json!({
            "version":2,"sample_rate":48000,"nodes":[{"id":"dc","kind":"constant","params":{"value":0.5}}],
            "cables":[],"output":"dc"
        })).unwrap();
        let stereo: Patch = serde_json::from_value(serde_json::json!({
            "version":2,"sample_rate":48000,"nodes":[{"id":"dc","kind":"constant","params":{"value":0.5}},
            {"id":"out","kind":"audio_stereo","params":{}}],
            "cables":[{"from":"dc","to":"out","input":"left"}],"output":"out"
        })).unwrap();
        let (mut callback, mut updates, _disposal) = callback_for(&mono);
        callback.render(&mut [0.0_f32; 6], 6);
        assert!(updates.push(stereo.compile().unwrap()).is_ok());
        let mut output = [0.0_f32; 6 * 513];
        without_heap_activity(|| callback.render(&mut output, 6));
        assert_eq!(output[..6], [0.5; 6]);
        assert_eq!(output[output.len() - 6..], [0.05, 0.05, 0.0, 0.0, 0.0, 0.0]);
        for frames in output.as_chunks::<6>().0.windows(2) {
            for (previous, current) in frames[0].iter().zip(&frames[1]) {
                assert!((previous - current).abs() < 0.003);
            }
        }
    }

    #[test]
    fn simultaneous_hardware_then_ui_events_have_stable_tie_order() {
        let (mut callback, _, _) = callback_for(&midi_patch());
        let mut ui = callback.midi.take_ui().unwrap();
        let mut hardware = callback.midi.take_hardware().unwrap();
        assert!(ui.send(0, note_off(60)));
        assert!(hardware.send(0, note_on(60)));
        let mut output = [99.0_f32; 1];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [0.0]); // Hardware on precedes UI off, independently of enqueue order.
    }
    #[test]
    fn stale_keyboard_observation_cannot_retag_a_note_after_hardware_panic() {
        let (mut callback, _, _) = callback_for(&midi_patch());
        let mut ui = callback.midi.take_ui().unwrap();
        let hardware = callback.midi.take_hardware().unwrap();
        let keyboard_epoch = callback.shared.midi_epoch.load(Ordering::SeqCst);
        // The UI has synchronized its tracking, then hardware resets before the
        // UI submits its newly tracked key. The submission must remain stale.
        hardware.panic();
        assert!(!ui.send_observed(0, note_on(60), keyboard_epoch));
        let mut output = [99.0_f32; 8];
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [0.0; 8]);
        let refreshed_epoch = callback.shared.midi_epoch.load(Ordering::SeqCst);
        assert!(ui.send_observed(8, note_on(60), refreshed_epoch));
        without_heap_activity(|| callback.render(&mut output, 1));
        assert_eq!(output, [1.0; 8]);
    }
}
