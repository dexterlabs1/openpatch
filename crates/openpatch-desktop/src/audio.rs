//! Native output with control-thread compilation and bounded ownership transfer.
//! The running callback never allocates, locks, formats errors, or drops engines.

use crate::port_light_transport::{self, PortLightProducer, PortLightReader};
pub(crate) use crate::port_light_transport::{offline_frame, PortLightFrame, PortLightKey};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use openpatch::control::LiveModuleEvents;
pub use openpatch_core::ModuleTelemetry as SequenceTelemetry;
use openpatch_core::{
    Engine, MidiEvent, Patch, PreparedWaveBanks, ResolvedTap, TapSample, BLOCK_SIZE,
};
#[path = "audio_diagnostics.rs"]
mod audio_diagnostics;
pub use audio_diagnostics::Snapshot as AudioTimingSnapshot;
#[path = "performance.rs"]
mod performance;
pub(crate) use performance::ManualReleaseTicket;
use performance::{EventReader, ModulePlan, Performance, EVENT_CAPACITY};
#[path = "scope_stream.rs"]
mod scope_stream;
use rtrb::{Consumer, Producer, PushError, RingBuffer};
#[cfg(test)]
use scope_stream::SCOPE_PACKET_SIZE;
use scope_stream::{ScopeCapture, SCOPE_QUEUE_SIZE};
pub use scope_stream::{ScopeEndpoint, ScopePacket, ScopeSelection};
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
include!("audio_host_lifecycle.rs");

#[cfg(test)]
use openpatch_core::WavePreparationCache;

pub const WAVEFORM_SIZE: usize = 256;
const QUEUE_SIZE: usize = 2;
const MIDI_QUEUE_SIZE: usize = 512;
const DEFAULT_VOLUME: f32 = 0.65;

struct Shared {
    playing: AtomicBool,
    // Low bit is enable intent; upper bits advance only on boolean changes.
    port_light_control: AtomicU64,
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
            port_light_control: AtomicU64::new(1),
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
    fn set_port_lights_enabled(&self, enabled: bool) {
        let _ = self
            .port_light_control
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |old| {
                if old & 1 == u64::from(enabled) {
                    None
                } else {
                    old.checked_add(2)
                        .map(|epoch| (epoch & !1) | u64::from(enabled))
                }
            });
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

struct ScopeBinding {
    selection: ScopeSelection,
    kinds: [Option<String>; 2],
}
impl ScopeBinding {
    fn new(patch: &Patch, selection: ScopeSelection) -> Result<Self, String> {
        let mut kinds = [None, None];
        for (index, endpoint) in [&selection.x, &selection.y].into_iter().enumerate() {
            if let Some(endpoint) = endpoint {
                kinds[index] = Some(
                    patch
                        .nodes
                        .iter()
                        .find(|node| node.id == endpoint.id)
                        .ok_or_else(|| format!("Scope source '{}' does not exist", endpoint.id))?
                        .kind
                        .clone(),
                );
            }
        }
        Ok(Self { selection, kinds })
    }
    fn resolve(
        &self,
        patch: &Patch,
        engine: &Engine,
        strict: bool,
    ) -> Result<[Option<ResolvedTap>; 2], String> {
        let mut taps = [None; 2];
        for (index, endpoint) in [&self.selection.x, &self.selection.y]
            .into_iter()
            .enumerate()
        {
            let Some(endpoint) = endpoint else { continue };
            let same_kind = patch.nodes.iter().any(|node| {
                node.id == endpoint.id && Some(&node.kind) == self.kinds[index].as_ref()
            });
            let tap = if same_kind {
                engine.resolve_tap(&endpoint.id, &endpoint.port, endpoint.channel)
            } else {
                Err(format!(
                    "Scope source '{}' was removed or replaced",
                    endpoint.id
                ))
            };
            match tap {
                Ok(tap) => taps[index] = Some(tap),
                Err(error) if strict => return Err(error),
                Err(_) => {}
            }
        }
        Ok(taps)
    }
}
#[derive(Clone, Copy, Default)]
struct ScopeSetup {
    generation: u64,
    enabled: bool,
    taps: [Option<ResolvedTap>; 2],
}
struct PreparedGraph {
    engine: Engine,
    scope: ScopeSetup,
    modules: ModulePlan,
    revision: u64,
    port_lights: Option<PortLightProducer>,
    port_light_reader: Option<PortLightReader>,
    port_light_control: u64,
    host_lifecycle: PreparedHostLifecycle,
}
impl From<Engine> for PreparedGraph {
    fn from(engine: Engine) -> Self {
        Self {
            engine,
            scope: ScopeSetup::default(),
            modules: ModulePlan::default(),
            revision: 0,
            port_lights: None,
            port_light_reader: None,
            port_light_control: 1,
            host_lifecycle: PreparedHostLifecycle::default(),
        }
    }
}
impl PreparedGraph {
    /// Control-thread preparation; never resolve endpoints inside rendering.
    #[cfg(test)]
    fn prepare_port_lights(&mut self, patch: &Patch) -> Result<(), String> {
        self.prepare_port_lights_for_control(patch, 1)
    }
    fn prepare_port_lights_for_control(
        &mut self,
        patch: &Patch,
        control: u64,
    ) -> Result<(), String> {
        let (producer, reader) =
            port_light_transport::prepare_with_enabled(patch, &mut self.engine, control & 1 != 0)?;
        self.port_light_control = control;
        self.port_lights = Some(producer);
        self.port_light_reader = Some(reader);
        Ok(())
    }
    /// A missed OFF→ON transition must clear old detector history even when the
    /// latest enable bit matches. Toggles never affect graph/DSP state or clocks.
    fn sync_port_light_control(&mut self, control: u64, force_reset: bool) {
        if force_reset || self.port_light_control != control {
            self.engine.set_port_lights_enabled(false);
            let enabled = control & 1 != 0
                && self
                    .port_lights
                    .as_ref()
                    .is_some_and(PortLightProducer::has_endpoints);
            self.engine.set_port_lights_enabled(enabled);
            self.port_light_control = control;
        }
    }
}
impl Deref for PreparedGraph {
    type Target = Engine;
    fn deref(&self) -> &Engine {
        &self.engine
    }
}
impl DerefMut for PreparedGraph {
    fn deref_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }
}
#[cfg(test)]
fn prepare_graph(
    patch: &Patch,
    rate: u32,
    binding: Option<&ScopeBinding>,
    generation: u64,
    strict: bool,
) -> Result<PreparedGraph, String> {
    let banks = patch.prepare_wave_banks(&WavePreparationCache::default())?;
    prepare_graph_for_control(patch, &banks, rate, binding, generation, strict, 1)
}

fn prepare_graph_for_control(
    patch: &Patch,
    banks: &PreparedWaveBanks,
    rate: u32,
    binding: Option<&ScopeBinding>,
    generation: u64,
    strict: bool,
    port_light_control: u64,
) -> Result<PreparedGraph, String> {
    let mut device_patch = patch.clone();
    device_patch.sample_rate = rate;
    let engine = device_patch.compile_with_wave_banks(banks)?;
    let taps = binding
        .map(|binding| binding.resolve(&device_patch, &engine, strict))
        .transpose()?
        .unwrap_or([None; 2]);
    let mut graph = PreparedGraph {
        engine,
        modules: ModulePlan::default(),
        revision: 0,
        scope: ScopeSetup {
            generation,
            enabled: binding.is_some(),
            taps,
        },
        port_lights: None,
        port_light_reader: None,
        port_light_control,
        host_lifecycle: PreparedHostLifecycle::default(),
    };
    graph.prepare_port_lights_for_control(&device_patch, port_light_control)?;
    Ok(graph)
}

/// The stream keeps running during transport pause so gain can fade to silence.
/// Call `service` on each UI update to dispose of retired graphs and publish edits.
pub struct Audio {
    // Drop the stream before its control-side queue endpoints. CPAL shuts down the
    // callback before returning from Stream::drop; graph teardown is not rendering.
    _stream: Option<cpal::Stream>,
    incoming: Producer<PreparedGraph>,
    retired: Consumer<PreparedGraph>,
    pending: Option<PreparedGraph>,
    shared: Arc<Shared>,
    pub sample_rate: u32,
    pub device_name: String,
    pub device_channels: u16,
    midi_ui: MidiSender,
    midi_hardware: Option<HardwareMidiSender>,
    scope_binding: Option<ScopeBinding>,
    scope_generation: u64,
    scope_packets: Consumer<ScopePacket>,
    performance: Arc<Performance>,
    submitted_revision: u64,
    // Exact control-side graph paired with submitted_revision and banks. Scope
    // rebuilds must not substitute a newer UI patch for a sealed Session ticket.
    accepted_patch: Patch,
    prepared_wave_banks: Arc<PreparedWaveBanks>,
    port_light_reader: PortLightReader,
}

impl Audio {
    pub fn output_devices() -> Result<Vec<String>, String> {
        let host = cpal::default_host();
        let devices = host.output_devices().map_err(|e| e.to_string())?;
        Ok(devices.filter_map(|d| d.name().ok()).collect())
    }

    /// Uses a Session-prepared ticket; device-rate compilation never prepares banks.
    pub fn new_with_device_and_wave_banks(
        patch: &Patch,
        banks: Arc<PreparedWaveBanks>,
        selected: Option<&str>,
        port_lights_enabled: bool,
    ) -> Result<Self, String> {
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
        let engine = device_patch.compile_with_wave_banks(&banks)?;
        let (incoming, updates) = RingBuffer::new(QUEUE_SIZE);
        let (recycle, retired) = RingBuffer::new(QUEUE_SIZE);
        let shared = Arc::new(Shared::new());
        shared.set_port_lights_enabled(port_lights_enabled);
        let mut callback = Callback::new(engine, updates, recycle, shared.clone(), sample_rate);
        callback.graphs.active.prepare_port_lights_for_control(
            &device_patch,
            shared.port_light_control.load(Ordering::SeqCst),
        )?;
        let port_light_reader = callback
            .graphs
            .active
            .port_light_reader
            .as_ref()
            .unwrap()
            .clone();
        callback.performance.sync_patch(patch, 0);
        callback.graphs.active.modules =
            callback
                .performance
                .plan(patch, &callback.graphs.active.engine, 0)?;
        let performance = callback.performance.clone();
        let scope_packets = callback.scope_packets.take().unwrap();
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
            _stream: Some(stream),
            incoming,
            retired,
            pending: None,
            shared,
            sample_rate,
            device_name,
            device_channels: config.channels,
            midi_ui,
            midi_hardware,
            scope_binding: None,
            scope_generation: 0,
            scope_packets,
            performance,
            submitted_revision: 0,
            accepted_patch: patch.clone(),
            prepared_wave_banks: banks,
            port_light_reader,
        })
    }

    /// Compilation and superseded-edit disposal happen here, on the UI thread.
    /// If the callback is busy, the latest successfully compiled edit is retained.
    #[cfg(test)]
    pub fn submit_patch_at_revision(&mut self, patch: &Patch, revision: u64) -> Result<(), String> {
        let banks = patch.prepare_wave_banks(&WavePreparationCache::default())?;
        self.submit_patch_with_host_lifecycle(
            patch,
            revision,
            banks,
            &HostLifecycleDelta::default(),
        )
    }

    pub fn submit_patch_with_host_lifecycle(
        &mut self,
        patch: &Patch,
        revision: u64,
        banks: Arc<PreparedWaveBanks>,
        delta: &HostLifecycleDelta,
    ) -> Result<(), String> {
        if revision < self.submitted_revision {
            return Err("Stale audio patch revision".into());
        }
        let generation = self
            .scope_generation
            .checked_add(1)
            .ok_or("Scope generation exhausted")?;
        let mut graph = prepare_graph_for_control(
            patch,
            &banks,
            self.sample_rate,
            self.scope_binding.as_ref(),
            generation,
            false,
            self.shared.port_light_control.load(Ordering::SeqCst),
        )?;
        graph.modules = self.performance.plan(patch, &graph.engine, revision)?;
        graph.revision = revision;
        let mut combined = self
            .pending
            .as_ref()
            .map(|pending| pending.host_lifecycle.named.clone())
            .unwrap_or_default();
        combined.append(delta);
        graph.host_lifecycle = PreparedHostLifecycle::prepare(&graph.engine, combined)?;
        self.port_light_reader = graph.port_light_reader.as_ref().unwrap().clone();
        self.pending = Some(graph);
        self.submitted_revision = revision;
        self.accepted_patch = patch.clone();
        self.prepared_wave_banks = banks;
        self.scope_generation = generation;
        self.service();
        Ok(())
    }

    pub fn module_event_sender(&self) -> Arc<dyn LiveModuleEvents> {
        self.performance.clone()
    }
    pub(super) fn press_gate_with_ticket(&self, id: &str) -> Result<ManualReleaseTicket, String> {
        self.performance.press_gate_with_ticket(id)
    }
    pub(super) fn press_manual_with_ticket(
        &self,
        id: &str,
        press: openpatch_core::ModuleAction,
        release: openpatch_core::ModuleAction,
    ) -> Result<ManualReleaseTicket, String> {
        self.performance
            .press_manual_with_ticket(id, press, release)
    }
    /// Actual DSP telemetry; step is ONE-based and length is the active count.
    /// Read-only sequencer or sequential-switch state from the current graph.
    /// Unavailable while a newer registry revision awaits audio publication.
    pub fn sequence_state(&self, id: &str) -> Option<SequenceTelemetry> {
        self.performance.sequence_state(id)
    }

    /// Actual durable flags from one coherent callback publication, not saved defaults.
    pub fn stock_quantizer_states(&self) -> std::collections::BTreeMap<String, u16> {
        self.performance.snapshot()["modules"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, module)| {
                module["kind"] == "stock_quantizer" && module["available"] == true
            })
            .filter_map(|(id, module)| {
                module["state"]["scale_mask"]
                    .as_u64()
                    .filter(|mask| *mask <= 4095)
                    .map(|mask| (id.clone(), mask as u16))
            })
            .collect()
    }

    pub fn stock_seq3_states(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
        self.performance.snapshot()["modules"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, module)| module["kind"] == "stock_seq3" && module["available"] == true)
            .map(|(id, module)| (id.clone(), module["state"].clone()))
            .collect()
    }

    pub fn random_values_states(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
        self.performance.snapshot()["modules"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, module)| module["kind"] == "random_values" && module["available"] == true)
            .map(|(id, module)| (id.clone(), module["state"].clone()))
            .collect()
    }

    pub fn shift_register_states(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
        self.performance.snapshot()["modules"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, module)| module["kind"] == "sh_asr" && module["available"] == true)
            .map(|(id, module)| (id.clone(), module["state"].clone()))
            .collect()
    }

    /// Coherent, actual callback state; queued Learn requests are not guessed.
    pub fn midi_cc_states(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
        self.performance.snapshot()["modules"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, module)| module["kind"] == "midi_cc" && module["available"] == true)
            .map(|(id, module)| (id.clone(), module["state"].clone()))
            .collect()
    }

    pub fn midi_gate_states(&self) -> std::collections::BTreeMap<String, serde_json::Value> {
        self.performance.snapshot()["modules"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, module)| module["kind"] == "midi_gate" && module["available"] == true)
            .map(|(id, module)| (id.clone(), module["state"].clone()))
            .collect()
    }

    pub fn module_control_state(&self, id: &str) -> Option<openpatch_core::ModuleControlState> {
        self.performance.module_control_state(id)
    }
    pub fn module_scope(&self, id: &str) -> Option<Arc<openpatch_core::ScopeSnapshot>> {
        self.performance.scope_frame(id).map(|frame| frame.snapshot)
    }
    /// Actual active-graph endpoint state; unavailable before acquisition, for a
    /// pending revision, or 250ms after the reader last observed a new packet.
    pub(crate) fn port_lights(&self, revision: u64) -> Option<Arc<PortLightFrame>> {
        let control = self.shared.port_light_control.load(Ordering::SeqCst);
        if revision != self.submitted_revision || control & 1 == 0 {
            return None;
        }
        let frame = self.port_light_reader.read_epoch(revision, control)?;
        (self.shared.port_light_control.load(Ordering::SeqCst) == control).then_some(frame)
    }

    /// View-only observation intent; no graph submission, revision or history edit.
    /// Repeated positive brightness changes call this with true and do not reset.
    pub fn set_port_lights_enabled(&self, enabled: bool) {
        self.shared.set_port_lights_enabled(enabled);
    }

    pub fn module_lights(&self, id: &str) -> Option<openpatch_core::ModuleLightState> {
        self.performance.module_lights(id)
    }

    /// Resolve two raw module taps on the UI thread. A failed selection leaves
    /// playback and requested generation unchanged. Closing disables capture.
    pub fn set_scope(
        &mut self,
        _ui_patch: &Patch,
        selection: Option<ScopeSelection>,
    ) -> Result<u64, String> {
        // The UI may already show a revision beyond a retained Session ticket.
        // Rebuild the last accepted graph, including its pending ordered edges;
        // a subsequent ticket alone may advance patch/revision/banks together.
        let patch = &self.accepted_patch;
        let binding = selection
            .map(|selection| ScopeBinding::new(patch, selection))
            .transpose()?;
        let generation = self
            .scope_generation
            .checked_add(1)
            .ok_or("Scope generation exhausted")?;
        let mut graph = prepare_graph_for_control(
            patch,
            &self.prepared_wave_banks,
            self.sample_rate,
            binding.as_ref(),
            generation,
            true,
            self.shared.port_light_control.load(Ordering::SeqCst),
        )?;
        graph.modules = self
            .performance
            .plan(patch, &graph.engine, self.submitted_revision)?;
        graph.revision = self.submitted_revision;
        let delta = self
            .pending
            .as_ref()
            .map(|pending| pending.host_lifecycle.named.clone())
            .unwrap_or_default();
        graph.host_lifecycle = PreparedHostLifecycle::prepare(&graph.engine, delta)?;
        self.port_light_reader = graph.port_light_reader.as_ref().unwrap().clone();
        self.pending = Some(graph);
        self.scope_binding = binding;
        self.scope_generation = generation;
        self.service();
        Ok(generation)
    }
    /// Requested graph/tap generation. Queued packets can still belong to an
    /// older graph; the UI must reject them and reset capture on frame gaps.
    pub fn scope_generation(&self) -> u64 {
        self.scope_generation
    }
    /// Bounded even if the audio producer continues while the UI drains.
    pub fn scope_packets(&mut self) -> impl Iterator<Item = ScopePacket> + '_ {
        (0..SCOPE_QUEUE_SIZE).map_while(|_| self.scope_packets.pop().ok())
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

    pub fn diagnostics(&self) -> AudioTimingSnapshot {
        self.performance.diagnostics.snapshot()
    }

    pub fn reset_diagnostics(&self) -> u64 {
        self.performance.diagnostics.request_reset()
    }

    pub fn solver_failures(&self) -> usize {
        self.shared.solver_failures.load(Ordering::Relaxed)
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        self.performance.close();
        // Joining the callback is control-thread teardown. Only then finalize
        // queued-but-unapplied counters; retained MCP senders remain closed.
        drop(self._stream.take());
        self.performance.finish_close();
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
    graphs: Exchange<PreparedGraph>,
    shared: Arc<Shared>,
    gain: Gain,
    fade_length: usize,
    fade_position: usize,
    // Raw active-engine output, before host gain, clipping or device mapping.
    last_active_raw: [f32; 2],
    last_active_stereo: bool,
    frozen_previous: Option<([f32; 2], bool)>,
    scope_cursor: usize,
    scope_decimation: usize,
    midi: MidiQueues,
    frame_clock: u64,
    scope: ScopeCapture,
    scope_packets: Option<Consumer<ScopePacket>>,
    performance: Arc<Performance>,
    module_events: EventReader,
    timing: audio_diagnostics::Recorder,
    // Stream-global cadence survives every graph edit and same-revision rebuild.
    next_port_publish_frame: u64,
    port_publish_interval: u64,
    port_light_control: u64,
}

