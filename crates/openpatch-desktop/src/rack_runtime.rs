//! Optional original Rack worker. All process/JSON/file work runs off the audio
//! thread. Atomic bounded mailboxes keep graph callbacks nonblocking.
use openpatch_core::{Engine, Patch, PortFrame, RackPlaitsEndpoint, RACK_PLAITS_LATENCY};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const CAPACITY: usize = 8192;
const OUTPUT_CAPACITY: usize = 16384;
const BLOCK: usize = 128;

#[derive(Debug)]
struct AtomicPort {
    channels: AtomicU8,
    values: [AtomicU32; 16],
}
impl AtomicPort {
    fn new() -> Self {
        Self {
            channels: AtomicU8::new(0),
            values: std::array::from_fn(|_| AtomicU32::new(0)),
        }
    }
    fn store(&self, frame: &PortFrame) {
        let width = usize::from(frame.channels).min(16);
        for c in 0..width {
            self.values[c].store(frame.values[c].to_bits(), Ordering::Relaxed);
        }
        self.channels.store(width as u8, Ordering::Relaxed);
    }
    fn load(&self) -> PortFrame {
        let mut frame = PortFrame {
            channels: self.channels.load(Ordering::Relaxed).min(16),
            ..PortFrame::default()
        };
        for c in 0..usize::from(frame.channels) {
            frame.values[c] = f32::from_bits(self.values[c].load(Ordering::Relaxed));
        }
        frame
    }
}
#[derive(Clone, Copy, Debug)]
struct Input {
    sequence: u64,
    controls: [f32; 6],
    inputs: [PortFrame; 7],
}
#[derive(Debug)]
struct InputSlot {
    sequence: AtomicU64,
    controls: [AtomicU32; 6],
    inputs: [AtomicPort; 7],
}
impl InputSlot {
    fn new() -> Self {
        Self {
            sequence: AtomicU64::new(0),
            controls: std::array::from_fn(|_| AtomicU32::new(0)),
            inputs: std::array::from_fn(|_| AtomicPort::new()),
        }
    }
    fn store(&self, packet: &Input) {
        self.sequence.store(packet.sequence, Ordering::Relaxed);
        for (to, value) in self.controls.iter().zip(packet.controls) {
            to.store(value.to_bits(), Ordering::Relaxed);
        }
        for (to, frame) in self.inputs.iter().zip(&packet.inputs) {
            to.store(frame);
        }
    }
    fn load(&self) -> Input {
        Input {
            sequence: self.sequence.load(Ordering::Relaxed),
            controls: std::array::from_fn(|i| {
                f32::from_bits(self.controls[i].load(Ordering::Relaxed))
            }),
            inputs: std::array::from_fn(|i| self.inputs[i].load()),
        }
    }
}
#[derive(Debug)]
struct OutputSlot {
    stamp: AtomicU64,
    outputs: [AtomicPort; 2],
}
impl OutputSlot {
    fn new() -> Self {
        Self {
            stamp: AtomicU64::new(0),
            outputs: std::array::from_fn(|_| AtomicPort::new()),
        }
    }
    fn store(&self, sequence: u64, outputs: &[PortFrame; 2]) {
        self.stamp.store(0, Ordering::Release);
        for (to, frame) in self.outputs.iter().zip(outputs) {
            to.store(frame);
        }
        self.stamp.store(sequence + 1, Ordering::Release);
    }
    fn load(&self, sequence: u64) -> Option<[PortFrame; 2]> {
        if self.stamp.load(Ordering::Acquire) != sequence + 1 {
            return None;
        }
        let outputs = std::array::from_fn(|i| self.outputs[i].load());
        (self.stamp.load(Ordering::Acquire) == sequence + 1).then_some(outputs)
    }
}
#[derive(Debug)]
struct Bridge {
    incoming: Box<[InputSlot]>,
    outgoing: Box<[OutputSlot]>,
    head: AtomicU64,
    tail: AtomicU64,
    clock: AtomicU64,
    owner: AtomicU64,
    next_owner: AtomicU64,
    status: AtomicU8,
    drops: AtomicU64,
    underruns: AtomicU64,
    responses: AtomicU64,
    heartbeat: AtomicU64,
    started: Instant,
    error: Mutex<Option<String>>,
}
impl Bridge {
    fn new() -> Self {
        Self {
            incoming: (0..CAPACITY).map(|_| InputSlot::new()).collect(),
            outgoing: (0..OUTPUT_CAPACITY).map(|_| OutputSlot::new()).collect(),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            clock: AtomicU64::new(0),
            owner: AtomicU64::new(0),
            next_owner: AtomicU64::new(1),
            status: AtomicU8::new(0),
            drops: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            responses: AtomicU64::new(0),
            heartbeat: AtomicU64::new(0),
            started: Instant::now(),
            error: Mutex::new(None),
        }
    }
    // Only the active graph endpoint produces; only its worker consumes. Atomic
    // slots additionally avoid unsafe shared memory or Rust aliasing violations.
    fn push(&self, packet: &Input) {
        let tail = self.tail.load(Ordering::Relaxed);
        if tail.wrapping_sub(self.head.load(Ordering::Acquire)) >= CAPACITY as u64 {
            self.drops.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.incoming[tail as usize % CAPACITY].store(packet);
        self.tail.store(tail + 1, Ordering::Release);
    }
    fn pop(&self) -> Option<Input> {
        let head = self.head.load(Ordering::Relaxed);
        if head == self.tail.load(Ordering::Acquire) {
            return None;
        }
        let value = self.incoming[head as usize % CAPACITY].load();
        self.head.store(head + 1, Ordering::Release);
        Some(value)
    }
    fn failed(&self, error: String) {
        *self.error.lock().unwrap() = Some(error);
        self.status.store(2, Ordering::Release);
    }
    fn endpoint(self: &Arc<Self>) -> Box<dyn RackPlaitsEndpoint> {
        Box::new(Endpoint {
            bridge: self.clone(),
            owner: self.next_owner.fetch_add(1, Ordering::Relaxed),
            cursor: self.clock.load(Ordering::Relaxed),
        })
    }
}
#[derive(Debug)]
struct Endpoint {
    bridge: Arc<Bridge>,
    owner: u64,
    cursor: u64,
}
impl RackPlaitsEndpoint for Endpoint {
    fn activate(&mut self) {
        self.cursor = self.bridge.clock.load(Ordering::Relaxed);
        self.bridge.owner.fetch_max(self.owner, Ordering::AcqRel);
    }
    fn process(&mut self, controls: &[f32; 6], inputs: &[PortFrame], outputs: &mut [PortFrame]) {
        let bridge = &self.bridge;
        // Fresh document loads can exclude state adoption. The newest endpoint
        // claims ownership on first use as well; retired graphs cannot reclaim it.
        bridge.owner.fetch_max(self.owner, Ordering::AcqRel);
        let active = bridge.owner.load(Ordering::Acquire) == self.owner;
        // The callback renders the active block before the retiring block. Each
        // reader must keep its own position so both graphs hear matching frames,
        // rather than repeating the final active sample throughout the fade.
        let sequence = if active {
            bridge.clock.fetch_add(1, Ordering::Relaxed)
        } else {
            self.cursor
        };
        self.cursor = sequence + 1;
        if active && bridge.status.load(Ordering::Acquire) != 2 {
            bridge.push(&Input {
                sequence,
                controls: *controls,
                inputs: std::array::from_fn(|i| inputs.get(i).copied().unwrap_or_default()),
            });
        }
        let rendered = if sequence < RACK_PLAITS_LATENCY {
            [PortFrame::default(); 2]
        } else {
            let expected = sequence - RACK_PLAITS_LATENCY;
            bridge.outgoing[expected as usize % OUTPUT_CAPACITY]
                .load(expected)
                .unwrap_or_else(|| {
                    if active {
                        bridge.underruns.fetch_add(1, Ordering::Relaxed);
                    }
                    [PortFrame::default(); 2]
                })
        };
        for (to, value) in outputs.iter_mut().zip(rendered) {
            *to = value;
        }
    }
}

struct Worker {
    bridge: Arc<Bridge>,
    child: Arc<Mutex<Child>>,
    rate: u32,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.lock().unwrap().kill();
    }
}
#[derive(Default)]
pub(crate) struct Manager {
    workers: BTreeMap<String, Worker>,
}
impl Manager {
    pub fn attach(&mut self, patch: &Patch, engine: &mut Engine, rate: u32) -> Result<(), String> {
        let ids = patch
            .nodes
            .iter()
            .filter(|n| n.kind == "rack_plaits")
            .map(|n| n.id.clone())
            .collect::<Vec<_>>();
        for id in &ids {
            if !self
                .workers
                .get(id)
                .is_some_and(|w| w.rate == rate && w.bridge.status.load(Ordering::Acquire) != 2)
            {
                self.workers.insert(id.clone(), spawn(rate)?);
            }
            engine.attach_rack_plaits(id, self.workers[id].bridge.endpoint())?;
        }
        self.workers.retain(|id, _| ids.contains(id));
        Ok(())
    }
    pub fn service(&mut self) {
        for worker in self.workers.values() {
            let bridge = &worker.bridge;
            let idle = (bridge.started.elapsed().as_millis() as u64)
                .saturating_sub(bridge.heartbeat.load(Ordering::Relaxed));
            if idle > 10000 && bridge.status.load(Ordering::Acquire) != 2 {
                bridge.failed("Original Rack worker stopped responding".into());
                let _ = worker.child.lock().unwrap().kill();
            }
        }
    }
    pub fn status(&self) -> Option<String> {
        let worker = self.workers.values().next()?;
        let bridge = &worker.bridge;
        if bridge.status.load(Ordering::Acquire) == 2 {
            return Some(format!(
                "Rack Plaits: {}",
                bridge
                    .error
                    .lock()
                    .unwrap()
                    .as_deref()
                    .unwrap_or("worker failed")
            ));
        }
        let latency = 1000.0 * RACK_PLAITS_LATENCY as f64 / f64::from(worker.rate);
        Some(format!(
            "Original Plaits · {latency:.0} ms · {} late samples / {} dropped",
            bridge.underruns.load(Ordering::Relaxed),
            bridge.drops.load(Ordering::Relaxed)
        ))
    }
}

