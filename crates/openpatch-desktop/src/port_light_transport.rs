//! Passive endpoint telemetry. Registry/reader work happens off the callback;
//! the producer only copies fixed storage and performs a triple-buffer exchange.
use openpatch_core::{Engine, Patch, PortDirection, PortLightState, ResolvedPortLight};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    sync::{
        atomic::{AtomicU64, Ordering as AtomicOrdering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use triple_buffer::{Input, Output};

pub(crate) const MAX_PORT_LIGHTS: usize = 2048;
const LEASE: Duration = Duration::from_millis(250);
static NEXT_TOKEN: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub(crate) struct PortLightKey {
    pub id: String,
    pub direction: PortDirection,
    pub port: String,
}
impl PortLightKey {
    pub(crate) fn new(
        id: impl Into<String>,
        direction: PortDirection,
        port: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            direction,
            port: port.into(),
        }
    }
    fn direction_rank(&self) -> u8 {
        match self.direction {
            PortDirection::Input => 0,
            PortDirection::Output => 1,
        }
    }
}
impl PartialEq for PortLightKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for PortLightKey {}
impl PartialOrd for PortLightKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for PortLightKey {
    fn cmp(&self, other: &Self) -> Ordering {
        (&self.id, self.direction_rank(), &self.port).cmp(&(
            &other.id,
            other.direction_rank(),
            &other.port,
        ))
    }
}

#[derive(Clone)]
struct Packet {
    token: u64,
    revision: u64,
    backend: u32,
    frame: u64,
    last_observed_frame: u64,
    sequence: u64,
    control_epoch: u64,
    count: usize,
    produced_at: Instant,
    states: Box<[Option<PortLightState>; MAX_PORT_LIGHTS]>,
}

/// Immutable coherent capture, allocated only by a non-realtime reader.
#[derive(Debug)]
pub(crate) struct PortLightFrame {
    /// Unique registry/backend instance; changes even for same-revision rebuilds.
    pub token: u64,
    pub revision: u64,
    /// Actual engine backend, 1 (legacy) or 2 (polyphonic).
    pub backend: u32,
    /// End of the callback which published this capture, in stream sample frames.
    pub frame: u64,
    /// Last actual seven-sample acquisition, in the same stream clock.
    pub last_observed_frame: u64,
    pub sequence: u64,
    /// Packed enable intent acknowledged by the callback; offline fixtures use 1.
    pub control_epoch: u64,
    pub ports: BTreeMap<PortLightKey, PortLightState>,
}

pub(crate) struct PortLightProducer {
    input: Input<Packet>,
    handles: Box<[ResolvedPortLight]>,
    sequence: u64,
    last_acquisition: Option<(u64, u64)>,
}
struct Consumer {
    output: Output<Packet>,
    keys: Box<[PortLightKey]>,
    token: u64,
    backend: u32,
    cached: Option<Arc<PortLightFrame>>,
    observed: Option<Instant>,
}
#[derive(Clone)]
pub(crate) struct PortLightReader(Arc<Mutex<Consumer>>);

/// Resolve only unique physically patched endpoints, including ports above 16.
/// This never creates a DSP consumer or modifies the graph's routing.
#[cfg(test)]
pub(crate) fn prepare(
    patch: &Patch,
    engine: &mut Engine,
) -> Result<(PortLightProducer, PortLightReader), String> {
    prepare_with_enabled(patch, engine, true)
}

pub(crate) fn prepare_with_enabled(
    patch: &Patch,
    engine: &mut Engine,
    enabled: bool,
) -> Result<(PortLightProducer, PortLightReader), String> {
    let (keys, handles) = registry(patch, engine)?;
    let token = NEXT_TOKEN.fetch_add(1, AtomicOrdering::Relaxed);
    let initial = Packet {
        token,
        revision: 0,
        backend: patch.version,
        frame: 0,
        last_observed_frame: 0,
        sequence: 0,
        control_epoch: 0,
        count: keys.len(),
        produced_at: Instant::now(),
        states: Box::new([None; MAX_PORT_LIGHTS]),
    };
    let (input, output) = triple_buffer::triple_buffer(&initial);
    engine.set_port_lights_enabled(enabled && !keys.is_empty());
    Ok((
        PortLightProducer {
            input,
            handles,
            sequence: 0,
            last_acquisition: None,
        },
        PortLightReader(Arc::new(Mutex::new(Consumer {
            output,
            keys,
            token,
            backend: patch.version,
            cached: None,
            observed: None,
        }))),
    ))
}

type PortLightRegistry = (Box<[PortLightKey]>, Box<[ResolvedPortLight]>);

fn registry(patch: &Patch, engine: &Engine) -> Result<PortLightRegistry, String> {
    let keys: BTreeSet<_> = patch
        .cables
        .iter()
        .flat_map(|cable| {
            [
                PortLightKey::new(&cable.from, PortDirection::Output, &cable.output),
                PortLightKey::new(&cable.to, PortDirection::Input, &cable.input),
            ]
        })
        .collect();
    if keys.len() > MAX_PORT_LIGHTS {
        return Err(format!(
            "Port-light registry exceeds {MAX_PORT_LIGHTS} physical endpoints"
        ));
    }
    let handles = keys
        .iter()
        .map(|key| engine.resolve_port_light(&key.id, key.direction, &key.port))
        .collect::<Result<Vec<_>, _>>()?
        .into_boxed_slice();
    Ok((
        keys.into_iter().collect::<Vec<_>>().into_boxed_slice(),
        handles,
    ))
}

/// Explicit offline fixture extraction from an engine the caller already enabled
/// and processed. This does not compile, run, or manufacture signal state.
pub(crate) fn offline_frame(
    patch: &Patch,
    engine: &Engine,
    revision: u64,
) -> Result<Arc<PortLightFrame>, String> {
    let (keys, handles) = registry(patch, engine)?;
    let observed = engine
        .port_light_observation_frame()
        .ok_or("Port lights have not acquired an actual sample")?;
    let mut states = vec![None; handles.len()];
    if !engine.copy_port_lights(&handles, &mut states) || states.iter().any(Option::is_none) {
        return Err("Port-light registry does not belong to this engine".into());
    }
    Ok(Arc::new(PortLightFrame {
        token: NEXT_TOKEN.fetch_add(1, AtomicOrdering::Relaxed),
        revision,
        backend: patch.version,
        frame: engine.port_light_processed_frames(),
        last_observed_frame: observed,
        sequence: 1,
        control_epoch: 1,
        ports: keys
            .into_vec()
            .into_iter()
            .zip(states.into_iter().map(Option::unwrap))
            .collect(),
    }))
}

impl PortLightProducer {
    /// Caller enforces the global publication deadline. No allocation, release,
    /// lock, strings, map access, or UI-clock smoothing occurs on this path.
    pub(crate) fn has_endpoints(&self) -> bool {
        !self.handles.is_empty()
    }
    #[cfg(test)]
    fn publish(&mut self, engine: &Engine, revision: u64, frame: u64) -> bool {
        self.publish_at(engine, revision, frame, Instant::now())
    }
    #[cfg(test)]
    fn publish_at(&mut self, engine: &Engine, revision: u64, frame: u64, now: Instant) -> bool {
        self.publish_epoch_at(engine, revision, frame, 1, now)
    }
    pub(crate) fn publish_epoch(
        &mut self,
        engine: &Engine,
        revision: u64,
        frame: u64,
        control_epoch: u64,
    ) -> bool {
        self.publish_epoch_at(engine, revision, frame, control_epoch, Instant::now())
    }
    fn publish_epoch_at(
        &mut self,
        engine: &Engine,
        revision: u64,
        frame: u64,
        control_epoch: u64,
        now: Instant,
    ) -> bool {
        if control_epoch & 1 == 0 {
            return false;
        }
        let Some(acquisition) = engine.port_light_observation_frame() else {
            return false;
        };
        if self.last_acquisition == Some((control_epoch, acquisition)) {
            return false;
        }
        let processed = engine.port_light_processed_frames();
        if acquisition > processed {
            return false;
        }
        let packet = self.input.input_buffer_mut();
        if !engine.copy_port_lights(&self.handles, &mut packet.states[..self.handles.len()])
            || packet.states[..self.handles.len()]
                .iter()
                .any(Option::is_none)
        {
            return false;
        }
        self.sequence = self.sequence.wrapping_add(1).max(1);
        packet.revision = revision;
        packet.frame = frame;
        packet.last_observed_frame = frame.saturating_sub(processed - acquisition);
        packet.sequence = self.sequence;
        packet.control_epoch = control_epoch;
        packet.produced_at = now;
        self.last_acquisition = Some((control_epoch, acquisition));
        self.input.publish();
        true
    }
}
impl PortLightReader {
    #[cfg(test)]
    pub(crate) fn read(&self, revision: u64) -> Option<Arc<PortLightFrame>> {
        self.read_at(revision, Instant::now())
    }
    #[cfg(test)]
    fn read_at(&self, revision: u64, now: Instant) -> Option<Arc<PortLightFrame>> {
        self.read_epoch_at(revision, 1, now)
    }
    pub(crate) fn read_epoch(
        &self,
        revision: u64,
        control_epoch: u64,
    ) -> Option<Arc<PortLightFrame>> {
        self.read_epoch_at(revision, control_epoch, Instant::now())
    }
    fn read_epoch_at(
        &self,
        revision: u64,
        control_epoch: u64,
        now: Instant,
    ) -> Option<Arc<PortLightFrame>> {
        if control_epoch & 1 == 0 {
            return None;
        }
        let mut consumer = self.0.lock().ok()?;
        consumer.output.update();
        let packet = consumer.output.output_buffer();
        if packet.sequence == 0
            || packet.token != consumer.token
            || packet.backend != consumer.backend
            || packet.control_epoch != control_epoch
            || packet.revision != revision
            || packet.count != consumer.keys.len()
            || packet.states[..packet.count].iter().any(Option::is_none)
            || now.saturating_duration_since(packet.produced_at) >= LEASE
        {
            return None;
        }
        if consumer.cached.as_ref().map(|old| old.sequence) != Some(packet.sequence) {
            let frame = Arc::new(PortLightFrame {
                token: packet.token,
                revision: packet.revision,
                backend: packet.backend,
                frame: packet.frame,
                last_observed_frame: packet.last_observed_frame,
                sequence: packet.sequence,
                control_epoch: packet.control_epoch,
                ports: consumer
                    .keys
                    .iter()
                    .cloned()
                    .zip(
                        packet.states[..packet.count]
                            .iter()
                            .copied()
                            .map(Option::unwrap),
                    )
                    .collect(),
            });
            consumer.cached = Some(frame);
            consumer.observed = Some(now);
        }
        // Re-reading the same packet must never renew its lease.
        if now.saturating_duration_since(consumer.observed?) >= LEASE {
            return None;
        }
        consumer.cached.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patch() -> Patch {
        serde_json::from_value(serde_json::json!({"version":2,"nodes":[
            {"id":"dc","kind":"constant","params":{"value":5}},
            {"id":"gain","kind":"gain"}],
            "cables":[{"from":"dc","to":"gain","input":"signal"}],"output":"gain"}))
        .unwrap()
    }
    fn advance(engine: &mut Engine) {
        engine.process_audio_block(&mut [[0.0; 2]; 64]);
    }

    #[test]
    fn port_light_lease_and_identity_are_exact_and_repeated_reads_do_not_refresh() {
        let patch = patch();
        let mut engine = patch.compile().unwrap();
        let (mut producer, reader) = prepare(&patch, &mut engine).unwrap();
        assert!(!producer.publish(&engine, 4, 0));
        assert!(reader.read(4).is_none());
        advance(&mut engine);
        let now = Instant::now();
        assert!(producer.publish_at(&engine, 4, 64, now));
        let first = reader.read_at(4, now).unwrap();
        assert!(reader.read_at(3, now).is_none());
        assert!(reader.read_at(5, now).is_none());
        assert!(Arc::ptr_eq(
            &first,
            &reader.read_at(4, now + Duration::from_millis(249)).unwrap()
        ));
        assert!(reader.read_at(4, now + LEASE).is_none());
        assert!(
            !producer.publish(&engine, 4, 128),
            "no real acquisition cannot renew a lease"
        );
        advance(&mut engine);
        assert!(producer.publish_at(&engine, 4, 128, now + LEASE));
        assert_eq!(reader.read_at(4, now + LEASE).unwrap().sequence, 2);
        let (_, replacement) = prepare(&patch, &mut engine).unwrap();
        assert!(
            replacement.read(4).is_none(),
            "new registry never reads prior channel"
        );
    }

    #[test]
    fn port_light_control_epoch_rejects_cached_and_unread_old_packets_until_actual_reacquisition() {
        let patch = patch();
        let mut engine = patch.compile().unwrap();
        let (mut producer, reader) = prepare(&patch, &mut engine).unwrap();
        advance(&mut engine);
        assert!(producer.publish_epoch(&engine, 4, 64, 1));
        let old = reader.read_epoch(4, 1).unwrap();
        assert_eq!(old.control_epoch, 1);
        assert!(reader.read_epoch(4, 2).is_none());
        assert!(reader.read_epoch(4, 5).is_none());
        engine.set_port_lights_enabled(false);
        engine.set_port_lights_enabled(true);
        assert!(!producer.publish_epoch(&engine, 4, 64, 5));
        assert!(reader.read_epoch(4, 5).is_none());
        advance(&mut engine);
        assert!(producer.publish_epoch(&engine, 4, 128, 5));
        let fresh = reader.read_epoch(4, 5).unwrap();
        assert_eq!(fresh.control_epoch, 5);
        assert!(fresh.sequence > old.sequence);
        assert!(reader.read_epoch(4, 1).is_none());
    }

    #[test]
    fn port_light_unread_old_packet_cannot_start_a_fresh_lease() {
        let patch = patch();
        let mut engine = patch.compile().unwrap();
        let (mut producer, reader) = prepare(&patch, &mut engine).unwrap();
        advance(&mut engine);
        let now = Instant::now();
        assert!(producer.publish_at(&engine, 2, 64, now));
        assert!(reader.read_at(2, now + LEASE).is_none());
        assert!(reader.0.lock().unwrap().cached.is_none());
    }

    #[test]
    fn port_light_overwrite_and_concurrent_readers_keep_endpoint_payload_coherent() {
        let patch = patch();
        let mut engine = patch.compile().unwrap();
        let (mut producer, reader) = prepare(&patch, &mut engine).unwrap();
        let writer = std::thread::spawn(move || {
            for sequence in 1..=500 {
                advance(&mut engine);
                assert!(producer.publish(&engine, 9, sequence * 64));
            }
        });
        let other = reader.clone();
        let observer = std::thread::spawn(move || {
            for _ in 0..100 {
                if let Some(frame) = other.read(9) {
                    check(&frame);
                }
                std::thread::yield_now();
            }
        });
        while !writer.is_finished() {
            if let Some(frame) = reader.read(9) {
                check(&frame);
            }
            std::thread::yield_now();
        }
        writer.join().unwrap();
        observer.join().unwrap();
        let last = reader.read(9).unwrap();
        check(&last);
        assert_eq!(
            last.sequence, 500,
            "latest state survives an unread full channel"
        );
    }
    fn check(frame: &PortLightFrame) {
        assert_eq!(frame.revision, 9);
        assert_eq!(frame.backend, 2);
        assert_eq!(frame.frame, frame.sequence * 64);
        assert!(frame.last_observed_frame <= frame.frame);
        assert!(frame.frame - frame.last_observed_frame <= 7);
        assert_eq!(frame.ports.len(), 2);
        for state in frame.ports.values() {
            assert!(state.connected);
            assert_eq!(state.channels, 1);
            assert!(!state.invalid);
            assert!(state.raw_rgb[1] > 0.0);
        }
    }
}