impl Callback {
    fn new(
        engine: Engine,
        incoming: Consumer<PreparedGraph>,
        retired: Producer<PreparedGraph>,
        shared: Arc<Shared>,
        rate: u32,
    ) -> Self {
        let port_light_control = shared.port_light_control.load(Ordering::SeqCst);
        let midi = MidiQueues::new(shared.clone());
        let (scope, scope_packets) = ScopeCapture::new(rate);
        let (performance, module_events, timing) = Performance::new(rate);
        let last_active_stereo = engine.output_channels() == 2;
        Self {
            performance,
            module_events,
            timing,
            scope,
            scope_packets: Some(scope_packets),
            midi,
            frame_clock: 0,
            graphs: Exchange {
                active: engine.into(),
                previous: None,
                waiting_retirement: None,
                incoming,
                retired,
            },
            shared,
            gain: Gain::new(rate),
            fade_length: (rate as usize / 200).max(1),
            fade_position: 0,
            last_active_raw: [0.0; 2],
            last_active_stereo,
            frozen_previous: None,
            scope_cursor: 0,
            scope_decimation: 0,
            next_port_publish_frame: 0,
            port_publish_interval: u64::from(rate).div_ceil(30),
            port_light_control,
        }
    }

    fn render<T: OutputSample>(&mut self, output: &mut [T], channels: usize) {
        let started = std::time::Instant::now();
        let epoch = self.timing.begin();
        let (adopted, crossfade) = self.render_inner(output, channels);
        let elapsed = started.elapsed().as_nanos().min(u64::MAX as u128) as u64;
        // Only diagnostics accumulation/publication is outside this measurement.
        self.timing.record(
            epoch,
            elapsed,
            output.len().checked_div(channels).unwrap_or(0),
            channels,
            self.graphs.active.revision,
            adopted,
            crossfade,
        );
    }