fn read_reply(reader: &mut impl BufRead) -> Result<Value, String> {
    let mut line = String::new();
    reader
        .take(4 * 1024 * 1024)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.is_empty() || !line.ends_with('\n') {
        return Err("Rack worker disconnected or response too large".into());
    }
    let response: Value =
        serde_json::from_str(&line).map_err(|e| format!("Invalid Rack response: {e}"))?;
    if let Some(error) = response.get("error") {
        return Err(format!("Rack worker: {error}"));
    }
    Ok(response)
}

fn spawn(rate: u32) -> Result<Worker, String> {
    let config = std::env::var_os("OPENPATCH_RACK_HOST_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .unwrap_or_default(),
            )
            .join("OpenPatch/rack-host.json")
        });
    let settings: Value = serde_json::from_slice(&std::fs::read(&config).map_err(|_| {
        format!(
            "Configure the original Rack worker first: {}",
            config.display()
        )
    })?)
    .map_err(|e| e.to_string())?;
    let field = |name: &str| {
        settings[name]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| format!("Rack configuration missing {name}"))
    };
    let executable = field("worker")?;
    let sdk = field("sdk")?;
    let plugin = field("plugin")?;
    let system = field("system")?;
    let storage = tempfile::tempdir().map_err(|e| e.to_string())?;
    let mut diagnostics = tempfile::tempfile().map_err(|e| e.to_string())?;
    let mut child = Command::new(executable)
        .args([
            plugin,
            "Plaits".into(),
            rate.to_string(),
            system,
            storage.path().to_string_lossy().to_string(),
        ])
        .env("DYLD_LIBRARY_PATH", sdk)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(
            diagnostics.try_clone().map_err(|e| e.to_string())?,
        ))
        .spawn()
        .map_err(|e| format!("Could not start original Rack worker: {e}"))?;
    let mut input = child.stdin.take().ok_or("Missing Rack input pipe")?;
    let mut output = BufReader::new(child.stdout.take().ok_or("Missing Rack output pipe")?);
    let child = Arc::new(Mutex::new(child));
    let process = child.clone();
    let bridge = Arc::new(Bridge::new());
    let stream = bridge.clone();
    std::thread::Builder::new().name("original-rack-plaits".into()).spawn(move || {
        let mut run=||->Result<(),String> {
            let ready=read_reply(&mut output)?;
            if ready["ready"]["model"]!="Plaits" || ready["ready"]["plugin"]!="AudibleInstruments" { return Err("Original Plaits handshake failed".into()); }
            stream.status.store(1,Ordering::Release);
            let mut packets=Vec::with_capacity(BLOCK);
            loop {
                if process.lock().unwrap().try_wait().map_err(|e|e.to_string())?.is_some() { return Err("Original Rack worker exited".into()); }
                while packets.len()<BLOCK {
                    if let Some(packet)=stream.pop() { packets.push(packet); } else { break; }
                }
                if packets.len()<BLOCK { std::thread::sleep(Duration::from_micros(250)); continue; }
                let mut payload=Vec::with_capacity(packets.len()*7*65);
                for packet in &packets { for port in &packet.inputs {
                    let width=usize::from(port.channels).min(16);payload.push(width as u8);
                    for value in &port.values[..width] { payload.extend_from_slice(&value.to_le_bytes()); }
                } }
                let request=json!({"op":"stream","frames":packets.len(),"input_bytes":payload.len(),"controls":packets.last().unwrap().controls});
                serde_json::to_writer(&mut input,&request).map_err(|e|e.to_string())?;
                input.write_all(b"\n").map_err(|e|e.to_string())?;
                input.write_all(&payload).map_err(|e|e.to_string())?;
                input.flush().map_err(|e|e.to_string())?;
                let response=read_reply(&mut output)?;
                let count=response["result"]["bytes"].as_u64().ok_or("Missing Rack binary length")? as usize;
                if count>packets.len()*130 || response["result"]["frames"].as_u64()!=Some(packets.len() as u64) || response["result"]["encoding"]!="f32le-voices" { return Err("Invalid Rack binary response".into()); }
                let mut payload=vec![0u8;count];output.read_exact(&mut payload).map_err(|e|e.to_string())?;
                let mut cursor=0;
                for packet in &packets {
                    let mut rendered=[PortFrame::default();2];
                    for target in &mut rendered {
                        let width=*payload.get(cursor).ok_or("Missing Rack output width")? as usize;cursor+=1;
                        if width>16 || cursor+width*4>payload.len() { return Err("Invalid Rack output width".into()); }
                        target.channels=width as u8;
                        for value in &mut target.values[..width] {
                            let sample=f32::from_le_bytes(payload[cursor..cursor+4].try_into().unwrap());cursor+=4;
                            if !sample.is_finite() { return Err("Nonfinite Rack voltage".into()); }
                            *value=sample.clamp(-1000.0,1000.0);
                        }
                    }
                    stream.outgoing[packet.sequence as usize%OUTPUT_CAPACITY].store(packet.sequence,&rendered);
                }
                if cursor!=payload.len() { return Err("Unexpected trailing Rack audio bytes".into()); }
                stream.responses.fetch_add(packets.len() as u64,Ordering::Relaxed);
                stream.heartbeat.store(stream.started.elapsed().as_millis() as u64,Ordering::Relaxed);
                packets.clear();
            }
        };
        if let Err(mut error)=run() {
            let _=process.lock().unwrap().kill();
            let _=process.lock().unwrap().wait();
            let _=diagnostics.seek(SeekFrom::Start(0));
            let mut log=String::new();let _=diagnostics.take(2048).read_to_string(&mut log);
            if !log.is_empty() { error.push_str(&format!(" ({})",log.trim())); }
            stream.failed(error);
        }
        drop(storage);
    }).map_err(|e|e.to_string())?;
    Ok(Worker {
        bridge,
        child,
        rate,
    })
}

