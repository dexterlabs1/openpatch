//! Native output with control-thread compilation and bounded ownership transfer.
//! The running callback never allocates, locks, formats errors, or drops engines.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use openpatch_core::{Engine, Patch, BLOCK_SIZE};
use rtrb::{Consumer, Producer, PushError, RingBuffer};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;

pub const WAVEFORM_SIZE: usize = 256;
const QUEUE_SIZE: usize = 2;
const DEFAULT_VOLUME: f32 = 0.65;

struct Shared {
    playing: AtomicBool,
    volume: AtomicU32,
    peak: AtomicU32,
    stream_error: AtomicBool,
    waveform: [AtomicU32; WAVEFORM_SIZE],
    waveform_cursor: AtomicUsize,
}

impl Shared {
    fn new() -> Self {
        Self {
            playing: AtomicBool::new(false),
            volume: AtomicU32::new(DEFAULT_VOLUME.to_bits()),
            peak: AtomicU32::new(0),
            stream_error: AtomicBool::new(false),
            waveform: std::array::from_fn(|_| AtomicU32::new(0)),
            waveform_cursor: AtomicUsize::new(0),
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
}

impl Audio {
    pub fn new(patch: &Patch) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or("No output device found. Connect an audio device and retry.")?;
        let device_name = device.name().unwrap_or_else(|_| "Default output".into());
        let supported = choose_config(&device, patch.sample_rate)?;
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
        let callback = Callback::new(engine, updates, recycle, shared.clone(), sample_rate);
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
        self.shared.playing.store(playing, Ordering::Relaxed);
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
) -> Result<cpal::SupportedStreamConfig, String> {
    if let Ok(config) = device.default_output_config() {
        if supported_format(config.sample_format())
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
        let distance = rate.abs_diff(preferred_rate);
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
}

impl Callback {
    fn new(
        engine: Engine,
        incoming: Consumer<Engine>,
        retired: Producer<Engine>,
        shared: Arc<Shared>,
        rate: u32,
    ) -> Self {
        Self {
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
            self.fade_position = 0;
        }
        let target = if self.shared.playing.load(Ordering::Relaxed) {
            f32::from_bits(self.shared.volume.load(Ordering::Relaxed))
        } else {
            0.0
        };
        let mut peak = 0.0_f32;
        // Fixed stack buffers handle any device callback length and channel count.
        let mut current = [0.0_f32; BLOCK_SIZE];
        let mut previous = [0.0_f32; BLOCK_SIZE];
        for chunk in output.chunks_mut(BLOCK_SIZE * channels) {
            let frames = chunk.len() / channels;
            self.graphs.active.process_block(&mut current[..frames]);
            if let Some(engine) = self.graphs.previous.as_mut() {
                engine.process_block(&mut previous[..frames]);
            }
            for (index, frame) in chunk.chunks_mut(channels).enumerate() {
                if index >= frames {
                    frame.fill(T::from_mono(0.0));
                    continue;
                }
                let mono = if self.graphs.previous.is_some() {
                    let mix = (self.fade_position as f32 / self.fade_length as f32).min(1.0);
                    self.fade_position += 1;
                    previous[index] * (1.0 - mix) + current[index] * mix
                } else {
                    current[index]
                };
                let mono = finite_sample(mono * self.gain.next(target));
                peak = peak.max(mono.abs());
                frame.fill(T::from_mono(mono));
                // ~21ms scope window at 48kHz, decimated independently of frames.
                if self.scope_decimation == 0 {
                    self.shared.waveform[self.scope_cursor]
                        .store(mono.to_bits(), Ordering::Relaxed);
                    self.scope_cursor = (self.scope_cursor + 1) % WAVEFORM_SIZE;
                }
                self.scope_decimation = (self.scope_decimation + 1) % 4;
            }
            if self.graphs.previous.is_some() && self.fade_position >= self.fade_length {
                self.graphs.finish_transition();
            }
        }
        self.shared
            .waveform_cursor
            .store(self.scope_cursor, Ordering::Release);
        let previous_peak = f32::from_bits(self.shared.peak.load(Ordering::Relaxed));
        self.shared
            .peak
            .store(peak.max(previous_peak * 0.92).to_bits(), Ordering::Relaxed);
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
}