    fn render_inner<T: OutputSample>(&mut self, output: &mut [T], channels: usize) -> (bool, bool) {
        let control = self.shared.port_light_control.load(Ordering::SeqCst);
        let control_changed = self.port_light_control != control;
        let adopted = self.graphs.poll();
        // Align a queued graph before adoption; steady enabled adoption may
        // inherit decay, while a newly acknowledged epoch is reset below.
        self.graphs
            .active
            .sync_port_light_control(control, control_changed);
        if adopted {
            let mut moved = false;
            if let Some(previous) = self.graphs.previous.as_mut() {
                let active = &mut self.graphs.active;
                moved = active.modules.handoff_state_with_host_exclusions(
                    &mut active.engine,
                    &previous.modules,
                    &mut previous.engine,
                    &active.host_lifecycle.resets,
                );
                for (slot, binding) in active.modules.bindings.iter_mut().enumerate() {
                    if let Some(binding) = binding {
                        let same_instance = previous.modules.bindings[slot]
                            .is_some_and(|old| old.token == binding.token);
                        if same_instance {
                            binding.release_epoch =
                                previous.modules.bindings[slot].unwrap().release_epoch;
                        } else {
                            active.engine.reset_module_state(binding.handle);
                        }
                    }
                }
            }
            self.frozen_previous = moved.then_some((self.last_active_raw, self.last_active_stereo));
            // A donor with transferred Delay state is frozen; ordinary donors
            // still render the fade. Only the active graph runs observers.
            if let Some(previous) = self.graphs.previous.as_mut() {
                previous.engine.set_port_lights_enabled(false);
            }
            let active = &mut self.graphs.active;
            active.host_lifecycle.apply(&mut active.engine);
            active.modules.initialize_pending(
                &mut active.engine,
                self.graphs.previous.as_ref().map(|graph| &graph.modules),
            );
            active.modules.load_saved_pending(
                &mut active.engine,
                self.graphs
                    .previous
                    .as_mut()
                    .map(|graph| (&mut graph.modules, &mut graph.engine)),
            );
            self.fade_position = 0;
            self.scope.reset(self.graphs.active.scope.generation);
        }
        if adopted {
            // Adoption may copy an old epoch's observer state. Its DSP history
            // remains valid; only an unseen transition resets the detector.
            self.graphs
                .active
                .sync_port_light_control(control, control_changed);
            let active = &mut self.graphs.active;
            let enabled = control & 1 != 0
                && active
                    .port_lights
                    .as_ref()
                    .is_some_and(PortLightProducer::has_endpoints);
            active.engine.set_port_lights_enabled(enabled);
        }
        self.port_light_control = control;
        let target = if self.shared.playing.load(Ordering::Relaxed) {
            f32::from_bits(self.shared.volume.load(Ordering::Relaxed))
        } else {
            0.0
        };
        let mut peak = 0.0_f32;
        let mut module_budget = EVENT_CAPACITY;
        let mut midi_budget = MIDI_QUEUE_SIZE * 2;
        self.midi.refill_budget = MIDI_QUEUE_SIZE * 4;
        let mut midi_flooded = false;
        // Fixed stack buffers handle any device callback length and channel count.
        let mut current = [[0.0_f32; 2]; BLOCK_SIZE];
        let mut previous = [[0.0_f32; 2]; BLOCK_SIZE];
        let mut taps = [[TapSample::default(); 2]; BLOCK_SIZE];
        if channels == 0 {
            return (adopted, false);
        }
        let crossfade = self.graphs.previous.is_some() && output.len() >= channels;
        for chunk in output.chunks_mut(BLOCK_SIZE * channels) {
            let frames = chunk.len() / channels;
            let mut offset = 0;
            while offset < frames {
                let one_frame = {
                    let Exchange {
                        active, previous, ..
                    } = &mut self.graphs;
                    self.module_events.apply(
                        (&mut active.engine, &mut active.modules, active.revision),
                        previous
                            .as_mut()
                            .map(|old| (&mut old.engine, &mut old.modules)),
                        self.frame_clock,
                        &mut module_budget,
                    )
                };
                self.sync_midi_reset();
                self.module_events.sync_midi_learn(
                    &mut self.graphs.active.engine,
                    &mut self.graphs.active.modules,
                    self.graphs.active.revision,
                );
                let playing = self.shared.playing.load(Ordering::SeqCst);
                // Each producer is FIFO; equal-time hardware precedes UI events.
                // Bounded draining also protects the callback from a busy producer.
                while !midi_flooded && midi_budget > 0 {
                    let Some(event) = self.midi.pop_due(self.frame_clock) else {
                        break;
                    };
                    midi_budget -= 1;
                    if playing {
                        self.module_events.before_midi_learn(
                            &mut self.graphs.active.engine,
                            &mut self.graphs.active.modules,
                            self.graphs.active.revision,
                        );
                        // Crossfade copies still receive notes/controllers, but
                        // may never consume a Learn ticket or publish history.
                        if let Some(old) = self.graphs.previous.as_mut() {
                            old.modules.cancel_all_midi_learn(&mut old.engine);
                        }
                        self.graphs.active.handle_midi(event);
                        self.module_events.after_midi_learn(
                            &mut self.graphs.active.engine,
                            &mut self.graphs.active.modules,
                            self.frame_clock,
                            event == MidiEvent::Panic,
                        );
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
                let count = if one_frame { 1 } else { count };
                let active = &mut self.graphs.active;
                if active.scope.enabled {
                    active.engine.process_audio_block_with_taps(
                        &mut current[..count],
                        &active.scope.taps,
                        &mut taps[..count],
                    );
                } else {
                    active.engine.process_audio_block(&mut current[..count]);
                }
                let active_stereo = self.graphs.active.output_channels() == 2;
                let previous_stereo = self.frozen_previous.map_or_else(
                    || {
                        self.graphs
                            .previous
                            .as_ref()
                            .is_some_and(|g| g.output_channels() == 2)
                    },
                    |(_, stereo)| stereo,
                );
                if let Some(engine) = self.graphs.previous.as_mut() {
                    if let Some((raw, _)) = self.frozen_previous {
                        // Its Delay histories now belong to the active engine.
                        // Never advance any part of the donor graph again.
                        previous[..count].fill(raw);
                    } else {
                        engine.process_audio_block(&mut previous[..count]);
                    }
                }
                for index in 0..count {
                    if self.graphs.active.scope.enabled {
                        self.scope
                            .sample(self.frame_clock + index as u64, taps[index]);
                    }
                    let old = self.graphs.previous.as_ref().map(|_| previous[index]);
                    let mix = if old.is_some() {
                        let mix = (self.fade_position as f32 / self.fade_length as f32).min(1.0);
                        self.fade_position += 1;
                        mix
                    } else {
                        1.0
                    };
                    let gain = self.gain.next(target);
                    self.last_active_raw = current[index];
                    self.last_active_stereo = active_stereo;
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
                    self.frozen_previous = None;
                }
            }
            // CPAL normally supplies complete frames; incomplete tails are silence.
            chunk[frames * channels..].fill(T::from_mono(0.0));
        }
        self.graphs
            .active
            .modules
            .publish_scope(&self.graphs.active.engine, self.frame_clock);
        self.module_events.publish(
            &self.graphs.active.engine,
            &self.graphs.active.modules,
            self.graphs.active.revision,
            self.frame_clock,
        );
        if control & 1 != 0
            && self.shared.port_light_control.load(Ordering::SeqCst) == control
            && self.frame_clock >= self.next_port_publish_frame
        {
            let active = &mut self.graphs.active;
            if active.port_lights.as_mut().is_some_and(|producer| {
                producer.publish_epoch(&active.engine, active.revision, self.frame_clock, control)
            }) {
                self.next_port_publish_frame =
                    self.frame_clock.saturating_add(self.port_publish_interval);
            }
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
        (adopted, crossfade)
    }
    fn sync_midi_reset(&mut self) {
        if self.midi.sync_epoch() {
            self.module_events.cancel_all_midi_learn(
                &mut self.graphs.active.engine,
                &mut self.graphs.active.modules,
            );
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
pub(crate) fn test_manual_release_ticket() -> (ManualReleaseTicket, impl Fn() -> serde_json::Value)
{
    let (performance, reader, _timing) = Performance::new(48_000);
    let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
        "nodes":[{"id":"envelope","kind":"adsr_eg"}]}))
    .unwrap();
    performance.sync_patch(&patch, 1);
    let ticket = performance.press_gate_with_ticket("envelope").unwrap();
    (ticket, move || {
        let _keep_reader_alive = &reader;
        performance.snapshot()
    })
}

#[cfg(test)]
pub(crate) fn test_stock_seq3_manual_release_ticket(
    press: &str,
    release: &str,
) -> (
    ManualReleaseTicket,
    impl FnMut(usize) -> serde_json::Value,
    impl Fn() -> Result<ManualReleaseTicket, String>,
) {
    let patch:Patch=serde_json::from_value(serde_json::json!({"version":2,"nodes":[{"id":"seq","kind":"stock_seq3"}],"output":"seq","output_port":"cv1"})).unwrap();
    let (mut callback, _updates, _disposal) = tests::sequence_callback(&patch);
    callback.shared.playing.store(false, Ordering::SeqCst);
    callback.render(&mut [0.0_f32; 64], 1);
    let press =
        serde_json::from_value::<openpatch_core::ModuleAction>(serde_json::json!(press)).unwrap();
    let release =
        serde_json::from_value::<openpatch_core::ModuleAction>(serde_json::json!(release)).unwrap();
    let performance = callback.performance.clone();
    let ticket = performance
        .press_manual_with_ticket("seq", press, release)
        .unwrap();
    (
        ticket,
        move |frames| {
            let mut output = vec![0.0_f32; frames];
            callback.render(&mut output, 1);
            callback.performance.snapshot()
        },
        move || performance.press_manual_with_ticket("seq", press, release),
    )
}

#[cfg(test)]
pub(crate) fn test_module_incarnation_tokens(
    session: &mut openpatch::control::Session,
) -> impl Fn(&Patch, u64) -> Vec<u64> {
    let (performance, reader, _timing) = Performance::new(48_000);
    session
        .set_live_module_events(Some(performance.clone()))
        .unwrap();
    move |patch, revision| {
        let _keep_reader_alive = &reader;
        let engine = patch.compile().unwrap();
        performance
            .plan(patch, &engine, revision)
            .unwrap()
            .states
            .iter()
            .flatten()
            .map(|binding| binding.token)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_and_scope_graphs_use_admitted_ticket_without_preparation() {
        let patch: Patch = serde_json::from_value(serde_json::json!({
            "version":2,"nodes":[{"id":"wt","kind":"wavetable_vco"}]
        }))
        .unwrap();
        let cache = WavePreparationCache::default();
        let banks = patch.prepare_wave_banks(&cache).unwrap();
        let admitted = cache.stats().admitted_bytes;
        let live_banks = cache.stats().live_banks;
        assert!(admitted > 0);
        for (generation, rate) in [(1, 44_100), (2, 48_000), (3, 96_000)] {
            let graph =
                prepare_graph_for_control(&patch, &banks, rate, None, generation, true, 1).unwrap();
            assert_eq!(graph.scope.generation, generation);
            assert_eq!(cache.stats().admitted_bytes, admitted);
            assert_eq!(cache.stats().live_banks, live_banks);
        }
        assert!(prepare_graph_for_control(
            &patch,
            &PreparedWaveBanks::default(),
            48_000,
            None,
            4,
            true,
            1,
        )
        .is_err());
        assert_eq!(cache.stats().admitted_bytes, admitted);
    }

    use openpatch_core::ModuleAction;
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
            content: None,
            score: None,
            bypassed: false,
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
        callback.graphs.active.prepare_port_lights(&patch).unwrap();
        let mut stereo = [0.0_f32; 2034];
        without_heap_activity(|| callback.render(&mut stereo, 2));
        for frame in stereo.as_chunks::<2>().0 {
            assert_eq!(frame[0], frame[1]);
        }
        callback.gain.value = 1.0;
        patch.nodes[0].params.insert("value".into(), -1.0);
        assert!(updates.push(patch.compile().unwrap().into()).is_ok());
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
            callback
                .graphs
                .active
                .prepare_port_lights(&original)
                .unwrap();
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
            assert!(updates.push(edited.compile().unwrap().into()).is_ok());
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
    fn callback_for(patch: &Patch) -> (Callback, Producer<PreparedGraph>, Consumer<PreparedGraph>) {
        let (updates, incoming) = RingBuffer::new(2);
        let (retired, disposal) = RingBuffer::new(2);
        let shared = Arc::new(Shared::new());
        shared.playing.store(true, Ordering::SeqCst);
        shared.volume.store(1.0_f32.to_bits(), Ordering::Relaxed);
        let mut callback =
            Callback::new(patch.compile().unwrap(), incoming, retired, shared, 48000);
        callback.graphs.active.prepare_port_lights(patch).unwrap();
        callback.gain.value = 1.0;
        (callback, updates, disposal)
    }
    #[test]
    fn timing_covers_callback_adoption_conversion_reset_and_close_without_heap() {
        let patch = sequence_patch();
        let (mut measured, mut updates, _retired) = sequence_callback(&patch);
        let (mut reference, mut raw_updates, _raw_retired) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(measured.performance.clone()))
            .unwrap();
        assert!(!measured.performance.diagnostics.snapshot().available);
        let mut actual = [0_i16; 513];
        let mut expected = actual;
        without_heap_activity(|| measured.render(&mut actual, 2));
        reference.render_inner(&mut expected, 2);
        assert_eq!(actual, expected);
        assert_eq!(actual[512], 0);
        let first = measured.performance.diagnostics.snapshot();
        assert!(first.available && first.coherent);
        assert_eq!(
            (first.frames, first.channels, first.sample_rate),
            (256, 2, 48_000)
        );
        assert_eq!((first.callbacks, first.callback_sequence), (1, 1));
        assert!(!first.adopted_graph && !first.crossfade);
        assert!(first.utilization.is_some());
        updates
            .push(sequence_graph(&measured, &patch, 2))
            .ok()
            .unwrap();
        raw_updates
            .push(sequence_graph(&reference, &patch, 2))
            .ok()
            .unwrap();
        without_heap_activity(|| measured.render(&mut actual, 2));
        reference.render_inner(&mut expected, 2);
        assert_eq!(actual, expected);
        let second = measured.performance.diagnostics.snapshot();
        assert!(second.adopted_graph && second.crossfade);
        assert_eq!((second.revision, second.transition_callbacks), (2, 1));
        let receipt = session
            .execute("inspect_performance", serde_json::json!({}))
            .unwrap();
        assert_eq!(
            receipt["performance"]["audio_timing"],
            serde_json::to_value(&second).unwrap()
        );
        let epoch = measured.performance.diagnostics.request_reset();
        assert!(measured.performance.diagnostics.snapshot().reset_pending);
        without_heap_activity(|| measured.render(&mut [0.0_f32; 2], 0));
        let empty = measured.performance.diagnostics.snapshot();
        assert_eq!(empty.epoch, epoch);
        assert!(!empty.reset_pending);
        assert_eq!(
            (
                empty.callbacks,
                empty.invalid_callbacks,
                empty.callback_sequence
            ),
            (0, 1, 3)
        );
        assert!(empty.utilization.is_none() && empty.peak_utilization.is_none());
        measured.performance.close();
        assert!(measured.performance.diagnostics.snapshot().closed);
        assert!(!measured.performance.diagnostics.snapshot().available);
    }

    #[test]
    fn timing_matches_uninstrumented_audio_across_formats_and_variable_buffers() {
        let patch = sequence_patch();
        let (mut measured, _, _retired) = sequence_callback(&patch);
        let (mut reference, _, _raw_retired) = sequence_callback(&patch);
        for samples in [0, 1, 17, 511, 512, 777] {
            let mut actual = vec![0.0_f32; samples];
            let mut expected = actual.clone();
            without_heap_activity(|| measured.render(&mut actual, 2));
            reference.render_inner(&mut expected, 2);
            assert_eq!(actual, expected);
            assert_eq!(
                measured.performance.diagnostics.snapshot().frames,
                samples / 2
            );
        }
        let mut actual = [0_u16; 57];
        let mut expected = actual;
        without_heap_activity(|| measured.render(&mut actual, 6));
        reference.render_inner(&mut expected, 6);
        assert_eq!(actual, expected);
        assert_eq!(&actual[54..], &[32768; 3]);
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
        assert!(updates.push(patch.compile().unwrap().into()).is_ok());
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
        assert!(updates.push(stereo.compile().unwrap().into()).is_ok());
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
    fn scope_binding(patch: &Patch, channel: usize) -> ScopeBinding {
        ScopeBinding::new(
            patch,
            ScopeSelection {
                x: Some(ScopeEndpoint {
                    id: "dc".into(),
                    port: "out".into(),
                    channel,
                }),
                y: None,
            },
        )
        .unwrap()
    }
    fn scope_patch(value: f32) -> Patch {
        serde_json::from_value(serde_json::json!({"version":2,"nodes":[{"id":"dc","kind":"constant","params":{"value":value}}],"cables":[],"output":"dc"})).unwrap()
    }

    #[test]
    fn scope_packets_are_raw_timestamped_and_partition_independent_without_callback_heap() {
        fn capture(partitions: &[usize], version: u32) -> Vec<ScopePacket> {
            let mut patch = scope_patch(3.0);
            patch.version = version;
            let (mut callback, mut updates, _disposal) = callback_for(&patch);
            let mut packets = callback.scope_packets.take().unwrap();
            callback
                .shared
                .volume
                .store(0.0_f32.to_bits(), Ordering::Relaxed);
            callback.gain.value = 0.0;
            let binding = scope_binding(&patch, 0);
            assert!(updates
                .push(prepare_graph(&patch, 48000, Some(&binding), 7, true).unwrap())
                .is_ok());
            let mut output = [9.0_f32; 257];
            let mut offset = 0;
            without_heap_activity(|| {
                for count in partitions {
                    callback.render(&mut output[offset..offset + count], 1);
                    offset += count;
                }
            });
            assert_eq!(output, [0.0; 257]); // Master volume does not scale taps.
            let mut result = Vec::new();
            while let Ok(packet) = packets.pop() {
                result.push(packet);
            }
            assert_eq!(result.len(), 4);
            for (index, packet) in result.iter().enumerate() {
                assert_eq!(packet.start_frame, index as u64 * SCOPE_PACKET_SIZE as u64);
                assert_eq!(packet.generation, 7);
                assert_eq!(packet.sample_rate, 48000);
                assert_eq!(packet.x, [3.0; SCOPE_PACKET_SIZE]);
                assert_eq!(packet.x_active, [true; SCOPE_PACKET_SIZE]);
                assert_eq!(packet.y, [0.0; SCOPE_PACKET_SIZE]);
                assert_eq!(packet.y_active, [false; SCOPE_PACKET_SIZE]);
            }
            result
        }
        let whole = capture(&[257], 2);
        let partitioned = capture(&[1, 2, 64, 127, 63], 2);
        let legacy = capture(&[1, 2, 64, 127, 63], 1);
        assert_eq!(legacy[0].x, whole[0].x);
        for (a, b) in whole.iter().zip(partitioned) {
            assert_eq!(a.start_frame, b.start_frame);
            assert_eq!(a.x, b.x);
            assert_eq!(a.x_active, b.x_active);
        }
    }

    #[test]
    fn scope_generation_swap_discards_partial_packet_and_deletion_or_kind_reuse_stays_inactive() {
        let patch = scope_patch(1.0);
        let binding = scope_binding(&patch, 0);
        let (mut callback, mut updates, _disposal) = callback_for(&patch);
        let mut packets = callback.scope_packets.take().unwrap();
        assert!(updates
            .push(prepare_graph(&patch, 48000, Some(&binding), 1, true).unwrap())
            .is_ok());
        callback.render(&mut [0.0_f32; 321], 1); // 5 packets + 1 partial, transition finished.
        let edited = scope_patch(2.0);
        assert!(updates
            .push(prepare_graph(&edited, 48000, Some(&binding), 2, true).unwrap())
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 320], 1));
        let old: Vec<_> = (0..5).map(|_| packets.pop().unwrap()).collect();
        assert!(old
            .iter()
            .all(|p| p.generation == 1 && p.x == [1.0; SCOPE_PACKET_SIZE]));
        let first_new = packets.pop().unwrap();
        assert_eq!(first_new.generation, 2);
        assert_eq!(first_new.start_frame, 321);
        assert_eq!(first_new.x, [2.0; SCOPE_PACKET_SIZE]); // No old/new crossfade enters taps.
        let removed = Patch::new_v2();
        let graph = prepare_graph(&removed, 48000, Some(&binding), 3, false).unwrap();
        assert!(graph.scope.enabled && graph.scope.taps.iter().all(Option::is_none));
        assert!(prepare_graph(&removed, 48000, Some(&binding), 3, true).is_err());
        let mut replaced = patch.clone();
        replaced.nodes[0].kind = "output".into();
        replaced.nodes[0].params.clear();
        let graph = prepare_graph(&replaced, 48000, Some(&binding), 4, false).unwrap();
        assert!(
            graph.scope.taps.iter().all(Option::is_none),
            "same ID/out must not silently bind a different model"
        );
        let absent_voice = scope_binding(&patch, 1);
        let graph = prepare_graph(&patch, 48000, Some(&absent_voice), 5, true).unwrap();
        assert!(updates.push(graph).is_ok());
        // Drain old queued packets then verify a valid but absent mono lane.
        while packets.pop().is_ok() {}
        without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
        let packet = packets.pop().unwrap();
        assert_eq!(packet.generation, 5);
        assert_eq!(packet.x_active, [false; SCOPE_PACKET_SIZE]);
    }

    #[test]
    fn scope_queue_overflow_keeps_real_frame_gaps_and_disabled_capture_publishes_nothing() {
        let patch = scope_patch(1.0);
        let binding = scope_binding(&patch, 0);
        let (mut callback, mut updates, _disposal) = callback_for(&patch);
        let mut packets = callback.scope_packets.take().unwrap();
        assert!(updates
            .push(prepare_graph(&patch, 48000, Some(&binding), 1, true).unwrap())
            .is_ok());
        let mut output = [0.0_f32; SCOPE_PACKET_SIZE * (SCOPE_QUEUE_SIZE + 2)];
        without_heap_activity(|| callback.render(&mut output, 1));
        let mut last = 0;
        for _ in 0..SCOPE_QUEUE_SIZE {
            last = packets.pop().unwrap().start_frame;
        }
        assert!(packets.pop().is_err());
        without_heap_activity(|| callback.render(&mut output[..SCOPE_PACKET_SIZE], 1));
        let next = packets.pop().unwrap();
        assert_eq!(next.start_frame - last, 3 * SCOPE_PACKET_SIZE as u64);
        assert!(updates
            .push(prepare_graph(&patch, 48000, None, 2, true).unwrap())
            .is_ok());
        without_heap_activity(|| callback.render(&mut output[..SCOPE_PACKET_SIZE * 2], 1));
        assert!(packets.pop().is_err());
    }
    #[test]
    fn scope_packet_offsets_follow_midi_subspans_without_decimation_or_heap() {
        let patch = midi_patch();
        let binding = ScopeBinding::new(
            &patch,
            ScopeSelection {
                x: Some(ScopeEndpoint {
                    id: "midi".into(),
                    port: "gate".into(),
                    channel: 0,
                }),
                y: None,
            },
        )
        .unwrap();
        let (mut callback, mut updates, _disposal) = callback_for(&patch);
        let mut packets = callback.scope_packets.take().unwrap();
        let mut ui = callback.midi.take_ui().unwrap();
        assert!(ui.send(3, note_on(60)));
        assert!(ui.send(7, note_off(60)));
        assert!(updates
            .push(prepare_graph(&patch, 48000, Some(&binding), 11, true).unwrap())
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; SCOPE_PACKET_SIZE], 1));
        let packet = packets.pop().unwrap();
        assert_eq!(packet.start_frame, 0);
        assert_eq!(packet.generation, 11);
        assert_eq!(packet.x_active, [true; SCOPE_PACKET_SIZE]);
        for (frame, value) in packet.x.iter().enumerate() {
            assert_eq!(*value, if (3..7).contains(&frame) { 10.0 } else { 0.0 });
        }
    }
    fn sequence_patch() -> Patch {
        let mut patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"seq","kind":"seq3","params":{"tempo":15,"running":1,"steps":8,"row1_1":1,"row1_2":2,"row1_3":3}},
                {"id":"zero","kind":"constant","params":{"value":0}}],
            "cables":[{"from":"zero","to":"seq","input":"clock"}],"output":"seq","output_port":"cv1"})).unwrap();
        patch.assign_missing_cable_ids().unwrap();
        patch
    }
    pub(super) fn sequence_callback(
        patch: &Patch,
    ) -> (Callback, Producer<PreparedGraph>, Consumer<PreparedGraph>) {
        let (mut callback, updates, disposal) = callback_for(patch);
        callback.performance.sync_patch(patch, 1);
        callback.graphs.active.modules = callback
            .performance
            .plan(patch, &callback.graphs.active.engine, 1)
            .unwrap();
        callback.graphs.active.revision = 1;
        (callback, updates, disposal)
    }
    pub(super) fn sequence_graph(
        callback: &Callback,
        patch: &Patch,
        revision: u64,
    ) -> PreparedGraph {
        let mut graph = prepare_graph(patch, 48000, None, revision, true).unwrap();
        graph.modules = callback
            .performance
            .plan(patch, &graph.engine, revision)
            .unwrap();
        graph.revision = revision;
        graph
    }

    fn sequence_preset(params: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"format":"openpatch-module-preset","version":1,"kind":"seq3","params":params})
    }

    #[test]
    fn ordinary_sequence_knob_history_does_not_require_or_restore_live_run() {
        let patch = sequence_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        // Knob changes remain usable even before the first telemetry callback.
        session
            .execute(
                "set_parameter",
                serde_json::json!({"id":"seq","parameter":"row1_1","value":4}),
            )
            .unwrap();
        session
            .execute(
                "rotate_sequence",
                serde_json::json!({"id":"seq","direction":"right"}),
            )
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert!(!callback.performance.sequence_state("seq").unwrap().running);
        session.execute("undo", serde_json::json!({})).unwrap();
        session.execute("undo", serde_json::json!({})).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert!(!callback.performance.sequence_state("seq").unwrap().running);
    }

    #[test]
    fn preset_capture_reads_actual_run_with_coherent_frame_and_rejects_stale_audio() {
        for explicit_running in [false, true] {
            let mut patch = sequence_patch();
            if !explicit_running {
                patch.nodes[0].params.remove("running");
            }
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            let workspace = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
            session.replace_patch(patch, Some(0)).unwrap();
            session
                .set_live_module_events(Some(callback.performance.clone()))
                .unwrap();
            assert!(session
                .execute("capture_module_preset", serde_json::json!({"id":"seq"}))
                .is_err());
            callback
                .performance
                .trigger("seq", ModuleAction::Run)
                .unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 137], 1));
            let capture = session
                .execute(
                    "capture_module_preset",
                    serde_json::json!({"id":"seq","expected_revision":1}),
                )
                .unwrap();
            assert_eq!(capture["preset"]["params"]["running"], 0.0);
            assert_eq!(capture["frame"], 137);
            assert_eq!(capture["revision"], 1);
            let status = callback.performance.snapshot();
            assert_eq!(status["modules"]["seq"]["frame"], 137);
            // File save writes the reviewed immutable capture even if live Run changes later.
            callback
                .performance
                .trigger("seq", ModuleAction::Run)
                .unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            session
                .execute(
                    "save_module_preset",
                    serde_json::json!({"filename":"paused.opreset","preset":capture["preset"]}),
                )
                .unwrap();
            let saved = session
                .execute(
                    "read_module_preset",
                    serde_json::json!({"filename":"paused.opreset"}),
                )
                .unwrap();
            assert_eq!(saved["preset"]["params"]["running"], 0.0);
            session
                .execute(
                    "set_parameter",
                    serde_json::json!({"id":"seq","parameter":"row1_1","value":2}),
                )
                .unwrap();
            assert!(session
                .execute("capture_module_preset", serde_json::json!({"id":"seq"}))
                .is_err());
            assert!(updates
                .push(sequence_graph(
                    &callback,
                    session.patch(),
                    session.revision()
                ))
                .is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            let current = session
                .execute("capture_module_preset", serde_json::json!({"id":"seq"}))
                .unwrap();
            assert_eq!(current["preset"]["params"]["running"], 1.0);
            assert_eq!(current["frame"], 139);
        }
    }

    fn project_capture_fixture() -> Patch {
        serde_json::from_value(serde_json::json!({
            "version":2,
            "nodes":[
                {"id":"seq_a","kind":"seq3","params":{"tempo":15,"running":1,"steps":8}},
                {"id":"seq_b","kind":"seq3","params":{"tempo":15,"running":1,"steps":8}},
                {"id":"manual","kind":"logic"},
                {"id":"osc","kind":"oscillator","params":{"frequency":123}},
                {"id":"bias","kind":"constant","params":{"value":3}},
                {"id":"sum","kind":"gain"}
            ],
            "cables":[
                {"id":"osc_route","from":"osc","to":"sum","input":"signal"},
                {"id":"bias_route","from":"bias","to":"sum","input":"signal"}
            ],
            "output":"sum"
        }))
        .unwrap()
    }

    #[test]
    fn project_capture_reads_two_live_sequences_without_touching_dsp_state() {
        let patch = project_capture_fixture();
        let (mut callback, _updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();

        without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
        for (id, action) in [
            ("seq_a", ModuleAction::Clock),
            ("seq_b", ModuleAction::Clock),
            ("seq_b", ModuleAction::Run),
            ("manual", ModuleAction::BPress),
        ] {
            callback.performance.trigger(id, action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let a = callback.performance.sequence_state("seq_a").unwrap();
        let b = callback.performance.sequence_state("seq_b").unwrap();
        assert_eq!((a.step, a.running), (2, true));
        assert_eq!((b.step, b.running), (2, false));
        assert!(
            callback
                .performance
                .module_control_state("manual")
                .unwrap()
                .pressed
        );
        let before_session = session.snapshot();
        let before_live = callback.performance.snapshot();
        let before_frame = callback.frame_clock;
        let before_bindings = callback
            .graphs
            .active
            .modules
            .states
            .map(|b| b.map(|b| b.token));
        let before_routes = [
            callback.graphs.active.modules.cables[0].unwrap().token,
            callback.graphs.active.modules.cables[1].unwrap().token,
        ];
        let mut reference = patch.compile().unwrap();
        reference.adopt_state_from(&callback.graphs.active.engine);

        let captured = session.capture_project_patch(Some(1)).unwrap();
        assert_eq!(captured.state_source, "live");
        assert_eq!(captured.frame, Some(before_frame));
        assert!(captured.running["seq_a"]);
        assert!(!captured.running["seq_b"]);
        assert_eq!(captured.patch.nodes[0].params["running"], 1.0);
        assert_eq!(captured.patch.nodes[1].params["running"], 0.0);
        assert_eq!(session.patch().nodes[1].params["running"], 1.0);
        assert_eq!(session.snapshot(), before_session);
        assert_eq!(callback.performance.snapshot(), before_live);
        assert_eq!(callback.frame_clock, before_frame);
        assert_eq!(callback.performance.sequence_state("seq_a"), Some(a));
        assert_eq!(callback.performance.sequence_state("seq_b"), Some(b));
        assert!(
            callback
                .performance
                .module_control_state("manual")
                .unwrap()
                .pressed
        );
        assert_eq!(
            callback
                .graphs
                .active
                .modules
                .states
                .map(|b| b.map(|b| b.token)),
            before_bindings
        );
        assert_eq!(
            [
                callback.graphs.active.modules.cables[0].unwrap().token,
                callback.graphs.active.modules.cables[1].unwrap().token,
            ],
            before_routes
        );
        // Advancing identical engines once compares oscillator phase and both
        // cable-delay histories after the read-only project capture.
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("osc", "out"),
                ("sum", "out")
            ),
            read_raw_pair(&mut reference, ("osc", "out"), ("sum", "out"))
        );
    }

    #[test]
    fn layout_checkpoint_history_preserves_live_state_identity_and_first_delayed_samples() {
        let patch = project_capture_fixture();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
        for (id, action) in [
            ("seq_a", ModuleAction::Clock),
            ("seq_b", ModuleAction::Clock),
            ("seq_b", ModuleAction::Run),
            ("manual", ModuleAction::BPress),
        ] {
            callback.performance.trigger(id, action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let a = callback.performance.sequence_state("seq_a").unwrap();
        let b = callback.performance.sequence_state("seq_b").unwrap();
        assert_eq!((a.step, a.running), (2, true));
        assert_eq!((b.step, b.running), (2, false));
        assert!(
            callback
                .performance
                .module_control_state("manual")
                .unwrap()
                .pressed
        );
        assert_eq!(session.patch().nodes[1].params["running"], 1.0);
        let states = callback
            .graphs
            .active
            .modules
            .states
            .map(|binding| binding.map(|binding| binding.token));
        let bindings = callback
            .graphs
            .active
            .modules
            .bindings
            .map(|binding| binding.map(|binding| binding.token));
        let routes = [
            callback.graphs.active.modules.cables[0].unwrap().token,
            callback.graphs.active.modules.cables[1].unwrap().token,
        ];
        let original_state = session.state_id();
        let mut checkpoint_state = None;
        let original_patch = session.snapshot()["patch"].clone();
        let mut reference = patch.compile().unwrap();
        reference.adopt_state_from(&callback.graphs.active.engine);
        let mut reference_frames = [[0.0; 2]; 256];
        for operation in ["checkpoint", "undo", "redo", "coalesced_undo_redo"] {
            let expected_state = match operation {
                "checkpoint" => {
                    session
                        .record_layout_checkpoint(session.revision())
                        .unwrap();
                    checkpoint_state = Some(session.state_id());
                    assert_ne!(checkpoint_state, Some(original_state));
                    session.state_id()
                }
                "undo" => {
                    session.execute("undo", serde_json::json!({})).unwrap();
                    original_state
                }
                "redo" => {
                    session.execute("redo", serde_json::json!({})).unwrap();
                    checkpoint_state.unwrap()
                }
                "coalesced_undo_redo" => {
                    session.execute("undo", serde_json::json!({})).unwrap();
                    session.execute("redo", serde_json::json!({})).unwrap();
                    checkpoint_state.unwrap()
                }
                _ => unreachable!(),
            };
            assert_eq!(session.state_id(), expected_state, "{operation}");
            assert_eq!(session.snapshot()["patch"], original_patch, "{operation}");
            assert!(updates
                .push(sequence_graph(
                    &callback,
                    session.patch(),
                    session.revision()
                ))
                .is_ok());
            // Poll the real adoption path without processing audio yet. Reading
            // its very first sample catches lost one-frame cable histories,
            // which would have refilled before the end of a normal callback.
            without_heap_activity(|| callback.render(&mut [0.0_f32; 0], 1));
            assert_eq!(callback.graphs.active.revision, session.revision());
            assert_eq!(
                callback
                    .graphs
                    .active
                    .modules
                    .states
                    .map(|binding| binding.map(|binding| binding.token)),
                states,
                "{operation}"
            );
            assert_eq!(
                callback
                    .graphs
                    .active
                    .modules
                    .bindings
                    .map(|binding| binding.map(|binding| binding.token)),
                bindings,
                "{operation}"
            );
            assert_eq!(
                [
                    callback.graphs.active.modules.cables[0].unwrap().token,
                    callback.graphs.active.modules.cables[1].unwrap().token
                ],
                routes,
                "{operation}"
            );
            assert_eq!(
                read_raw_pair(
                    &mut callback.graphs.active.engine,
                    ("osc", "out"),
                    ("sum", "out")
                ),
                read_raw_pair(&mut reference, ("osc", "out"), ("sum", "out")),
                "first sample after {operation}"
            );
            without_heap_activity(|| {
                callback.render(&mut [0.0_f32; 256], 1);
                reference.process_audio_block(&mut reference_frames);
            });
            assert_eq!(
                callback.performance.sequence_state("seq_a"),
                Some(a),
                "{operation}"
            );
            assert_eq!(
                callback.performance.sequence_state("seq_b"),
                Some(b),
                "{operation}"
            );
            assert!(
                callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed,
                "{operation}"
            );
            assert_eq!(
                read_raw_pair(
                    &mut callback.graphs.active.engine,
                    ("osc", "out"),
                    ("sum", "out")
                ),
                read_raw_pair(&mut reference, ("osc", "out"), ("sum", "out")),
                "advancing samples after {operation}"
            );
            assert!(callback.graphs.previous.is_none());
            while disposal.pop().is_ok() {}
        }
    }

    #[test]
    fn explicit_preset_run_assigns_even_when_saved_value_matches_and_history_preserves_step() {
        let patch = sequence_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        for action in [ModuleAction::Clock, ModuleAction::Run] {
            callback.performance.trigger("seq", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (2, false));
        assert_eq!(session.patch().nodes[0].params["running"], 1.0);
        let preset = sequence_preset(serde_json::json!({"running":1}));
        session
            .execute(
                "apply_module_preset",
                serde_json::json!({"id":"seq","preset":preset}),
            )
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (2, true));
        while disposal.pop().is_ok() {}
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        session.execute("undo", serde_json::json!({})).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (3, false));
        while disposal.pop().is_ok() {}
        session.execute("redo", serde_json::json!({})).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (3, true));
    }

    #[test]
    fn partial_preset_preserves_run_and_presence_and_history_restores_actual_capture() {
        let mut patch = sequence_patch();
        patch.nodes[0].params.remove("running");
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let preset = sequence_preset(serde_json::json!({"row1_1":7}));
        session
            .execute(
                "apply_module_preset",
                serde_json::json!({"id":"seq","preset":preset}),
            )
            .unwrap();
        assert!(!session.patch().nodes[0].params.contains_key("running"));
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert!(!callback.performance.sequence_state("seq").unwrap().running);
        while disposal.pop().is_ok() {}
        callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        session.execute("undo", serde_json::json!({})).unwrap();
        session.execute("redo", serde_json::json!({})).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert!(!callback.performance.sequence_state("seq").unwrap().running);
        assert!(!session.patch().nodes[0].params.contains_key("running"));
    }

    #[test]
    fn running_load_fences_cancel_old_actions_survive_stale_plans_and_do_not_replay() {
        let patch = sequence_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let stale = sequence_graph(&callback, &patch, 1);
        callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        callback.performance.sync_patch(&patch, 2);
        callback
            .performance
            .load_module_running(&[("seq".into(), false)].into());
        callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        assert!(updates.push(stale).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(callback.performance.snapshot()["rejected"], 1);
        assert_eq!(callback.performance.snapshot()["pending"], 1);
        while disposal.pop().is_ok() {}
        let older = sequence_graph(&callback, &patch, 1);
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (2, true));
        while disposal.pop().is_ok() {}
        assert!(updates.push(older).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert!(callback.performance.sequence_state("seq").unwrap().running);
        while disposal.pop().is_ok() {}
        callback.performance.sync_patch(&patch, 3);
        callback
            .performance
            .load_module_running(&[("seq".into(), false)].into());
        // A second state load is applied, even with the same saved patch.
        assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert!(!callback.performance.sequence_state("seq").unwrap().running);
    }

    #[test]
    fn running_load_is_incarnation_scoped_and_works_while_bypassed() {
        let mut patch = sequence_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        callback.performance.sync_patch(&patch, 2);
        callback
            .performance
            .load_module_running(&[("seq".into(), false)].into());
        callback.performance.retire_modules(&["seq".into()]);
        callback.performance.sync_patch(&patch, 3);
        assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert!(callback.performance.sequence_state("seq").unwrap().running);
        while disposal.pop().is_ok() {}
        patch.nodes[0].bypassed = true;
        callback.performance.sync_patch(&patch, 4);
        callback
            .performance
            .load_module_running(&[("seq".into(), false)].into());
        assert!(updates.push(sequence_graph(&callback, &patch, 4)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        patch.nodes[0].bypassed = false;
        callback.performance.sync_patch(&patch, 5);
        assert!(updates.push(sequence_graph(&callback, &patch, 5)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert!(!callback.performance.sequence_state("seq").unwrap().running);
    }

    #[test]
    fn initialize_preserves_paused_sequence_and_runs_once_across_history() {
        for explicit_running in [false, true] {
            for undo_before_publication in [false, true] {
                let mut patch = sequence_patch();
                if !explicit_running {
                    patch.nodes[0].params.remove("running");
                }
                let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
                let workspace = tempfile::tempdir().unwrap();
                let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
                session.replace_patch(patch, Some(0)).unwrap();
                session
                    .set_live_module_events(Some(callback.performance.clone()))
                    .unwrap();
                for action in [ModuleAction::Clock, ModuleAction::Clock, ModuleAction::Run] {
                    callback.performance.trigger("seq", action).unwrap();
                }
                without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
                let state = callback.performance.sequence_state("seq").unwrap();
                assert_eq!((state.step, state.running), (3, false));
                let token = callback.graphs.active.modules.states[0].unwrap().token;
                session
                    .execute("initialize_modules", serde_json::json!({"ids":["seq"]}))
                    .unwrap();
                if undo_before_publication {
                    session.execute("undo", serde_json::json!({})).unwrap();
                }
                let graph = sequence_graph(&callback, session.patch(), session.revision());
                assert_eq!(graph.modules.states[0].unwrap().token, token);
                assert!(updates.push(graph).is_ok());
                without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
                let state = callback.performance.sequence_state("seq").unwrap();
                assert_eq!((state.step, state.running), (1, false));
                while disposal.pop().is_ok() {}
                // Advance after initialization. A history transition must not
                // reapply Initialize or rewind the step. It does restore the
                // captured custom Run state, independent of later button presses.
                for action in [ModuleAction::Run, ModuleAction::Clock] {
                    callback.performance.trigger("seq", action).unwrap();
                }
                without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
                assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
                if !undo_before_publication {
                    session.execute("undo", serde_json::json!({})).unwrap();
                }
                session.execute("redo", serde_json::json!({})).unwrap();
                assert!(updates
                    .push(sequence_graph(
                        &callback,
                        session.patch(),
                        session.revision()
                    ))
                    .is_ok());
                without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
                let state = callback.performance.sequence_state("seq").unwrap();
                assert_eq!((state.step, state.running), (2, false));
            }
        }
    }

    #[test]
    fn initialize_while_bypassed_resets_sequence_without_restarting_it_on_resume() {
        let patch = sequence_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        for action in [ModuleAction::Clock, ModuleAction::Run] {
            callback.performance.trigger("seq", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (2, false));
        session
            .execute(
                "apply_edits",
                serde_json::json!({"edits":[
                    {"tool":"set_bypass","arguments":{"id":"seq","bypassed":true}},
                    {"tool":"initialize_modules","arguments":{"ids":["seq"]}}
                ]}),
            )
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let handle = callback
            .graphs
            .active
            .engine
            .resolve_module_telemetry("seq")
            .unwrap();
        let state = callback
            .graphs
            .active
            .engine
            .module_telemetry(handle)
            .unwrap();
        assert_eq!((state.step, state.running), (1, false));
        while disposal.pop().is_ok() {}
        session
            .execute(
                "set_bypass",
                serde_json::json!({"id":"seq","bypassed":false}),
            )
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (1, false));
        while disposal.pop().is_ok() {}
        for action in [ModuleAction::Run, ModuleAction::Clock] {
            callback.performance.trigger("seq", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        session
            .execute("initialize_modules", serde_json::json!({"ids":["seq"]}))
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 1);
    }

    #[test]
    fn initialization_cancels_old_actions_and_new_actions_wait_for_its_graph() {
        let patch = sequence_patch();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        callback.performance.sync_patch(&patch, 2);
        callback.performance.initialize_modules(&["seq".into()]);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        // Old Run is rejected now; new Clock waits for revision 2.
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let snapshot = callback.performance.snapshot();
        assert_eq!(snapshot["rejected"], 1);
        assert_eq!(snapshot["pending"], 1);
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (2, true));
        assert_eq!(callback.performance.snapshot()["pending"], 0);
    }

    #[test]
    fn initialization_fence_is_not_consumed_by_stale_graph_or_replayed_by_older_plan() {
        let patch = sequence_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let stale = sequence_graph(&callback, &patch, 1);
        callback.performance.sync_patch(&patch, 2);
        callback.performance.initialize_modules(&["seq".into()]);
        assert!(updates.push(stale).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let handle = callback
            .graphs
            .active
            .engine
            .resolve_module_telemetry("seq")
            .unwrap();
        assert_eq!(
            callback
                .graphs
                .active
                .engine
                .module_telemetry(handle)
                .unwrap()
                .step,
            2
        );
        while disposal.pop().is_ok() {}
        let older = sequence_graph(&callback, &patch, 1);
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 1);
        while disposal.pop().is_ok() {}
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        assert!(updates.push(older).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
    }

    #[test]
    fn initialization_releases_held_buttons_without_replacing_the_instance() {
        for kind in ["logic", "process", "pulses", "push", "gates"] {
            let patch = manual_patch(kind);
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            let (press, _) = manual_actions(kind);
            callback.performance.trigger("manual", press).unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
            assert!(
                callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            let token = callback.graphs.active.modules.bindings[0].unwrap().token;
            callback.performance.trigger("manual", press).unwrap();
            callback.performance.sync_patch(&patch, 2);
            callback.performance.initialize_modules(&["manual".into()]);
            let graph = sequence_graph(&callback, &patch, 2);
            assert_eq!(graph.modules.bindings[0].unwrap().token, token);
            assert!(updates.push(graph).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert!(
                !callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            let old = callback.graphs.previous.as_ref().unwrap();
            let handle = old.modules.bindings[0].unwrap().handle;
            assert!(!old.engine.module_control_state(handle).unwrap().pressed);
            assert_eq!(callback.performance.snapshot()["rejected"], 1);
            callback.performance.trigger("manual", press).unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert!(
                callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
        }
    }

    #[test]
    fn preset_load_and_history_release_held_buttons_without_replacing_the_instance() {
        for kind in ["logic", "process", "pulses", "push", "gates"] {
            let patch = manual_patch(kind);
            let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
            let workspace = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
            session.replace_patch(patch, Some(0)).unwrap();
            session
                .set_live_module_events(Some(callback.performance.clone()))
                .unwrap();
            let (press, _) = manual_actions(kind);
            let token = callback.graphs.active.modules.bindings[0].unwrap().token;
            for operation in ["apply_module_preset", "undo", "redo"] {
                callback.performance.trigger("manual", press).unwrap();
                without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
                while disposal.pop().is_ok() {}
                assert!(
                    callback
                        .performance
                        .module_control_state("manual")
                        .unwrap()
                        .pressed
                );
                let args = if operation == "apply_module_preset" {
                    serde_json::json!({"id":"manual","preset":{
                        "format":"openpatch-module-preset","version":1,"kind":kind,"params":{}
                    }})
                } else {
                    serde_json::json!({})
                };
                session.execute(operation, args).unwrap();
                let graph = sequence_graph(&callback, session.patch(), session.revision());
                assert_eq!(graph.modules.bindings[0].unwrap().token, token);
                assert!(updates.push(graph).is_ok());
                without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
                assert!(
                    !callback
                        .performance
                        .module_control_state("manual")
                        .unwrap()
                        .pressed
                );
                let old = callback.graphs.previous.as_ref().unwrap();
                let handle = old.modules.bindings[0].unwrap().handle;
                assert!(!old.engine.module_control_state(handle).unwrap().pressed);
            }
        }
    }

    #[test]
    fn initialization_at_defaults_preserves_vco_and_unrelated_oscillator_samples() {
        let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"vco","kind":"multi_vco","params":{}},
                {"id":"osc","kind":"oscillator","params":{"frequency":123}}],
            "output":"vco","output_port":"saw"}))
        .unwrap();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        let mut reference = patch.compile().unwrap();
        let mut frames = [[0.0; 2]; 137];
        without_heap_activity(|| callback.render(&mut [0.0_f32; 137], 1));
        reference.process_audio_block(&mut frames);
        session
            .execute("initialize_modules", serde_json::json!({"ids":["vco"]}))
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 137], 1));
        reference.process_audio_block(&mut frames);
        let actual = read_raw_pair(
            &mut callback.graphs.active.engine,
            ("vco", "saw"),
            ("osc", "out"),
        );
        let expected = read_raw_pair(&mut reference, ("vco", "saw"), ("osc", "out"));
        assert_eq!(actual, expected);
    }

    fn switch_patch(kind: &str) -> Patch {
        serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"clock","kind":"constant","params":{"value":10}},
                {"id":"switch","kind":kind,"params":{"steps":4,"declick":0}}],
            "cables":[{"from":"clock","to":"switch","input":"clock"}],
            "output":"clock"}))
        .unwrap()
    }

    #[test]
    fn switch_telemetry_reports_dsp_steps_and_rejects_actions_before_queueing() {
        for kind in ["switch_1to4", "switch_4to1"] {
            let patch = switch_patch(kind);
            let (mut callback, _, _) = sequence_callback(&patch);
            assert!(callback.performance.sequence_state("switch").is_none());
            for action in [ModuleAction::Clock, ModuleAction::Reset, ModuleAction::Run] {
                assert!(callback
                    .performance
                    .trigger("switch", action)
                    .unwrap_err()
                    .contains("read-only telemetry"));
            }
            let status = callback.performance.snapshot();
            assert_eq!(status["queued"], 0);
            assert_eq!(status["pending"], 0);
            assert_eq!(status["rejected"], 3);
            without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
            let state = callback.performance.sequence_state("switch").unwrap();
            assert_eq!((state.step, state.length, state.running), (2, 4, true));
        }
    }

    #[test]
    fn switch_compatible_edits_retain_state_and_wait_for_current_graph_telemetry() {
        for kind in ["switch_1to4", "switch_4to1"] {
            let mut patch = switch_patch(kind);
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
            let token = callback.graphs.active.modules.bindings[0].unwrap().token;
            patch.nodes[1].params.insert("declick".into(), 1.0);
            patch.nodes[1].params.insert("steps".into(), 3.0);
            callback.performance.sync_patch(&patch, 2);
            assert!(callback.performance.sequence_state("switch").is_none());
            // The old graph may publish again while its successor is prepared.
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert!(callback.performance.sequence_state("switch").is_none());
            let graph = sequence_graph(&callback, &patch, 2);
            assert_eq!(graph.modules.bindings[0].unwrap().token, token);
            assert!(updates.push(graph).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            let state = callback.performance.sequence_state("switch").unwrap();
            assert_eq!((state.step, state.length, state.running), (2, 3, true));
        }
    }

    #[test]
    fn switch_retyping_and_skipped_remove_readd_reset_identity_and_delayed_clock() {
        for kind in ["switch_1to4", "switch_4to1"] {
            let patch = switch_patch(kind);
            for retype in [false, true] {
                let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
                without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
                assert_eq!(
                    callback.performance.sequence_state("switch").unwrap().step,
                    2
                );
                let token = callback.graphs.active.modules.bindings[0].unwrap().token;
                if retype {
                    let mut opposite = patch.clone();
                    opposite.nodes[1].kind = if kind == "switch_1to4" {
                        "switch_4to1".into()
                    } else {
                        "switch_1to4".into()
                    };
                    callback.performance.sync_patch(&opposite, 2);
                    let opposite_graph = sequence_graph(&callback, &opposite, 2);
                    assert_ne!(opposite_graph.modules.bindings[0].unwrap().token, token);
                    let stale = sequence_graph(&callback, &patch, 2);
                    assert!(stale.modules.bindings.iter().all(Option::is_none));
                } else {
                    callback.performance.sync_patch(&Patch::new_v2(), 2);
                }
                callback.performance.sync_patch(&patch, 3);
                assert!(callback.performance.sequence_state("switch").is_none());
                let stale = sequence_graph(&callback, &patch, 1);
                assert!(stale.modules.bindings.iter().all(Option::is_none));
                let graph = sequence_graph(&callback, &patch, 3);
                assert_ne!(graph.modules.bindings[0].unwrap().token, token);
                assert!(updates.push(graph).is_ok());
                without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
                assert_eq!(
                    callback.performance.sequence_state("switch").unwrap().step,
                    1
                );
                without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
                assert_eq!(
                    callback.performance.sequence_state("switch").unwrap().step,
                    2
                );
            }
        }
    }

    #[test]
    fn identical_switch_document_replacement_resets_state_and_clock_edges() {
        for kind in ["switch_1to4", "switch_4to1"] {
            let patch = switch_patch(kind);
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            let workspace = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
            session.replace_patch(patch.clone(), Some(0)).unwrap();
            session
                .set_live_module_events(Some(callback.performance.clone()))
                .unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
            assert_eq!(
                callback.performance.sequence_state("switch").unwrap().step,
                2
            );
            let token = callback.graphs.active.modules.bindings[0].unwrap().token;
            session.replace_patch(patch.clone(), Some(1)).unwrap();
            assert!(callback.performance.sequence_state("switch").is_none());
            let graph = sequence_graph(&callback, session.patch(), session.revision());
            assert_ne!(graph.modules.bindings[0].unwrap().token, token);
            assert!(updates.push(graph).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert_eq!(
                callback.performance.sequence_state("switch").unwrap().step,
                1
            );
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert_eq!(
                callback.performance.sequence_state("switch").unwrap().step,
                2
            );
        }
    }

    #[test]
    fn sequencer_buttons_are_ordered_runtime_actions_and_publish_actual_dsp_telemetry() {
        let patch = sequence_patch();
        let (mut callback, _, _) = sequence_callback(&patch);
        let receipt = callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        assert_eq!(receipt["queued"], true);
        assert_eq!(receipt["applied"], false);
        assert_eq!(receipt["minimum_revision"], 1);
        for action in [ModuleAction::Clock, ModuleAction::Run, ModuleAction::Clock] {
            callback.performance.trigger("seq", action).unwrap();
        }
        assert!(callback.performance.sequence_state("seq").is_none());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running, state.length), (2, true, 8));
        let status = callback.performance.snapshot();
        assert_eq!(status["applied"], 4);
        assert_eq!(status["last_applied_sequence"], 4);
        assert_eq!(status["pending"], 0);
        assert_eq!(patch.nodes[0].params["running"], 1.0);
    }

    #[test]
    fn queued_buttons_survive_compatible_graph_edits_and_fan_to_both_crossfade_engines() {
        let mut patch = sequence_patch();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        callback.render(&mut [0.0_f32; 1], 1);
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        patch.nodes[0].params.insert("row1_2".into(), 4.0);
        callback.performance.sync_patch(&patch, 2);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        assert!(callback.performance.sequence_state("seq").is_none());
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 3);
        let old = callback.graphs.previous.as_ref().unwrap();
        let handle = old.modules.bindings.iter().flatten().next().unwrap().handle;
        assert_eq!(old.engine.module_telemetry(handle).unwrap().step, 3);
        assert_eq!(callback.performance.snapshot()["applied"], 2);
    }

    #[test]
    fn skipped_remove_readd_invalidates_old_actions_and_resets_reincarnated_runtime() {
        let patch = sequence_patch();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        callback.render(&mut [0.0_f32; 1], 1);
        assert!(!callback.performance.sequence_state("seq").unwrap().running);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        callback.performance.sync_patch(&Patch::new_v2(), 2);
        callback.performance.sync_patch(&patch, 3);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        let stale = sequence_graph(&callback, &patch, 1);
        assert!(stale.modules.bindings.iter().all(Option::is_none));
        assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (2, true));
        let status = callback.performance.snapshot();
        assert_eq!(status["applied"], 2);
        assert_eq!(status["obsolete"], 1);
        assert_eq!(status["dropped"], 1);
    }

    #[test]
    fn same_batch_identical_replacement_retires_actions_and_clears_incoming_high_clock() {
        let mut patch = sequence_patch();
        // Match fresh-connection presentation ranks so replacement remains
        // byte-identical while runtime incarnation effects are exercised.
        patch.cables[0].input_order = Some(1);
        patch.cables[0].output_order = Some(1);
        patch.nodes[1].params.insert("value".into(), 10.0);
        // Keep the replacement at its original position so the committed graph
        // is byte-equivalent even though its sequencer identity has changed.
        patch.nodes.swap(0, 1);
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        callback.render(&mut [0.0_f32; 2], 1);
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);

        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        let node = &patch.nodes[1];
        session
            .execute(
                "apply_edits",
                serde_json::json!({"expected_revision":1,"edits":[
                    {"tool":"remove_module","arguments":{"id":"seq"}},
                    {"tool":"add_module","arguments":{"id":"seq","kind":"seq3","params":node.params}},
                    {"tool":"connect","arguments":{"from":"zero","to":"seq","input":"clock"}},
                    {"tool":"set_output","arguments":{"id":"seq","port":"cv1"}}
                ]}),
            )
            .unwrap();
        assert_eq!(
            serde_json::to_value(session.patch()).unwrap(),
            serde_json::to_value(&patch).unwrap()
        );
        assert!(callback.performance.sequence_state("seq").is_none());
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());

        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let state = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((state.step, state.running), (1, true));
        let status = callback.performance.snapshot();
        assert_eq!(status["obsolete"], 1);
        assert_eq!(status["applied"], 0);
        assert_eq!(status["pending"], 0);
        // Only a freshly delivered cable frame can clock the new instance.
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
    }

    #[test]
    fn identical_document_reload_starts_fresh_but_rejected_replacement_preserves_instance() {
        let patch = sequence_patch();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let paused = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((paused.step, paused.running), (2, false));
        let old_token = callback.graphs.active.modules.bindings[0].unwrap().token;
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();

        let before = session.snapshot();
        let mut invalid = patch.clone();
        invalid.nodes[0].params.insert("steps".into(), 9.0);
        assert!(session.replace_patch(invalid, Some(1)).is_err());
        assert!(session.replace_patch(patch.clone(), Some(0)).is_err());
        assert_eq!(session.snapshot(), before);
        assert_eq!(callback.performance.sequence_state("seq"), Some(paused));
        let unchanged = sequence_graph(&callback, session.patch(), session.revision());
        assert_eq!(unchanged.modules.bindings[0].unwrap().token, old_token);
        let status = callback.performance.snapshot();
        assert_eq!(status["pending"], 1);
        assert_eq!(status["obsolete"], 0);

        session.replace_patch(patch.clone(), Some(1)).unwrap();
        assert!(callback.performance.sequence_state("seq").is_none());
        let reloaded = sequence_graph(&callback, session.patch(), session.revision());
        assert_ne!(reloaded.modules.bindings[0].unwrap().token, old_token);
        assert!(updates.push(reloaded).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let fresh = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((fresh.step, fresh.running), (1, true));
        let status = callback.performance.snapshot();
        assert_eq!(status["obsolete"], 1);
        assert_eq!(status["applied"], 2);
        assert_eq!(status["pending"], 0);
    }

    #[test]
    fn bypass_freezes_sequence_identity_rejects_actions_and_allows_parameter_edits() {
        let patch = sequence_patch();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        let token = callback.graphs.active.modules.bindings[0].unwrap().token;
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        session
            .execute(
                "set_bypass",
                serde_json::json!({"id":"seq","bypassed":true}),
            )
            .unwrap();
        assert!(callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap_err()
            .contains("bypassed"));
        assert!(session
            .execute(
                "trigger_module",
                serde_json::json!({"id":"seq","action":"reset"})
            )
            .unwrap_err()
            .contains("bypassed"));
        let bypassed = sequence_graph(&callback, session.patch(), session.revision());
        assert_eq!(bypassed.modules.bindings[0].unwrap().token, token);
        assert!(updates.push(bypassed).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        let status = callback.performance.snapshot();
        assert_eq!(status["applied"], 1);
        assert_eq!(status["dropped"], 1);
        assert_eq!(status["rejected"], 2);
        assert_eq!(status["obsolete"], 0);
        session
            .execute(
                "set_parameter",
                serde_json::json!({"id":"seq","parameter":"row1_2","value":4}),
            )
            .unwrap();
        let edited = sequence_graph(&callback, session.patch(), session.revision());
        assert_eq!(edited.modules.bindings[0].unwrap().token, token);
        assert!(updates.push(edited).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        session
            .execute(
                "set_bypass",
                serde_json::json!({"id":"seq","bypassed":false}),
            )
            .unwrap();
        let resumed = sequence_graph(&callback, session.patch(), session.revision());
        assert_eq!(resumed.modules.bindings[0].unwrap().token, token);
        assert!(updates.push(resumed).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 3);
    }

    #[test]
    fn coalesced_bypass_resume_cancels_old_actions_without_resetting_dsp_state() {
        let patch = sequence_patch();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let token = callback.graphs.active.modules.bindings[0].unwrap().token;
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        session
            .execute(
                "set_bypass",
                serde_json::json!({"id":"seq","bypassed":true}),
            )
            .unwrap();
        session
            .execute(
                "set_bypass",
                serde_json::json!({"id":"seq","bypassed":false}),
            )
            .unwrap();
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        // The prior graph cannot apply a pre-bypass action while waiting for
        // the coalesced replacement. The post-resume action waits for revision.
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let status = callback.performance.snapshot();
        assert_eq!(status["applied"], 1);
        assert_eq!(status["dropped"], 1);
        assert_eq!(status["pending"], 1);
        let resumed = sequence_graph(&callback, session.patch(), session.revision());
        assert_eq!(resumed.modules.bindings[0].unwrap().token, token);
        assert!(updates.push(resumed).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 3);
        let old = callback.graphs.previous.as_ref().unwrap();
        let handle = old.modules.bindings[0].unwrap().handle;
        assert_eq!(old.engine.module_telemetry(handle).unwrap().step, 3);
        let status = callback.performance.snapshot();
        assert_eq!(status["applied"], 2);
        assert_eq!(status["dropped"], 1);
        assert_eq!(status["rejected"], 1);
        assert_eq!(status["obsolete"], 0);
        assert_eq!(status["pending"], 0);
    }

    #[test]
    fn gates_layout_history_preserves_delay_and_live_reset_clears_both_crossfade_engines() {
        let mut patch = manual_patch("gates");
        patch.nodes[1].params.insert("length".into(), 1.0 / 512.0);
        patch.assign_missing_cable_ids().unwrap();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        let mut reference = patch.compile().unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 32], 1));
        reference.process_block(&mut [0.0; 32]);
        session
            .record_layout_checkpoint(session.revision())
            .unwrap();
        let graph = sequence_graph(&callback, session.patch(), session.revision());
        assert!(updates.push(graph).is_ok());
        // Traverse both sides of the delayed rising edge after graph adoption.
        for _ in 0..80 {
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            reference.process_block(&mut [0.0; 1]);
            assert_eq!(
                read_raw_pair(
                    &mut callback.graphs.active.engine,
                    ("manual", "gate"),
                    ("manual", "delay")
                ),
                read_raw_pair(&mut reference, ("manual", "gate"), ("manual", "delay"))
            );
        }
        for action in [ModuleAction::ResetPress, ModuleAction::ResetRelease] {
            session
                .execute(
                    "trigger_module",
                    serde_json::json!({"id":"manual","action":action}),
                )
                .unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let lights = callback.performance.module_lights("manual").unwrap();
        assert_eq!(lights.count, 6);
        assert!(!lights.manual);
        // Lights decay on a divided clock; signal outputs change immediately.
        assert!(lights.brightness[..6]
            .iter()
            .flatten()
            .all(|v| (0.0..=1.0).contains(v)));
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("manual", "flip"),
                ("manual", "flop")
            ),
            [10.0, 0.0]
        );
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("manual", "gate"),
                ("manual", "delay")
            ),
            [0.0, 0.0]
        );
        let old = callback.graphs.previous.as_mut().unwrap();
        assert_eq!(
            read_raw_pair(&mut old.engine, ("manual", "gate"), ("manual", "delay")),
            [0.0, 0.0]
        );
        let snapshot = callback.performance.snapshot();
        assert_eq!(snapshot["applied"], 2);
        assert_eq!(
            snapshot["modules"]["manual"]["state"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn pulses_publish_ten_independent_rows_and_validate_rust_action_indices() {
        let patch = manual_patch("pulses");
        let (mut callback, _, _) = sequence_callback(&patch);
        for action in [
            ModuleAction::PulsePress(0),
            ModuleAction::PulsePress(11),
            ModuleAction::PulseRelease(0),
            ModuleAction::PulseRelease(255),
            ModuleAction::GatePress,
        ] {
            assert!(callback.performance.trigger("manual", action).is_err());
        }
        for row in 1..=10 {
            callback
                .performance
                .trigger("manual", ModuleAction::PulsePress(row))
                .unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
        let controls = callback.performance.module_control_state("manual").unwrap();
        assert!(controls.pressed);
        assert_eq!(controls.pressed_mask, 0x3ff);
        let lights = callback.performance.module_lights("manual").unwrap();
        assert_eq!(lights.count, 10);
        assert!(lights.brightness[..10].iter().all(|pair| pair[0] > 0.0));
        assert!(lights.brightness[10..].iter().all(|pair| *pair == [0.0; 2]));
        let snapshot = callback.performance.snapshot();
        assert_eq!(snapshot["coherent"], true);
        assert_eq!(
            snapshot["modules"]["manual"]["controls"]["pressed_mask"],
            0x3ff
        );
        callback
            .performance
            .trigger("manual", ModuleAction::PulseRelease(1))
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            callback
                .performance
                .module_control_state("manual")
                .unwrap()
                .pressed_mask,
            0x3fe
        );
        callback
            .performance
            .trigger("manual", ModuleAction::PulseRelease(10))
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            callback
                .performance
                .module_control_state("manual")
                .unwrap()
                .pressed_mask,
            0x1fe
        );
        // A saturated row release clears every held row, invalidating all older presses.
        for _ in 0..EVENT_CAPACITY {
            callback
                .performance
                .trigger("manual", ModuleAction::PulsePress(10))
                .unwrap();
        }
        let receipt = callback
            .performance
            .trigger("manual", ModuleAction::PulseRelease(1))
            .unwrap();
        assert_eq!(receipt["emergency_release"], true);
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let controls = callback.performance.module_control_state("manual").unwrap();
        assert!(!controls.pressed);
        assert_eq!(controls.pressed_mask, 0);
        assert_eq!(callback.performance.snapshot()["pending"], 0);
    }

    #[test]
    fn pulses_layout_checkpoint_preserves_held_rows_and_exact_trigger_tail() {
        let patch = manual_patch("pulses");
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        for action in [
            ModuleAction::PulsePress(1),
            ModuleAction::PulsePress(10),
            ModuleAction::PulseRelease(10),
        ] {
            callback.performance.trigger("manual", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1)); // sample 1
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("manual", "gate1"),
                ("manual", "trig10")
            ),
            [10.0, 10.0]
        ); // sample 2
        let token = callback.graphs.active.modules.bindings[0].unwrap().token;
        session
            .record_layout_checkpoint(session.revision())
            .unwrap();
        let graph = sequence_graph(&callback, session.patch(), session.revision());
        assert_eq!(graph.modules.bindings[0].unwrap().token, token);
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1)); // sample 3
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("manual", "gate1"),
                ("manual", "trig10")
            ),
            [10.0, 10.0]
        ); // sample 4
        assert_eq!(
            callback
                .performance
                .module_control_state("manual")
                .unwrap()
                .pressed_mask,
            1
        );
        without_heap_activity(|| callback.render(&mut [0.0_f32; 42], 1)); // samples 5..46
        for _ in 0..2 {
            // final high samples 47,48
            assert_eq!(
                read_raw_pair(
                    &mut callback.graphs.active.engine,
                    ("manual", "gate1"),
                    ("manual", "trig10")
                ),
                [10.0, 10.0]
            );
        }
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("manual", "gate1"),
                ("manual", "trig10")
            ),
            [10.0, 0.0]
        ); // sample49
    }

    fn manual_patch(kind: &str) -> Patch {
        if kind == "gates" {
            return serde_json::from_value(serde_json::json!({"version":2,
                "nodes":[{"id":"signal","kind":"constant","params":{"value":2}},
                    {"id":"manual","kind":"gates"}],
                "cables":[{"from":"signal","to":"manual","input":"in"}],
                "output":"manual","output_port":"gate"}))
            .unwrap();
        }
        if kind == "push" {
            return serde_json::from_value(serde_json::json!({"version":2,
                "nodes":[{"id":"signal","kind":"constant","params":{"value":2}},
                    {"id":"manual","kind":"push","params":{"holding":0}}],
                "output":"manual","output_port":"gate"}))
            .unwrap();
        }
        if kind == "pulses" {
            return serde_json::from_value(serde_json::json!({"version":2,
                "nodes":[{"id":"signal","kind":"constant","params":{"value":2}},
                    {"id":"manual","kind":"pulses"}],
                "output":"manual","output_port":"gate10"}))
            .unwrap();
        }
        serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"signal","kind":"constant","params":{"value":2}},
                {"id":"manual","kind":kind}],
            "cables":[{"from":"signal","to":"manual","input":if kind=="logic" {"a"} else {"signal"}}],
            "output":"manual","output_port":if kind=="logic" {"or"} else {"track_hold"}})).unwrap()
    }
    fn manual_actions(kind: &str) -> (ModuleAction, ModuleAction) {
        if kind == "gates" {
            (ModuleAction::ResetPress, ModuleAction::ResetRelease)
        } else if kind == "pulses" {
            (ModuleAction::PulsePress(10), ModuleAction::PulseRelease(10))
        } else if kind == "push" {
            (ModuleAction::PushPress, ModuleAction::PushRelease)
        } else if kind == "logic" {
            (ModuleAction::BPress, ModuleAction::BRelease)
        } else {
            (ModuleAction::GatePress, ModuleAction::GateRelease)
        }
    }

    #[test]
    fn manual_controls_publish_coherent_actual_lights_and_preserve_ordered_releases() {
        for kind in ["logic", "process", "pulses", "push", "gates"] {
            let patch = manual_patch(kind);
            let (mut callback, _, _) = sequence_callback(&patch);
            let (press, release) = manual_actions(kind);
            assert!(callback
                .performance
                .module_control_state("manual")
                .is_none());
            let before = callback.performance.snapshot();
            assert_eq!(before["modules"]["manual"]["available"], false);
            assert_eq!(
                before["modules"]["manual"]["revision"],
                serde_json::Value::Null
            );
            for action in [press, release, press] {
                callback.performance.trigger("manual", action).unwrap();
            }
            without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
            assert!(
                callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            let lights = callback.performance.module_lights("manual").unwrap();
            assert!(lights.manual);
            assert_eq!(
                lights.count,
                match kind {
                    "logic" => 8,
                    "pulses" => 10,
                    "push" => 2,
                    "gates" => 6,
                    _ => 0,
                }
            );
            assert!(callback.performance.sequence_state("manual").is_none());
            let actual = callback.performance.snapshot();
            assert_eq!(actual["modules"]["manual"]["available"], true);
            assert_eq!(actual["modules"]["manual"]["revision"], 1);
            assert_eq!(actual["modules"]["manual"]["controls"]["pressed"], true);
            assert_eq!(actual["modules"]["manual"]["lights"]["manual"], true);
            assert_eq!(
                actual["modules"]["manual"]["state"],
                if kind == "push" {
                    serde_json::json!({"holding":false})
                } else {
                    serde_json::Value::Null
                }
            );
            callback.performance.trigger("manual", release).unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert!(
                !callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            assert!(!callback.performance.module_lights("manual").unwrap().manual);
            assert_eq!(callback.performance.snapshot()["applied"], 4);
        }
    }

    #[test]
    fn full_manual_queue_uses_emergency_release_and_cancels_old_presses() {
        for kind in ["logic", "process", "pulses", "push", "gates"] {
            let mut patch = manual_patch(kind);
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            let (press, release) = manual_actions(kind);
            callback.performance.trigger("manual", press).unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
            for _ in 0..EVENT_CAPACITY {
                callback.performance.trigger("manual", press).unwrap();
            }
            let receipt = callback.performance.trigger("manual", release).unwrap();
            assert_eq!(receipt["queued"], false);
            assert_eq!(receipt["emergency_release"], true);
            assert_eq!(receipt["applied"], false);
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert!(
                !callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            let status = callback.performance.snapshot();
            assert_eq!(status["queued"], EVENT_CAPACITY + 1);
            assert_eq!(status["applied"], 1);
            assert_eq!(status["dropped"], EVENT_CAPACITY);
            assert_eq!(status["pending"], 0);
            assert_eq!(status["backpressure"], 1);
            assert_eq!(status["emergency_release_requests"], 1);
            assert_eq!(status["emergency_release_applied"], 1);
            // New presses after the safety epoch remain valid across an edit.
            callback.performance.trigger("manual", press).unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            patch.nodes[0].params.insert("value".into(), 3.0);
            callback.performance.sync_patch(&patch, 2);
            assert!(callback.performance.module_lights("manual").is_none());
            assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert!(
                callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            let old = callback.graphs.previous.as_ref().unwrap();
            let handle = old.modules.bindings[0].unwrap().handle;
            assert!(old.engine.module_control_state(handle).unwrap().pressed);
            assert_eq!(
                callback.performance.snapshot()["emergency_release_applied"],
                1
            );
            for _ in 0..EVENT_CAPACITY {
                callback.performance.trigger("manual", press).unwrap();
            }
            assert_eq!(
                callback.performance.trigger("manual", release).unwrap()["emergency_release"],
                true
            );
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert!(
                !callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            let old = callback.graphs.previous.as_ref().unwrap();
            assert!(
                !old.engine
                    .module_control_state(old.modules.bindings[0].unwrap().handle)
                    .unwrap()
                    .pressed
            );
            assert_eq!(
                callback.performance.snapshot()["emergency_release_applied"],
                2
            );
        }
    }

    #[test]
    fn coalesced_manual_bypass_releases_held_controls_and_allows_bypassed_release() {
        for kind in ["logic", "process", "pulses", "push", "gates"] {
            let patch = manual_patch(kind);
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            let workspace = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
            session.replace_patch(patch.clone(), Some(0)).unwrap();
            session
                .set_live_module_events(Some(callback.performance.clone()))
                .unwrap();
            let (press, release) = manual_actions(kind);
            callback.performance.trigger("manual", press).unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
            assert!(
                callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            let token = callback.graphs.active.modules.bindings[0].unwrap().token;
            callback.performance.trigger("manual", press).unwrap();
            session
                .execute(
                    "set_bypass",
                    serde_json::json!({"id":"manual","bypassed":true}),
                )
                .unwrap();
            assert!(callback
                .performance
                .trigger("manual", press)
                .unwrap_err()
                .contains("bypassed"));
            callback.performance.trigger("manual", release).unwrap();
            session
                .execute(
                    "set_bypass",
                    serde_json::json!({"id":"manual","bypassed":false}),
                )
                .unwrap();
            let resumed = sequence_graph(&callback, session.patch(), session.revision());
            assert_eq!(resumed.modules.bindings[0].unwrap().token, token);
            assert!(updates.push(resumed).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert!(
                !callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            assert!(!callback.performance.module_lights("manual").unwrap().manual);
            let old = callback.graphs.previous.as_ref().unwrap();
            assert!(
                !old.engine
                    .module_control_state(old.modules.bindings[0].unwrap().handle)
                    .unwrap()
                    .pressed
            );
            let status = callback.performance.snapshot();
            assert_eq!(status["applied"], 2);
            assert_eq!(status["dropped"], 1);
            assert_eq!(status["emergency_release_applied"], 1);
        }
    }

    #[test]
    fn removed_manual_module_reappears_with_fresh_controls_and_lights() {
        for kind in ["logic", "process", "pulses", "push", "gates"] {
            let patch = manual_patch(kind);
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            let (press, _) = manual_actions(kind);
            callback.performance.trigger("manual", press).unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
            let token = callback.graphs.active.modules.bindings[0].unwrap().token;
            callback.performance.trigger("manual", press).unwrap();
            callback.performance.sync_patch(&Patch::new_v2(), 2);
            callback.performance.sync_patch(&patch, 3);
            let recreated = sequence_graph(&callback, &patch, 3);
            assert_ne!(recreated.modules.bindings[0].unwrap().token, token);
            assert!(updates.push(recreated).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert!(
                !callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed
            );
            assert!(!callback.performance.module_lights("manual").unwrap().manual);
            assert_eq!(callback.performance.snapshot()["obsolete"], 1);
        }
    }

    #[test]
    fn push_holding_publication_preserves_actual_latch_across_edits_and_bypass() {
        let mut patch = manual_patch("push");
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        for action in [ModuleAction::HoldPress, ModuleAction::HoldRelease] {
            callback.performance.trigger("manual", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
        let actual = callback.performance.snapshot();
        assert_eq!(
            actual["modules"]["manual"]["state"],
            serde_json::json!({"holding":true})
        );
        assert_eq!(actual["modules"]["manual"]["controls"]["pressed_mask"], 0);
        assert!(callback.performance.sequence_state("manual").is_none());
        let lights = callback.performance.module_lights("manual").unwrap();
        assert_eq!(lights.count, 2);
        assert_eq!(lights.brightness[0][0], 1.0);
        assert_eq!(lights.brightness[1][0], 1.0);
        assert!(!lights.manual);
        assert_eq!(patch.nodes[1].params["holding"], 0.0);
        for (revision, bypassed) in [(2, false), (3, true), (4, false)] {
            patch.nodes[0]
                .params
                .insert("value".into(), revision as f32);
            patch.nodes[1].bypassed = bypassed;
            callback.performance.sync_patch(&patch, revision);
            assert!(updates
                .push(sequence_graph(&callback, &patch, revision))
                .is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
            while disposal.pop().is_ok() {}
            assert_eq!(
                callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
                true
            );
            let handle = callback
                .graphs
                .active
                .engine
                .resolve_module_state("manual")
                .unwrap();
            assert_eq!(
                callback.graphs.active.engine.module_saved_state(handle),
                Some(openpatch_core::SavedModuleState::PushHold(true))
            );
        }
    }

    #[test]
    fn push_emergency_release_clears_both_buttons_and_preserves_holding_in_both_engines() {
        for release in [ModuleAction::PushRelease, ModuleAction::HoldRelease] {
            let mut patch = manual_patch("push");
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            for action in [ModuleAction::HoldPress, ModuleAction::PushPress] {
                callback.performance.trigger("manual", action).unwrap();
            }
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert_eq!(
                callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed_mask,
                3
            );
            patch.nodes[0].params.insert("value".into(), 3.0);
            callback.performance.sync_patch(&patch, 2);
            assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            for _ in 0..EVENT_CAPACITY {
                callback
                    .performance
                    .trigger("manual", ModuleAction::PushPress)
                    .unwrap();
            }
            assert!(callback
                .performance
                .trigger("manual", ModuleAction::GateRelease)
                .is_err());
            assert_eq!(
                callback.performance.trigger("manual", release).unwrap()["emergency_release"],
                true
            );
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            assert_eq!(
                callback
                    .performance
                    .module_control_state("manual")
                    .unwrap()
                    .pressed_mask,
                0
            );
            assert_eq!(
                callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
                true
            );
            for graph in [
                &callback.graphs.active,
                callback.graphs.previous.as_ref().unwrap(),
            ] {
                let handle = graph.engine.resolve_module_state("manual").unwrap();
                assert_eq!(
                    graph
                        .engine
                        .module_control_state(handle)
                        .unwrap()
                        .pressed_mask,
                    0
                );
                assert_eq!(
                    graph.engine.module_saved_state(handle),
                    Some(openpatch_core::SavedModuleState::PushHold(true))
                );
            }
            assert_eq!(callback.performance.snapshot()["dropped"], EVENT_CAPACITY);
        }
    }

    #[test]
    fn holding_load_fences_stale_actions_and_plans_and_assigns_both_crossfade_engines() {
        let patch = manual_patch("push");
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        for action in [ModuleAction::HoldPress, ModuleAction::HoldRelease] {
            callback.performance.trigger("manual", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let stale = sequence_graph(&callback, &patch, 1);
        callback
            .performance
            .trigger("manual", ModuleAction::HoldPress)
            .unwrap();
        callback.performance.sync_patch(&patch, 2);
        callback
            .performance
            .load_module_holding(&[("manual".into(), false)].into());
        assert!(updates.push(stale).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(callback.performance.snapshot()["rejected"], 1);
        while disposal.pop().is_ok() {}
        let older = sequence_graph(&callback, &patch, 1);
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        for graph in [
            &callback.graphs.active,
            callback.graphs.previous.as_ref().unwrap(),
        ] {
            let handle = graph.engine.resolve_module_state("manual").unwrap();
            assert_eq!(
                graph.engine.module_saved_state(handle),
                Some(openpatch_core::SavedModuleState::PushHold(false))
            );
        }
        // The patch already configured false: an explicit load still wins over
        // actual HOLD. Older prepared graphs cannot replay it after a new press.
        for action in [ModuleAction::HoldPress, ModuleAction::HoldRelease] {
            callback.performance.trigger("manual", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        assert!(updates.push(older).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        assert!(updates.push(sequence_graph(&callback, &patch, 2)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        assert_eq!(
            callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
            true
        );
        // Loading the wrong saved-state kind must neither mutate nor fence PUSH.
        callback.performance.sync_patch(&patch, 3);
        callback
            .performance
            .trigger("manual", ModuleAction::HoldPress)
            .unwrap();
        callback
            .performance
            .load_module_running(&[("manual".into(), false)].into());
        assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        assert_eq!(
            callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
            false
        );
        callback.performance.sync_patch(&patch, 4);
        callback
            .performance
            .load_module_holding(&[("manual".into(), true)].into());
        callback.performance.retire_modules(&["manual".into()]);
        callback.performance.sync_patch(&patch, 5);
        assert!(updates.push(sequence_graph(&callback, &patch, 5)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
            false
        );
    }

    #[test]
    fn initialize_mixed_saved_states_resets_push_hold_and_preserves_sequence_run() {
        let mut patch = sequence_patch();
        patch.nodes.push(
            serde_json::from_value(
                serde_json::json!({"id":"manual","kind":"push","params":{"holding":0}}),
            )
            .unwrap(),
        );
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        for action in [ModuleAction::Clock, ModuleAction::Run] {
            callback.performance.trigger("seq", action).unwrap();
        }
        for action in [ModuleAction::HoldPress, ModuleAction::HoldRelease] {
            callback.performance.trigger("manual", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
            true
        );
        session
            .execute(
                "initialize_modules",
                serde_json::json!({"ids":["seq","manual"]}),
            )
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let sequence = callback.performance.sequence_state("seq").unwrap();
        assert_eq!((sequence.step, sequence.running), (1, false));
        assert_eq!(
            callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
            false
        );
        for graph in [
            &callback.graphs.active,
            callback.graphs.previous.as_ref().unwrap(),
        ] {
            let handle = graph.engine.resolve_module_state("manual").unwrap();
            assert_eq!(
                graph.engine.module_saved_state(handle),
                Some(openpatch_core::SavedModuleState::PushHold(false))
            );
        }
    }

    #[test]
    fn push_project_and_preset_capture_and_duplication_use_actual_coherent_holding() {
        let mut patch = sequence_patch();
        patch.nodes.push(
            serde_json::from_value(serde_json::json!({"id":"manual","kind":"push"})).unwrap(),
        );
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        assert!(session.capture_project_patch(Some(1)).is_err());
        assert!(session
            .execute("capture_module_preset", serde_json::json!({"id":"manual"}))
            .is_err());
        callback
            .performance
            .trigger("seq", ModuleAction::Run)
            .unwrap();
        for action in [
            ModuleAction::HoldPress,
            ModuleAction::HoldRelease,
            ModuleAction::PushPress,
        ] {
            callback.performance.trigger("manual", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 137], 1));
        let before_live = callback.performance.snapshot();
        let before_session = session.snapshot();
        let captured = session.capture_project_patch(Some(1)).unwrap();
        assert_eq!(captured.frame, Some(137));
        assert!(!captured.running["seq"]);
        assert!(captured.holding["manual"]);
        assert_eq!(captured.patch.nodes[2].params["holding"], 1.0);
        assert!(!session.patch().nodes[2].params.contains_key("holding"));
        let preset = session
            .execute("capture_module_preset", serde_json::json!({"id":"manual"}))
            .unwrap();
        assert_eq!(preset["preset"]["params"]["holding"], 1.0);
        assert_eq!(preset["frame"], 137);
        assert_eq!(callback.performance.snapshot(), before_live);
        assert_eq!(session.snapshot(), before_session);
        let (mut reopened, _, _) = sequence_callback(&captured.patch);
        without_heap_activity(|| reopened.render(&mut [0.0_f32; 1], 1));
        assert!(!reopened.performance.sequence_state("seq").unwrap().running);
        assert_eq!(
            reopened.performance.snapshot()["modules"]["manual"]["state"]["holding"],
            true
        );
        assert_eq!(
            reopened
                .performance
                .module_control_state("manual")
                .unwrap()
                .pressed_mask,
            0
        );
        session
            .execute(
                "duplicate_modules",
                serde_json::json!({"modules":[{"id":"manual","new_id":"copy"}]}),
            )
            .unwrap();
        assert!(session.capture_project_patch(None).is_err());
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        assert_eq!(
            callback.performance.snapshot()["modules"]["copy"]["state"]["holding"],
            true
        );
        assert_eq!(
            callback
                .performance
                .module_control_state("copy")
                .unwrap()
                .pressed_mask,
            0
        );
        assert_eq!(
            callback
                .performance
                .module_control_state("manual")
                .unwrap()
                .pressed_mask,
            1
        );
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("manual", "gate"),
                ("copy", "gate")
            ),
            [0.0, 10.0]
        );
        session.execute("undo", serde_json::json!({})).unwrap();
        session.execute("redo", serde_json::json!({})).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        while disposal.pop().is_ok() {}
        assert_eq!(
            callback.performance.snapshot()["modules"]["copy"]["state"]["holding"],
            true
        );
        assert_eq!(
            callback
                .performance
                .module_control_state("copy")
                .unwrap()
                .pressed_mask,
            0
        );
        // The immutable captured preset remains true after the live latch changes.
        for action in [ModuleAction::HoldPress, ModuleAction::HoldRelease] {
            callback.performance.trigger("manual", action).unwrap();
        }
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
            false
        );
        session
            .execute(
                "save_module_preset",
                serde_json::json!({"filename":"held.opreset","preset":preset["preset"]}),
            )
            .unwrap();
        let saved = session
            .execute(
                "read_module_preset",
                serde_json::json!({"filename":"held.opreset"}),
            )
            .unwrap();
        assert_eq!(saved["preset"]["params"]["holding"], 1.0);
    }

    #[test]
    fn push_preset_loads_assign_equal_values_and_partial_history_preserves_latch_and_presence() {
        for explicit in [false, true] {
            for partial in [false, true] {
                let mut patch = manual_patch("push");
                if !explicit {
                    patch.nodes[1].params.remove("holding");
                }
                let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
                let workspace = tempfile::tempdir().unwrap();
                let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
                session.replace_patch(patch, Some(0)).unwrap();
                session
                    .set_live_module_events(Some(callback.performance.clone()))
                    .unwrap();
                for action in [ModuleAction::HoldPress, ModuleAction::HoldRelease] {
                    callback.performance.trigger("manual", action).unwrap();
                }
                without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
                session.execute("apply_module_preset", serde_json::json!({"id":"manual","preset":{
                    "format":"openpatch-module-preset","version":1,"kind":"push",
                    "params":if partial {serde_json::json!({})} else {serde_json::json!({"holding":0})}
                }})).unwrap();
                assert!(updates
                    .push(sequence_graph(
                        &callback,
                        session.patch(),
                        session.revision()
                    ))
                    .is_ok());
                without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
                while disposal.pop().is_ok() {}
                assert_eq!(
                    callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
                    partial
                );
                if partial {
                    assert_eq!(
                        session.patch().nodes[1].params.contains_key("holding"),
                        explicit
                    );
                    for action in [ModuleAction::HoldPress, ModuleAction::HoldRelease] {
                        callback.performance.trigger("manual", action).unwrap();
                    }
                    without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
                }
                session.execute("undo", serde_json::json!({})).unwrap();
                assert!(updates
                    .push(sequence_graph(
                        &callback,
                        session.patch(),
                        session.revision()
                    ))
                    .is_ok());
                without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
                while disposal.pop().is_ok() {}
                assert_eq!(
                    callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
                    true
                );
                assert_eq!(
                    session.patch().nodes[1].params.contains_key("holding"),
                    explicit
                );
                session.execute("redo", serde_json::json!({})).unwrap();
                assert!(updates
                    .push(sequence_graph(
                        &callback,
                        session.patch(),
                        session.revision()
                    ))
                    .is_ok());
                without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
                assert_eq!(
                    callback.performance.snapshot()["modules"]["manual"]["state"]["holding"],
                    partial
                );
            }
        }
    }

    #[test]
    fn read_only_signal_module_lights_are_published_without_sequence_state() {
        for (kind, count) in [
            ("compare", 4),
            ("rescale", 2),
            ("vca_mix", 4),
            ("poly_sum", 6),
        ] {
            let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
                "nodes":[{"id":"meter","kind":kind}],"output":"meter",
                "output_port":match kind {"compare" => "max", "vca_mix" => "mix", _ => "out"}}))
            .unwrap();
            let (mut callback, _, _) = sequence_callback(&patch);
            assert!(callback
                .performance
                .trigger("meter", ModuleAction::BPress)
                .is_err());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 64], 1));
            assert_eq!(
                callback.performance.module_lights("meter").unwrap().count,
                count
            );
            assert!(callback.performance.module_control_state("meter").is_none());
            assert!(callback.performance.sequence_state("meter").is_none());
            assert_eq!(
                callback.performance.snapshot()["modules"]["meter"]["kind"],
                kind
            );
        }
    }

    include!("sum_audio_tests.rs");
    include!("merge_audio_tests.rs");
    include!("split_audio_tests.rs");
    include!("viz_audio_tests.rs");
    include!("rack_scope_audio_tests.rs");

    #[test]
    fn mixer_meters_follow_audio_revisions_bypass_and_document_identity_without_heap_work() {
        let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"source","kind":"constant","params":{"value":0.5}},
                     {"id":"mixer","kind":"vca_mix"}],
            "cables":[{"from":"source","to":"mixer","input":"in1"}],
            "output":"mixer","output_port":"mix"}))
        .unwrap();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        assert!(callback.performance.module_lights("mixer").is_none());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 512], 1));
        let original = callback.performance.module_lights("mixer").unwrap();
        assert_eq!(original.count, 4);
        assert!((original.brightness[0][0] - 0.668_383_36).abs() < 1e-5);
        assert_eq!(original.brightness[0][1], 0.0);
        assert_eq!(&original.brightness[1..4], &[[0.0; 2]; 3]);
        let token = callback.graphs.active.modules.bindings[0].unwrap().token;

        session
            .execute(
                "set_parameter",
                serde_json::json!({
            "id":"mixer","parameter":"level1","value":0.5}),
            )
            .unwrap();
        assert!(callback.performance.module_lights("mixer").is_none());
        // An old callback publication must not masquerade as the new revision.
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert!(callback.performance.module_lights("mixer").is_none());
        let graph = sequence_graph(&callback, session.patch(), session.revision());
        assert_eq!(graph.modules.bindings[0].unwrap().token, token);
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        // Parameter edits retain the actual peak envelope and divider phase.
        assert_eq!(
            callback.performance.module_lights("mixer").unwrap(),
            original
        );
        assert_eq!(
            callback.performance.snapshot()["modules"]["mixer"]["revision"],
            session.revision()
        );
        without_heap_activity(|| callback.render(&mut [0.0_f32; 4096], 1));
        let decayed = callback.performance.module_lights("mixer").unwrap();
        assert!(decayed.brightness[0][0] < original.brightness[0][0]);
        assert!(decayed.brightness[0][0] > 0.0);

        session
            .execute(
                "set_bypass",
                serde_json::json!({"id":"mixer","bypassed":true}),
            )
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1024], 1));
        assert_eq!(
            callback.performance.module_lights("mixer").unwrap(),
            decayed
        );
        assert_eq!(
            callback.performance.snapshot()["modules"]["mixer"]["bypassed"],
            true
        );

        // Reopening a document, even with the same IDs, starts fresh meters.
        session
            .replace_patch(patch, Some(session.revision()))
            .unwrap();
        assert!(callback.performance.module_lights("mixer").is_none());
        let graph = sequence_graph(&callback, session.patch(), session.revision());
        assert_ne!(graph.modules.bindings[0].unwrap().token, token);
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            callback
                .performance
                .module_lights("mixer")
                .unwrap()
                .brightness,
            [[0.0; 2]; openpatch_core::MAX_MODULE_LIGHTS]
        );
    }

    fn read_raw_pair(engine: &mut Engine, first: (&str, &str), second: (&str, &str)) -> [f32; 2] {
        let taps = [
            Some(engine.resolve_tap(first.0, first.1, 0).unwrap()),
            Some(engine.resolve_tap(second.0, second.1, 0).unwrap()),
        ];
        let mut samples = [[TapSample::default(); 2]; 1];
        without_heap_activity(|| {
            engine.process_audio_block_with_taps(&mut [[0.0; 2]; 1], &taps, &mut samples)
        });
        assert!(samples[0][0].active && samples[0][1].active);
        [samples[0][0].value, samples[0][1].value]
    }

    #[test]
    fn duplicated_oscillators_restart_after_coalesced_undo_redo_while_original_phase_continues() {
        for version in [1, 2] {
            let patch: Patch = serde_json::from_value(serde_json::json!({"version":version,
                "nodes":[{"id":"osc","kind":"oscillator","params":{"frequency":123}}],"output":"osc"})).unwrap();
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            let workspace = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
            session.replace_patch(patch.clone(), Some(0)).unwrap();
            session
                .set_live_module_events(Some(callback.performance.clone()))
                .unwrap();
            let mut reference = patch.compile().unwrap();
            let mut block = [[0.0; 2]; 256];
            without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
            reference.process_audio_block(&mut block[..80]);
            session
                .execute(
                    "duplicate_modules",
                    serde_json::json!({"modules":[{"id":"osc","new_id":"copy"}]}),
                )
                .unwrap();
            assert!(updates
                .push(sequence_graph(
                    &callback,
                    session.patch(),
                    session.revision()
                ))
                .is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
            reference.process_audio_block(&mut block);
            let pair = read_raw_pair(
                &mut callback.graphs.active.engine,
                ("osc", "out"),
                ("copy", "out"),
            );
            reference.process_audio_block(&mut block[..1]);
            assert_eq!(pair[0], block[0][0]);
            let original_token = callback.graphs.active.modules.states[0].unwrap().token;
            let copy_token = callback.graphs.active.modules.states[1].unwrap().token;
            session.execute("undo", serde_json::json!({})).unwrap();
            session.execute("redo", serde_json::json!({})).unwrap();
            let stale = sequence_graph(&callback, session.patch(), 2);
            assert_eq!(stale.modules.states[0].unwrap().token, original_token);
            assert!(stale.modules.states[1].is_none());
            assert_eq!(stale.modules.fresh_states.iter().flatten().count(), 1);
            assert!(updates.push(stale).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
            reference.process_audio_block(&mut block);
            let pair = read_raw_pair(
                &mut callback.graphs.active.engine,
                ("osc", "out"),
                ("copy", "out"),
            );
            reference.process_audio_block(&mut block[..1]);
            assert_eq!(pair[0], block[0][0]);
            let mut stale_fresh = patch.compile().unwrap();
            stale_fresh.process_audio_block(&mut block);
            stale_fresh.process_audio_block(&mut block[..1]);
            assert_eq!(pair[1], block[0][0]);
            let redone = sequence_graph(&callback, session.patch(), session.revision());
            assert_eq!(redone.modules.states[0].unwrap().token, original_token);
            assert_ne!(redone.modules.states[1].unwrap().token, copy_token);
            assert!(updates.push(redone).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
            reference.process_audio_block(&mut block[..1]);
            let pair = read_raw_pair(
                &mut callback.graphs.active.engine,
                ("osc", "out"),
                ("copy", "out"),
            );
            reference.process_audio_block(&mut block[..1]);
            assert_eq!(pair[0], block[0][0]);
            let mut fresh = patch.compile().unwrap();
            fresh.process_audio_block(&mut block[..1]);
            fresh.process_audio_block(&mut block[..1]);
            assert_eq!(pair[1], block[0][0]);
            assert_ne!(pair[0], pair[1]);
        }
    }

    #[test]
    fn maximum_graph_mixed_stale_identities_remain_fresh_until_current_publication() {
        let mut nodes = vec![serde_json::json!({"id":"seq","kind":"seq3","params":{"tempo":15}})];
        nodes.extend((1..openpatch_core::MAX_NODES).map(|index| {
            serde_json::json!({
            "id":format!("osc{index}"),"kind":"oscillator","params":{"frequency":123}})
        }));
        let patch: Patch =
            serde_json::from_value(serde_json::json!({"version":2,"nodes":nodes,"output":"osc1"}))
                .unwrap();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
        let mut removed = patch.clone();
        removed.nodes.truncate(openpatch_core::MAX_NODES / 2);
        callback.performance.sync_patch(&removed, 2);
        callback.performance.sync_patch(&patch, 3);
        let stale = sequence_graph(&callback, &patch, 1);
        assert_eq!(stale.modules.states.iter().flatten().count(), 128);
        assert_eq!(stale.modules.fresh_states.iter().flatten().count(), 128);
        assert!(updates.push(stale).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let pair = read_raw_pair(
            &mut callback.graphs.active.engine,
            ("osc1", "out"),
            ("osc255", "out"),
        );
        let single: Patch = serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"osc","kind":"oscillator","params":{"frequency":123}}],"output":"osc"}))
        .unwrap();
        let mut reference = single.compile().unwrap();
        let mut source_frames = [[0.0; 2]; 337];
        reference.process_audio_block(&mut source_frames);
        assert_eq!(pair[0], source_frames[336][0]);
        let mut fresh = single.compile().unwrap();
        let mut fresh_frames = [[0.0; 2]; 257];
        fresh.process_audio_block(&mut fresh_frames);
        assert_eq!(pair[1], fresh_frames[256][0]);
        assert!(callback.performance.sequence_state("seq").is_none());
        assert_eq!(
            callback.performance.snapshot()["modules"]["seq"]["available"],
            false
        );
        assert!(updates.push(sequence_graph(&callback, &patch, 3)).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        let pair = read_raw_pair(
            &mut callback.graphs.active.engine,
            ("osc1", "out"),
            ("osc255", "out"),
        );
        reference.process_audio_block(&mut source_frames[..2]);
        assert_eq!(pair[0], source_frames[1][0]);
        let mut restarted = single.compile().unwrap();
        restarted.process_audio_block(&mut fresh_frames[..2]);
        assert_eq!(pair[1], fresh_frames[1][0]);
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        assert_eq!(
            callback.performance.snapshot()["modules"]["seq"]["revision"],
            3
        );
    }

    #[test]
    fn duplicated_midi_modules_are_fresh_after_coalesced_history_and_document_replacement() {
        let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"midi","kind":"midi_cv","params":{"channels":1}}],
            "output":"midi","output_port":"gate"}))
        .unwrap();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        callback.graphs.active.engine.handle_midi(note_on(60));
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        session
            .execute(
                "duplicate_modules",
                serde_json::json!({"modules":[{"id":"midi","new_id":"copy"}]}),
            )
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("midi", "gate"),
                ("copy", "gate")
            ),
            [10.0, 0.0]
        );
        callback.graphs.active.engine.handle_midi(note_on(62));
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("midi", "gate"),
                ("copy", "gate")
            ),
            [10.0, 10.0]
        );
        session.execute("undo", serde_json::json!({})).unwrap();
        session.execute("redo", serde_json::json!({})).unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("midi", "gate"),
                ("copy", "gate")
            ),
            [10.0, 0.0]
        );
        session
            .replace_patch(session.patch().clone(), None)
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(
            read_raw_pair(
                &mut callback.graphs.active.engine,
                ("midi", "gate"),
                ("copy", "gate")
            ),
            [0.0, 0.0]
        );
    }

    fn stacked_history_patch() -> Patch {
        serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"osc","kind":"oscillator","params":{"frequency":123}},
                {"id":"b","kind":"constant","params":{"value":3}}, {"id":"sum","kind":"gain"}],
            "cables":[{"id":"route_a","from":"osc","to":"sum","input":"signal"},
                {"id":"route_b","from":"b","to":"sum","input":"signal"}],"output":"sum"}))
        .unwrap()
    }
    fn scoped_route_graph(
        callback: &Callback,
        patch: &Patch,
        revision: u64,
        generation: u64,
    ) -> PreparedGraph {
        let mut graph = sequence_graph(callback, patch, revision);
        graph.scope = ScopeSetup {
            generation,
            enabled: true,
            taps: [
                Some(graph.engine.resolve_tap("sum", "out", 0).unwrap()),
                Some(graph.engine.resolve_tap("osc", "out", 0).unwrap()),
            ],
        };
        graph
    }
    fn oscillator_reference() -> Engine {
        let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"osc","kind":"oscillator","params":{"frequency":123}}],"output":"osc"}))
        .unwrap();
        patch.compile().unwrap()
    }

    #[test]
    fn plug_promotion_and_history_preserve_callback_phase_delay_and_tokens() {
        let patch = stacked_history_patch();
        let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
        let mut packets = callback.scope_packets.take().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
        let nodes = callback
            .graphs
            .active
            .modules
            .states
            .map(|binding| binding.map(|binding| binding.token));
        let cables: Vec<_> = callback
            .graphs
            .active
            .modules
            .cables
            .iter()
            .map(|binding| binding.map(|binding| binding.token))
            .collect();
        let mut reference = oscillator_reference();
        let mut warmup = [[0.0; 2]; 80];
        reference.process_audio_block(&mut warmup);
        let mut preceding = warmup[79][0];
        for (index, (tool, args)) in [
            ("promote_cables", serde_json::json!({"cable_ids":["route_a"],"end":"input"})),
            ("undo", serde_json::json!({})),
            ("redo", serde_json::json!({})),
            ("move_cables", serde_json::json!({"cable_ids":["route_b"],"end":"input","node":"sum","port":"signal","promote":true})),
        ].into_iter().enumerate() {
            session.execute(tool, args).unwrap();
            let graph = scoped_route_graph(&callback, session.patch(), session.revision(), index as u64 + 2);
            assert_eq!(graph.modules.states.map(|binding| binding.map(|binding| binding.token)), nodes);
            assert_eq!(graph.modules.cables.iter().map(|binding| binding.map(|binding| binding.token)).collect::<Vec<_>>(), cables);
            assert!(updates.push(graph).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
            let mut expected = [[0.0; 2]; 256];
            reference.process_audio_block(&mut expected);
            for packet_index in 0..256 / SCOPE_PACKET_SIZE {
                let packet = packets.pop().unwrap();
                for sample in 0..SCOPE_PACKET_SIZE {
                    let frame = packet_index * SCOPE_PACKET_SIZE + sample;
                    let delayed = if frame == 0 { preceding } else { expected[frame - 1][0] };
                    assert_eq!(packet.y[sample], expected[frame][0], "{tool}: oscillator phase");
                    assert_eq!(packet.x[sample], delayed + 3.0, "{tool}: summed delayed frame");
                }
            }
            preceding = expected[255][0];
            while packets.pop().is_ok() {}
            while disposal.pop().is_ok() {}
        }
    }

    #[test]
    fn outgoing_duplicate_preserves_original_phase_and_delays_and_restarts_only_copy_on_redo() {
        for source in ["b", "osc"] {
            let patch = stacked_history_patch();
            let (mut callback, mut updates, mut disposal) = sequence_callback(&patch);
            let mut packets = callback.scope_packets.take().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
            session.replace_patch(patch, Some(0)).unwrap();
            session
                .set_live_module_events(Some(callback.performance.clone()))
                .unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
            let original_routes = [
                callback.graphs.active.modules.cables[0].unwrap().token,
                callback.graphs.active.modules.cables[1].unwrap().token,
            ];
            let mut reference = oscillator_reference();
            let mut prior = [[0.0; 2]; 80];
            reference.process_audio_block(&mut prior);
            let mut last_original = prior[79][0];
            session
                .execute(
                    "duplicate_modules",
                    serde_json::json!({
                        "modules":[{"id":source,"new_id":"copy"}],"cable_mode":"all"
                    }),
                )
                .unwrap();
            for generation in [2, 3] {
                if generation == 3 {
                    // The callback never sees the intermediate removal. Both
                    // the recreated module and its route must nevertheless
                    // start fresh, without clearing the original routes.
                    session.execute("undo", serde_json::json!({})).unwrap();
                    session.execute("redo", serde_json::json!({})).unwrap();
                }
                let graph =
                    scoped_route_graph(&callback, session.patch(), session.revision(), generation);
                for (slot, token) in original_routes.iter().enumerate() {
                    assert_eq!(graph.modules.cables[slot].unwrap().token, *token);
                }
                assert!(updates.push(graph).is_ok());
                without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
                let packet = packets.pop().unwrap();
                let mut original = [[0.0; 2]; 256];
                reference.process_audio_block(&mut original);
                let mut fresh = oscillator_reference();
                let mut copied = [[0.0; 2]; SCOPE_PACKET_SIZE];
                fresh.process_audio_block(&mut copied);
                for index in 0..SCOPE_PACKET_SIZE {
                    let original_delay = if index == 0 {
                        last_original
                    } else {
                        original[index - 1][0]
                    };
                    let copy_delay = if index == 0 {
                        0.0
                    } else if source == "b" {
                        3.0
                    } else {
                        copied[index - 1][0]
                    };
                    assert_eq!(packet.y[index], original[index][0]);
                    assert_eq!(packet.x[index], original_delay + 3.0 + copy_delay);
                    assert!(packet.x_active[index] && packet.y_active[index]);
                }
                last_original = original[255][0];
                while packets.pop().is_ok() {}
                while disposal.pop().is_ok() {}
            }
        }
    }

    #[test]
    fn disconnect_and_clear_reset_removed_route_delays_without_resetting_source_state() {
        for (clear_all, through_history) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let patch = stacked_history_patch();
            let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
            let mut packets = callback.scope_packets.take().unwrap();
            let workspace = tempfile::tempdir().unwrap();
            let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
            session.replace_patch(patch, Some(0)).unwrap();
            session
                .set_live_module_events(Some(callback.performance.clone()))
                .unwrap();
            without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
            let mut reference = oscillator_reference();
            let mut expected = [[0.0; 2]; 80];
            reference.process_audio_block(&mut expected);
            let nodes = callback
                .graphs
                .active
                .modules
                .states
                .map(|binding| binding.map(|binding| binding.token));
            let old_a = callback.graphs.active.modules.cables[0].unwrap().token;
            let old_b = callback.graphs.active.modules.cables[1].unwrap().token;
            if through_history {
                session
                    .execute(
                        if clear_all {
                            "clear_cables"
                        } else {
                            "disconnect"
                        },
                        if clear_all {
                            serde_json::json!({})
                        } else {
                            serde_json::json!({"cable_id":"route_a"})
                        },
                    )
                    .unwrap();
                session.execute("undo", serde_json::json!({})).unwrap();
            } else {
                let mut edits = vec![
                    if clear_all {
                        serde_json::json!({"tool":"clear_cables","arguments":{}})
                    } else {
                        serde_json::json!({"tool":"disconnect","arguments":{"cable_id":"route_a"}})
                    },
                    serde_json::json!({"tool":"connect","arguments":{"from":"osc","to":"sum","input":"signal","mode":"stack","cable_id":"route_a"}}),
                ];
                if clear_all {
                    edits.push(serde_json::json!({"tool":"connect","arguments":{"from":"b","to":"sum","input":"signal","mode":"stack","cable_id":"route_b"}}));
                }
                session
                    .execute("apply_edits", serde_json::json!({"edits":edits}))
                    .unwrap();
            }
            let graph = scoped_route_graph(&callback, session.patch(), session.revision(), 2);
            assert_eq!(
                graph
                    .modules
                    .states
                    .map(|binding| binding.map(|binding| binding.token)),
                nodes
            );
            assert_ne!(graph.modules.cables[0].unwrap().token, old_a);
            assert_eq!(graph.modules.cables[1].unwrap().token == old_b, !clear_all);
            assert!(updates.push(graph).is_ok());
            without_heap_activity(|| callback.render(&mut [0.0_f32; SCOPE_PACKET_SIZE], 1));
            let packet = packets.pop().unwrap();
            assert_eq!(packet.x[0], if clear_all { 0.0 } else { 3.0 });
            reference.process_audio_block(&mut expected[..SCOPE_PACKET_SIZE]);
            for (index, expected_frame) in expected.iter().take(SCOPE_PACKET_SIZE).enumerate() {
                assert!(packet.x_active[index] && packet.y_active[index]);
                assert_eq!(packet.y[index], expected_frame[0]);
                if index > 0 {
                    assert_eq!(packet.x[index], 3.0 + packet.y[index - 1]);
                }
            }
        }
    }

    #[test]
    fn stale_cable_plan_and_latest_plan_each_start_recreated_route_fresh_without_resetting_nodes() {
        let patch = stacked_history_patch();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let mut packets = callback.scope_packets.take().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
        session
            .execute("disconnect", serde_json::json!({"cable_id":"route_a"}))
            .unwrap();
        session.execute("undo", serde_json::json!({})).unwrap();
        let stale = scoped_route_graph(&callback, session.patch(), 1, 2);
        assert!(stale.modules.cables[0].is_none());
        assert_eq!(stale.modules.fresh_cables.iter().flatten().count(), 1);
        assert!(updates.push(stale).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let first = packets.pop().unwrap();
        assert_eq!(first.x[0], 3.0);
        while packets.pop().is_ok() {}
        let current = scoped_route_graph(&callback, session.patch(), session.revision(), 3);
        assert!(updates.push(current).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; SCOPE_PACKET_SIZE], 1));
        let current = packets.pop().unwrap();
        assert_eq!(current.x[0], 3.0);
        let mut reference = oscillator_reference();
        let mut prior = [[0.0; 2]; 336];
        reference.process_audio_block(&mut prior);
        let mut expected = [[0.0; 2]; SCOPE_PACKET_SIZE];
        reference.process_audio_block(&mut expected);
        for (index, expected_frame) in expected.iter().enumerate() {
            assert_eq!(current.y[index], expected_frame[0]);
        }
    }

    #[test]
    fn cable_reordering_preserves_histories_and_endpoint_changes_reset_just_that_cable() {
        let patch = stacked_history_patch();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let mut packets = callback.scope_packets.take().unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
        let old_a = callback.graphs.active.modules.cables[0].unwrap().token;
        let old_b = callback.graphs.active.modules.cables[1].unwrap().token;
        let mut reordered = patch.clone();
        reordered.cables.reverse();
        callback.performance.sync_patch(&reordered, 2);
        let graph = scoped_route_graph(&callback, &reordered, 2, 2);
        assert_eq!(graph.modules.cables[0].unwrap().token, old_a);
        assert_eq!(graph.modules.cables[1].unwrap().token, old_b);
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        let first = packets.pop().unwrap();
        let mut reference = oscillator_reference();
        let mut expected = [[0.0; 2]; 80];
        reference.process_audio_block(&mut expected);
        assert_eq!(first.x[0], 3.0 + expected[79][0]);
        while packets.pop().is_ok() {}
        reordered.cables[1].from = "b".into();
        callback.performance.sync_patch(&reordered, 3);
        let graph = scoped_route_graph(&callback, &reordered, 3, 3);
        assert_ne!(graph.modules.cables[0].unwrap().token, old_a);
        assert_eq!(graph.modules.cables[1].unwrap().token, old_b);
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; SCOPE_PACKET_SIZE], 1));
        let changed = packets.pop().unwrap();
        assert_eq!(changed.x[0], 3.0);
        assert_eq!(changed.x[1..], [6.0; SCOPE_PACKET_SIZE - 1]);
    }

    #[test]
    fn maximum_stacked_cables_adopt_mixed_bound_and_stale_histories_without_heap() {
        let cables: Vec<_> = (0..openpatch_core::MAX_CABLES)
            .map(|index| {
                serde_json::json!({
            "id":format!("route_{index}"),"from":"one","to":"sum","input":"signal"})
            })
            .collect();
        let patch: Patch = serde_json::from_value(serde_json::json!({"version":2,
            "nodes":[{"id":"one","kind":"constant","params":{"value":1}}, {"id":"sum","kind":"gain"}],
            "cables":cables,"output":"sum"})).unwrap();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let mut packets = callback.scope_packets.take().unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 2], 1));
        let mut removed = patch.clone();
        removed.cables.truncate(openpatch_core::MAX_CABLES / 2);
        callback.performance.sync_patch(&removed, 2);
        callback.performance.sync_patch(&patch, 3);
        for (revision, frames) in [(1, 256), (3, SCOPE_PACKET_SIZE)] {
            let mut graph = sequence_graph(&callback, &patch, revision);
            if revision == 1 {
                assert_eq!(graph.modules.cables.iter().flatten().count(), 512);
                assert_eq!(graph.modules.fresh_cables.iter().flatten().count(), 512);
            }
            graph.scope = ScopeSetup {
                generation: revision,
                enabled: true,
                taps: [
                    Some(graph.engine.resolve_tap("sum", "out", 0).unwrap()),
                    None,
                ],
            };
            assert!(updates.push(graph).is_ok());
            let mut output = [0.0_f32; 256];
            without_heap_activity(|| callback.render(&mut output[..frames], 1));
            let first = packets.pop().unwrap();
            assert_eq!(first.x[0], 512.0);
            assert_eq!(first.x[1..], [1024.0; SCOPE_PACKET_SIZE - 1]);
            assert_eq!(first.x_active, [true; SCOPE_PACKET_SIZE]);
            while packets.pop().is_ok() {}
        }
    }

    #[test]
    fn full_document_replacement_clears_all_route_histories_and_source_state() {
        let patch = stacked_history_patch();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let mut packets = callback.scope_packets.take().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
        session.replace_patch(patch, Some(1)).unwrap();
        assert!(updates
            .push(scoped_route_graph(
                &callback,
                session.patch(),
                session.revision(),
                2
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; SCOPE_PACKET_SIZE], 1));
        let packet = packets.pop().unwrap();
        assert_eq!(packet.x[0], 0.0);
        assert_eq!(packet.y[0], 0.0);
        let mut reference = oscillator_reference();
        let mut expected = [[0.0; 2]; SCOPE_PACKET_SIZE];
        reference.process_audio_block(&mut expected);
        for (index, expected_frame) in expected.iter().take(SCOPE_PACKET_SIZE).enumerate() {
            assert_eq!(packet.y[index], expected_frame[0]);
            if index > 0 {
                assert_eq!(packet.x[index], 3.0 + packet.y[index - 1]);
            }
        }
    }

    #[test]
    fn undo_redo_of_already_rendered_same_shape_cable_replacement_replays_delay_reset() {
        let mut patch = stacked_history_patch();
        // A recreated route_b receives rank 1 above the legacy unranked route_a.
        patch.cables[1].input_order = Some(1);
        patch.cables[1].output_order = Some(1);
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let mut packets = callback.scope_packets.take().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 80], 1));
        let mut reference = oscillator_reference();
        let mut expected = [[0.0; 2]; 256];
        reference.process_audio_block(&mut expected[..80]);
        let preceding_source = expected[79][0];
        session.execute("apply_edits",serde_json::json!({"edits":[
            {"tool":"disconnect","arguments":{"cable_id":"route_b"}},
            {"tool":"connect","arguments":{"from":"b","to":"sum","input":"signal","mode":"stack","cable_id":"route_b"}}
        ]})).unwrap();
        assert_eq!(
            serde_json::to_value(session.patch()).unwrap(),
            serde_json::to_value(&patch).unwrap()
        );
        assert!(updates
            .push(scoped_route_graph(
                &callback,
                session.patch(),
                session.revision(),
                2
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(packets.pop().unwrap().x[0], preceding_source);
        while packets.pop().is_ok() {}
        reference.process_audio_block(&mut expected);
        let preceding_source = expected[255][0];
        let nodes = callback
            .graphs
            .active
            .modules
            .states
            .map(|binding| binding.map(|binding| binding.token));
        let surviving = callback.graphs.active.modules.cables[0].unwrap().token;
        let replaced = callback.graphs.active.modules.cables[1].unwrap().token;
        session.execute("undo", serde_json::json!({})).unwrap();
        session.execute("redo", serde_json::json!({})).unwrap();
        let graph = scoped_route_graph(&callback, session.patch(), session.revision(), 3);
        assert_eq!(
            graph
                .modules
                .states
                .map(|binding| binding.map(|binding| binding.token)),
            nodes
        );
        assert_eq!(graph.modules.cables[0].unwrap().token, surviving);
        assert_ne!(graph.modules.cables[1].unwrap().token, replaced);
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; SCOPE_PACKET_SIZE], 1));
        let packet = packets.pop().unwrap();
        assert_eq!(packet.x[0], preceding_source);
        reference.process_audio_block(&mut expected[..SCOPE_PACKET_SIZE]);
        for (index, expected_frame) in expected.iter().take(SCOPE_PACKET_SIZE).enumerate() {
            assert_eq!(packet.y[index], expected_frame[0]);
            if index > 0 {
                assert_eq!(packet.x[index], 3.0 + packet.y[index - 1]);
            }
        }
    }

    #[test]
    fn undo_redo_of_already_rendered_same_shape_module_replacement_replays_dsp_reset() {
        let mut patch = sequence_patch();
        // Match fresh-connection presentation ranks so replacement remains
        // byte-identical while runtime incarnation effects are exercised.
        patch.cables[0].input_order = Some(1);
        patch.cables[0].output_order = Some(1);
        patch.nodes.swap(0, 1);
        let parameters = patch.nodes[1].params.clone();
        let (mut callback, mut updates, _disposal) = sequence_callback(&patch);
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch.clone(), Some(0)).unwrap();
        session
            .set_live_module_events(Some(callback.performance.clone()))
            .unwrap();
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        session.execute("apply_edits",serde_json::json!({"edits":[
            {"tool":"remove_module","arguments":{"id":"seq"}},
            {"tool":"add_module","arguments":{"id":"seq","kind":"seq3","params":parameters}},
            {"tool":"connect","arguments":{"from":"zero","to":"seq","input":"clock","cable_id":"cable_1"}},
            {"tool":"set_output","arguments":{"id":"seq","port":"cv1"}}
        ]})).unwrap();
        assert_eq!(
            serde_json::to_value(session.patch()).unwrap(),
            serde_json::to_value(&patch).unwrap()
        );
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        assert!(updates
            .push(sequence_graph(
                &callback,
                session.patch(),
                session.revision()
            ))
            .is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 256], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 2);
        let original_zero = callback.graphs.active.modules.states[0].unwrap().token;
        let replaced_seq = callback.graphs.active.modules.states[1].unwrap().token;
        session.execute("undo", serde_json::json!({})).unwrap();
        session.execute("redo", serde_json::json!({})).unwrap();
        let graph = sequence_graph(&callback, session.patch(), session.revision());
        assert_eq!(graph.modules.states[0].unwrap().token, original_zero);
        assert_ne!(graph.modules.states[1].unwrap().token, replaced_seq);
        assert!(updates.push(graph).is_ok());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 1);
    }

    #[test]
    fn future_graph_actions_wait_then_expire_with_visible_diagnostics() {
        let mut patch = sequence_patch();
        let (mut callback, _, _) = sequence_callback(&patch);
        patch.nodes[0].params.insert("row1_2".into(), 2.5);
        callback.performance.sync_patch(&patch, 2);
        callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| callback.render(&mut [0.0_f32; 128], 1));
        assert_eq!(callback.performance.snapshot()["applied"], 0);
        callback.frame_clock = 96_001;
        without_heap_activity(|| callback.render(&mut [0.0_f32; 1], 1));
        assert_eq!(callback.performance.snapshot()["expired"], 1);
        assert_eq!(callback.performance.snapshot()["pending"], 0);
        assert_eq!(callback.performance.snapshot()["dropped"], 1);
    }

    #[test]
    fn module_queue_flood_has_backpressure_and_spreads_actions_over_core_sample_budgets() {
        let (mut callback, _, _) = sequence_callback(&sequence_patch());
        for _ in 0..EVENT_CAPACITY {
            callback
                .performance
                .trigger("seq", ModuleAction::Clock)
                .unwrap();
        }
        assert!(callback
            .performance
            .trigger("seq", ModuleAction::Clock)
            .is_err());
        assert!(callback
            .performance
            .trigger("missing", ModuleAction::Run)
            .is_err());
        without_heap_activity(|| callback.render(&mut [0.0_f32; 16], 1));
        let status = callback.performance.snapshot();
        assert_eq!(status["queued"], EVENT_CAPACITY);
        assert_eq!(status["applied"], EVENT_CAPACITY);
        assert_eq!(status["backpressure"], 1);
        assert_eq!(status["rejected"], 1);
        assert_eq!(status["pending"], 0);
        assert_eq!(status["dropped"], 0);
        assert_eq!(callback.performance.sequence_state("seq").unwrap().step, 1);
    }

    #[test]
    fn multiple_control_producers_are_ordered_and_device_close_rejects_retained_senders() {
        let (mut callback, _, _) = sequence_callback(&sequence_patch());
        let sender = callback.performance.clone();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let sender = sender.clone();
                scope.spawn(move || {
                    for _ in 0..64 {
                        sender.trigger("seq", ModuleAction::Run).unwrap();
                    }
                });
            }
        });
        without_heap_activity(|| callback.render(&mut [0.0_f32; 16], 1));
        assert_eq!(sender.snapshot()["applied"], 256);
        assert_eq!(sender.snapshot()["last_applied_sequence"], 256);
        assert!(sender.sequence_state("seq").unwrap().running);
        sender.trigger("seq", ModuleAction::Clock).unwrap();
        sender.close();
        drop(callback);
        sender.finish_close();
        assert!(sender.trigger("seq", ModuleAction::Run).is_err());
        assert!(sender.sequence_state("seq").is_none());
        let status = sender.snapshot();
        assert_eq!(status["available"], false);
        assert_eq!(status["closed_dropped"], 1);
        assert_eq!(status["pending"], 0);
        let (mut reopened, _, _) = sequence_callback(&sequence_patch());
        reopened
            .performance
            .trigger("seq", ModuleAction::Clock)
            .unwrap();
        without_heap_activity(|| reopened.render(&mut [0.0_f32; 1], 1));
        assert_eq!(reopened.performance.sequence_state("seq").unwrap().step, 2);
        assert_eq!(sender.snapshot()["applied"], 256);
    }
    include!("random_values_host_tests.rs");
    include!("shift_register_host_tests.rs");
    include!("adsr_eg_host_tests.rs");
    include!("midi_cc_host_tests.rs");
    include!("midi_gate_host_tests.rs");
    include!("stock_vcf_audio_tests.rs");
    include!("stock_vca_audio_tests.rs");
    include!("stock_delay_audio_tests.rs");
    include!("stock_lfo_audio_tests.rs");
    include!("stock_vco_audio_tests.rs");
    include!("stock_seq3_audio_tests.rs");
    include!("stock_quantizer_audio_tests.rs");
    include!("stock_octave_audio_tests.rs");
    include!("stock_vca_mix_audio_tests.rs");
    include!("stock_process_audio_tests.rs");
    include!("random_audio_tests.rs");
    include!("port_light_host_tests.rs");
    include!("host_lifecycle_audio_tests.rs");
}

#[cfg(test)]
#[path = "audio_profile.rs"]
mod profile;