#[cfg(test)]
pub(crate) fn callback_test_endpoint() -> Box<dyn RackPlaitsEndpoint> {
    Arc::new(Bridge::new()).endpoint()
}

#[cfg(test)]
pub(crate) fn callback_test_stream() -> impl Fn() -> Box<dyn RackPlaitsEndpoint> {
    let bridge = Arc::new(Bridge::new());
    bridge.clock.store(RACK_PLAITS_LATENCY, Ordering::Relaxed);
    for sequence in 0..CAPACITY {
        let signal = (sequence as f32 * 0.13).sin() * 0.2;
        bridge.outgoing[sequence].store(
            sequence as u64,
            &[PortFrame::mono(signal), PortFrame::mono(-signal)],
        );
    }
    move || bridge.endpoint()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retiring_graph_replays_matching_audio_frames_after_each_control_edit() {
        let bridge = Arc::new(Bridge::new());
        let controls = [4.0, 0.0, 0.3, 0.4, 0.5, 0.6];
        let inputs = [PortFrame::mono(0.0); 7];
        let mut old = bridge.endpoint();
        let mut output = [PortFrame::default(); 2];
        for _ in 0..RACK_PLAITS_LATENCY {
            old.process(&controls, &inputs, &mut output);
            bridge.pop().unwrap();
        }
        let mut sequence = 0;
        for (edit, length) in [128, 1, 17, 129, 64, 3, 128].into_iter().enumerate() {
            let mut edited = controls;
            edited[edit % 6] += 0.05;
            for frame in 0..length {
                let time = (sequence + frame) as f32;
                let signal = (time * 0.13).sin() * 3.0 + 0.25;
                bridge.outgoing[(sequence + frame) % OUTPUT_CAPACITY].store(
                    (sequence + frame) as u64,
                    &[PortFrame::mono(signal), PortFrame::mono(-signal)],
                );
            }
            let mut active = bridge.endpoint();
            active.activate();
            let mut expected = Vec::new();
            // The real callback renders an entire active block before rendering
            // the corresponding retiring block, rather than interleaving samples.
            for _ in 0..length {
                active.process(&edited, &inputs, &mut output);
                expected.push(output);
            }
            for frame in expected {
                old.process(&controls, &inputs, &mut output);
                assert_eq!(
                    output, frame,
                    "crossfade audio froze during control edit {edit}"
                );
            }
            assert_eq!(
                bridge.clock.load(Ordering::Relaxed),
                RACK_PLAITS_LATENCY + (sequence + length) as u64
            );
            old = active;
            sequence += length;
        }
    }
    #[test]
    #[ignore = "requires configured local original Rack Plaits binary"]
    fn original_plaits_live_bridge_polyphony_edits_and_deadlines() {
        let patch:Patch=serde_json::from_value(json!({"version":2,"sample_rate":48000,
            "nodes":[{"id":"pitch","kind":"poly_constant","params":{"channels":4,"v1":0,"v2":0.3333333,"v3":0.5833333,"v4":1}},
            {"id":"voice","kind":"rack_plaits","params":{"engine":4}}],
            "cables":[{"from":"pitch","output":"out","to":"voice","input":"pitch"}],"output":"voice"})).unwrap();
        let mut engine = patch.compile().unwrap();
        let mut manager = Manager::default();
        manager.attach(&patch, &mut engine, 48000).unwrap();
        let bridge = manager.workers["voice"].bridge.clone();
        let deadline = Instant::now() + Duration::from_secs(5);
        while bridge.status.load(Ordering::Acquire) == 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            bridge.status.load(Ordering::Acquire),
            1,
            "{:?}",
            bridge.error.lock().unwrap()
        );
        let start = Instant::now();
        let mut output = [PortFrame::default(); 128];
        let mut peak = 0f32;
        let mut sounding = 0;
        for block in 0..750 {
            if block == 375 {
                let mut edited = patch.clone();
                edited.nodes[1].params.insert("engine".into(), 5.0);
                let mut replacement = edited.compile().unwrap();
                manager.attach(&edited, &mut replacement, 48000).unwrap();
                replacement.adopt_state_from(&engine);
                engine = replacement;
            }
            engine.process_poly_block(&mut output);
            for frame in &output {
                if frame.channels == 4 {
                    sounding += 1;
                }
                peak = peak.max(frame.channel(0).abs());
            }
            let target = start + Duration::from_secs_f64((block + 1) as f64 * 128.0 / 48000.0);
            if let Some(wait) = target.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
        }
        manager.service();
        let late = bridge.underruns.load(Ordering::Relaxed);
        let dropped = bridge.drops.load(Ordering::Relaxed);
        eprintln!("original Plaits: peak={peak}, sounding={sounding}, late={late}, dropped={dropped}, status={:?}",manager.status());
        assert_eq!(bridge.status.load(Ordering::Acquire), 1);
        assert!(peak > 0.05);
        assert!(sounding > 80000);
        assert_eq!(dropped, 0);
        assert!(late < 96, "live worker missed audio deadlines");
    }
    #[test]
    fn transport_is_delayed_nonblocking_and_graph_adoption_has_one_producer() {
        let bridge = Arc::new(Bridge::new());
        let mut old = bridge.endpoint();
        let mut new = bridge.endpoint();
        let inputs = [PortFrame::mono(0.25); 7];
        let mut outputs = [PortFrame::default(); 2];
        let controls = [4.0, 0.0, 0.3, 0.4, 0.5, 0.6];
        for sequence in 0..RACK_PLAITS_LATENCY {
            old.process(&controls, &inputs, &mut outputs);
            assert_eq!(bridge.pop().unwrap().sequence, sequence);
            assert_eq!(outputs, [PortFrame::default(); 2]);
        }
        new.activate();
        bridge.outgoing[0].store(0, &[PortFrame::mono(2.0), PortFrame::mono(-3.0)]);
        new.process(&controls, &inputs, &mut outputs);
        assert_eq!(outputs, [PortFrame::mono(2.0), PortFrame::mono(-3.0)]);
        old.process(&controls, &inputs, &mut outputs);
        assert_eq!(outputs[0].channel(0), 2.0);
        assert_eq!(bridge.pop().unwrap().sequence, RACK_PLAITS_LATENCY);
        assert!(bridge.pop().is_none());
    }
    #[test]
    fn full_input_ring_drops_without_overwriting_and_late_output_is_silent() {
        let bridge = Arc::new(Bridge::new());
        let packet = Input {
            sequence: 0,
            controls: [0.0; 6],
            inputs: [PortFrame::default(); 7],
        };
        for _ in 0..CAPACITY + 2 {
            bridge.push(&packet);
        }
        assert_eq!(bridge.drops.load(Ordering::Relaxed), 2);
        for _ in 0..CAPACITY {
            assert!(bridge.pop().is_some());
        }
        assert!(bridge.pop().is_none());
        bridge.clock.store(RACK_PLAITS_LATENCY, Ordering::Relaxed);
        let mut endpoint = bridge.endpoint();
        let mut outputs = [PortFrame::mono(99.0); 2];
        endpoint.process(&[0.0; 6], &[], &mut outputs);
        assert_eq!(outputs, [PortFrame::default(); 2]);
        assert_eq!(bridge.underruns.load(Ordering::Relaxed), 1);
    }
}
