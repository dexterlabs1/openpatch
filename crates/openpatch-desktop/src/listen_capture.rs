//! One live capture ring per audio host, independent of prepared DSP graphs.
//! Ring endpoints never migrate during adoption. Only the active callback writes;
//! the worker acknowledges stopped production before draining/reusing the ring.
// Capture policy adapted from RackMCP ListenService.cpp, Zachary Scheffler,
// revision 285e84c5ff2d3ecef25890da07f578af6a0c5ca6 (GPL-3.0-or-later).
// Modified for one host-owned ring, acknowledged reuse and native lifecycle fences.
use openpatch_core::{ListenFrame, ListenSink, ListenStatus, ModuleHandle};
use rtrb::{Consumer, Producer, RingBuffer};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const CAPACITY: usize = 1 << 18;
const STALL: Duration = Duration::from_secs(2);
const STARTING: u8 = 0;
const CAPTURING: u8 = 1;
const ANALYZING: u8 = 2;
const DONE: u8 = 3;
const FAILED: u8 = 4;

#[derive(Clone, Copy, Debug)]
pub(super) struct Target {
    pub slot: usize,
    pub token: u64,
}
#[derive(Clone, Copy)]
struct Command {
    serial: u64,
    job: u64,
    target: Target,
    armed: bool,
}
struct Shared {
    closed: AtomicBool,
    serial: AtomicU64,
    command: AtomicU64,
    job: AtomicU64,
    slot: AtomicUsize,
    token: AtomicU64,
    armed: AtomicBool,
    acknowledgment: AtomicU64,
    cancelled: AtomicU64,
    invalid: AtomicU64,
    dropped: AtomicU64,
    hints: AtomicU8,
}
impl Shared {
    fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            serial: AtomicU64::new(0),
            command: AtomicU64::new(0),
            job: AtomicU64::new(0),
            slot: AtomicUsize::new(0),
            token: AtomicU64::new(0),
            armed: AtomicBool::new(false),
            acknowledgment: AtomicU64::new(0),
            cancelled: AtomicU64::new(0),
            invalid: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            hints: AtomicU8::new(0),
        }
    }
    // Worker is the sole command publisher; callback is the sole acknowledger.
    fn publish(&self, job: &Job, armed: bool) -> u64 {
        self.serial.fetch_add(1, Ordering::SeqCst);
        self.job.store(job.generation, Ordering::SeqCst);
        self.slot.store(job.target.slot, Ordering::SeqCst);
        self.token.store(job.target.token, Ordering::SeqCst);
        self.armed.store(armed, Ordering::SeqCst);
        let command = self.command.fetch_add(1, Ordering::SeqCst) + 1;
        self.serial.fetch_add(1, Ordering::SeqCst);
        command
    }
    fn read(&self) -> Option<Command> {
        for _ in 0..4 {
            let serial = self.serial.load(Ordering::SeqCst);
            if serial & 1 != 0 {
                continue;
            }
            let result = Command {
                serial: self.command.load(Ordering::SeqCst),
                job: self.job.load(Ordering::SeqCst),
                target: Target {
                    slot: self.slot.load(Ordering::SeqCst),
                    token: self.token.load(Ordering::SeqCst),
                },
                armed: self.armed.load(Ordering::SeqCst),
            };
            if self.serial.load(Ordering::SeqCst) == serial {
                return Some(result);
            }
        }
        None
    }
    fn aborted(&self, job: &Job) -> Option<&'static str> {
        if self.closed.load(Ordering::Acquire) {
            Some("audio host closed")
        } else if self.cancelled.load(Ordering::Acquire) == job.generation {
            Some("capture cancelled")
        } else if self.invalid.load(Ordering::Acquire) == job.generation {
            Some("Listen instance removed, replaced or suspended")
        } else {
            None
        }
    }
}

pub(super) struct CaptureProducer {
    shared: Arc<Shared>,
    producer: Producer<ListenFrame>,
    current: Option<Command>,
}
impl CaptureProducer {
    /// Re-resolve incarnation against the actual active graph on every render.
    /// A stop acknowledgment is after all pushes from the preceding render.
    pub fn prepare(
        &mut self,
        resolve: impl FnOnce(Target) -> Option<ModuleHandle>,
    ) -> Option<ModuleHandle> {
        let command = self.shared.read().or(self.current)?;
        self.current = Some(command);
        let handle = if command.armed && !self.shared.closed.load(Ordering::Acquire) {
            let handle = resolve(command.target);
            if handle.is_none() {
                self.shared.invalid.store(command.job, Ordering::Release);
            }
            handle
        } else {
            None
        };
        self.shared
            .acknowledgment
            .store(command.serial, Ordering::Release);
        handle.filter(|_| self.armed())
    }
    pub fn invalidate(&self) {
        if let Some(command) = self
            .shared
            .read()
            .or(self.current)
            .filter(|command| command.job != 0)
        {
            self.shared.invalid.store(command.job, Ordering::Release);
        }
    }
}
impl ListenSink for CaptureProducer {
    fn armed(&self) -> bool {
        self.current.is_some_and(|command| {
            command.armed
                && !self.shared.closed.load(Ordering::Acquire)
                && self.shared.cancelled.load(Ordering::Acquire) != command.job
                && self.shared.invalid.load(Ordering::Acquire) != command.job
        })
    }
    fn push(&mut self, frame: ListenFrame) -> bool {
        if self.producer.push(frame).is_ok() {
            true
        } else {
            self.shared.dropped.fetch_add(1, Ordering::Relaxed);
            false
        }
    }
    fn observe_status(&mut self, status: ListenStatus) {
        self.shared.hints.store(
            u8::from(status.left_connected) | (u8::from(status.right_connected) << 1),
            Ordering::Release,
        );
    }
}

struct Job {
    generation: u64,
    id: String,
    module: String,
    target: Target,
    revision: u64,
    rate: u32,
    frames: usize,
    write_wav: bool,
    workspace: PathBuf,
    phase: AtomicU8,
    collected: AtomicUsize,
    dropped: AtomicU64,
    result: Mutex<Option<Value>>,
}
impl Job {
    fn snapshot(&self) -> Value {
        let phase = self.phase.load(Ordering::Acquire);
        json!({"job":self.id,"id":self.module,"revision":self.revision,
            "state":match phase { STARTING | CAPTURING => "capturing", ANALYZING => "analyzing", DONE => "done", _ => "failed" },
            "phase":match phase { STARTING => "arming", CAPTURING => "collecting", ANALYZING => "analyzing", DONE => "done", _ => "failed" },
            "sample_rate":self.rate,"target_frames":self.frames,
            "collected_frames":self.collected.load(Ordering::Acquire),
            "dropped_frames":self.dropped.load(Ordering::Acquire),
            "result":self.result.lock().ok().and_then(|value| value.clone())})
    }
    fn fail(&self, error: impl Into<String>) {
        *self
            .result
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(json!({"error":error.into()}));
        self.phase.store(FAILED, Ordering::Release);
    }
}
struct Worker {
    sender: mpsc::SyncSender<Arc<Job>>,
    thread: JoinHandle<()>,
}
struct State {
    consumer: Option<Consumer<ListenFrame>>,
    worker: Option<Worker>,
    counter: u64,
    jobs: VecDeque<Arc<Job>>,
}
pub(super) struct CaptureService {
    shared: Arc<Shared>,
    state: Mutex<State>,
}
pub(super) fn channel() -> (CaptureService, CaptureProducer) {
    let (producer, consumer) = RingBuffer::new(CAPACITY);
    let shared = Arc::new(Shared::new());
    (
        CaptureService {
            shared: shared.clone(),
            state: Mutex::new(State {
                consumer: Some(consumer),
                worker: None,
                counter: 0,
                jobs: VecDeque::new(),
            }),
        },
        CaptureProducer {
            shared,
            producer,
            current: None,
        },
    )
}
pub(super) struct StartRequest<'a> {
    pub target: Target,
    pub module: &'a str,
    pub revision: u64,
    pub rate: u32,
    pub seconds: f64,
    pub write_wav: bool,
    pub workspace: &'a Path,
}
impl CaptureService {
    pub fn start(&self, request: StartRequest<'_>) -> Result<Value, String> {
        let StartRequest {
            target,
            module,
            revision,
            rate,
            seconds,
            write_wav,
            workspace,
        } = request;
        if !seconds.is_finite()
            || !(0.1..=30.0).contains(&seconds)
            || !(8000..=192000).contains(&rate)
        {
            return Err("Listen requires 0.1..30 seconds and a supported live sample rate".into());
        }
        let mut state = self.state.lock().map_err(|_| "Listen registry poisoned")?;
        if self.shared.closed.load(Ordering::Acquire) {
            return Err("audio host closed".into());
        }
        if state
            .jobs
            .iter()
            .any(|job| job.phase.load(Ordering::Acquire) <= ANALYZING)
        {
            return Err("a Listen capture is already capturing or analyzing".into());
        }
        if state.worker.is_none() {
            let (sender, receiver) = mpsc::sync_channel::<Arc<Job>>(1);
            let mut consumer = state.consumer.take().ok_or("Listen worker unavailable")?;
            let shared = self.shared.clone();
            let thread = thread::Builder::new()
                .name("openpatch-listen".into())
                .spawn(move || {
                    while !shared.closed.load(Ordering::Acquire) {
                        match receiver.recv_timeout(Duration::from_millis(5)) {
                            Ok(job) => {
                                let rescue = job.clone();
                                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    run_job(&mut consumer, &shared, job)
                                }))
                                .is_err()
                                {
                                    shared.publish(&rescue, false);
                                    rescue.fail("Listen worker failed unexpectedly");
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|error| format!("could not start Listen worker: {error}"))?;
            state.worker = Some(Worker { sender, thread });
        }
        state.counter = state
            .counter
            .checked_add(1)
            .ok_or("Listen job generation exhausted")?;
        let job = Arc::new(Job {
            generation: state.counter,
            id: format!("listen-{}", state.counter),
            module: module.into(),
            target,
            revision,
            rate,
            frames: (seconds * f64::from(rate)).ceil() as usize,
            write_wav,
            workspace: workspace.into(),
            phase: AtomicU8::new(STARTING),
            collected: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
            result: Mutex::new(None),
        });
        state
            .worker
            .as_ref()
            .unwrap()
            .sender
            .try_send(job.clone())
            .map_err(|_| "Listen worker queue unavailable")?;
        state.jobs.push_back(job.clone());
        while state.jobs.len() > 8 {
            state.jobs.pop_front();
        }
        Ok(job.snapshot())
    }
    pub fn latest_for(&self, module: &str) -> Option<Value> {
        self.state
            .lock()
            .ok()?
            .jobs
            .back()
            .filter(|job| job.module == module)
            .map(|job| job.snapshot())
    }
    pub fn poll(&self, id: &str) -> Result<Value, String> {
        let state = self.state.lock().map_err(|_| "Listen registry poisoned")?;
        state
            .jobs
            .iter()
            .find(|job| job.id == id)
            .map(|job| job.snapshot())
            .ok_or_else(|| "Listen job not found".into())
    }
    pub fn cancel(&self, id: &str) -> Result<Value, String> {
        let state = self.state.lock().map_err(|_| "Listen registry poisoned")?;
        let job = state
            .jobs
            .iter()
            .find(|job| job.id == id)
            .ok_or("Listen job not found")?;
        let requested = job.phase.load(Ordering::Acquire) <= ANALYZING;
        if requested {
            self.shared
                .cancelled
                .store(job.generation, Ordering::Release);
        }
        let mut snapshot = job.snapshot();
        snapshot["cancel_requested"] = json!(requested);
        Ok(snapshot)
    }
    /// Called on the control thread when document execution/incarnation changes.
    pub fn invalidate_unless(&self, valid: impl Fn(Target) -> bool) {
        if let Ok(state) = self.state.lock() {
            if let Some(job) = state
                .jobs
                .back()
                .filter(|job| job.phase.load(Ordering::Acquire) <= CAPTURING)
            {
                if !valid(job.target) {
                    self.shared.invalid.store(job.generation, Ordering::Release);
                }
            }
        }
    }
    pub fn close(&self) {
        self.shared.closed.store(true, Ordering::Release);
        let worker = self
            .state
            .lock()
            .ok()
            .and_then(|mut state| state.worker.take());
        if let Some(worker) = worker {
            let _ = worker.thread.join();
        }
        if let Ok(state) = self.state.lock() {
            for job in &state.jobs {
                if job.phase.load(Ordering::Acquire) <= ANALYZING {
                    job.fail("audio host closed");
                }
            }
        }
    }
}
impl Drop for CaptureService {
    fn drop(&mut self) {
        self.close();
    }
}

fn wait_stopped(shared: &Shared, job: &Job, command: u64) -> bool {
    let start = Instant::now();
    while shared.acknowledgment.load(Ordering::Acquire) != command {
        if shared.aborted(job).is_some() || start.elapsed() >= STALL {
            return false;
        }
        thread::sleep(Duration::from_millis(5));
    }
    true
}
fn run_job(consumer: &mut Consumer<ListenFrame>, shared: &Shared, job: Arc<Job>) {
    let stop = shared.publish(&job, false);
    if !wait_stopped(shared, &job, stop) {
        job.fail(
            shared
                .aborted(&job)
                .unwrap_or("no audio callback acknowledged capture start for 2 s"),
        );
        return;
    }
    while consumer.pop().is_ok() {}
    let mut left = Vec::with_capacity(job.frames);
    let mut right = Vec::with_capacity(job.frames);
    shared.dropped.store(0, Ordering::Release);
    shared.publish(&job, true);
    job.phase.store(CAPTURING, Ordering::Release);
    let mut progress = Instant::now();
    let mut error = None;
    while left.len() < job.frames {
        if let Some(reason) = shared.aborted(&job) {
            error = Some(reason);
            break;
        }
        let mut read = 0;
        for _ in 0..8192.min(job.frames - left.len()) {
            let Ok(frame) = consumer.pop() else {
                break;
            };
            left.push(frame.left);
            right.push(frame.right);
            read += 1;
        }
        job.collected.store(left.len(), Ordering::Release);
        job.dropped
            .store(shared.dropped.load(Ordering::Acquire), Ordering::Release);
        if read > 0 {
            progress = Instant::now();
        } else if progress.elapsed() >= STALL {
            error = Some("no audio frames arrived for 2 s");
            break;
        } else {
            thread::sleep(Duration::from_millis(5));
        }
    }
    shared.publish(&job, false);
    if let Some(error) = error {
        job.fail(error);
        return;
    }
    // Ring reuse still requires a stopped-producer acknowledgment. Analysis can
    // finish even if the audio device stops immediately after the final frame.
    job.phase.store(ANALYZING, Ordering::Release);
    let hints = shared.hints.load(Ordering::Acquire);
    let analysis = match openpatch_core::rackmcp_listen_analysis::analyze(
        &left,
        &right,
        job.rate as f32,
        hints & 1 != 0,
        hints & 2 != 0,
    ) {
        Ok(value) => value,
        Err(error) => {
            job.fail(format!("analysis failed: {error}"));
            return;
        }
    };
    if let Some(error) = shared.aborted(&job) {
        job.fail(error);
        return;
    }
    let mut wav_file = None;
    let mut wav_error = None;
    if job.write_wav {
        match save_wav(&job, &left, if hints & 2 == 0 { &left } else { &right }) {
            Ok(file) => wav_file = Some(file),
            Err(error) => wav_error = Some(error),
        }
    }
    if let Some(error) = shared.aborted(&job) {
        *job.result.lock().unwrap_or_else(|error| error.into_inner()) =
            Some(json!({"error":error,"wav_file":wav_file,"wav_error":wav_error}));
        job.phase.store(FAILED, Ordering::Release);
        return;
    }
    *job.result.lock().unwrap_or_else(|error| error.into_inner()) = Some(json!({
        "analysis":analysis,"left_connected":hints & 1 != 0,"right_connected":hints & 2 != 0,
        "wav_file":wav_file,"wav_error":wav_error,"dropped_frames":job.dropped.load(Ordering::Acquire)
    }));
    job.phase.store(DONE, Ordering::Release);
}
fn save_wav(job: &Job, left: &[f32], right: &[f32]) -> Result<String, String> {
    let data = openpatch_core::rackmcp_listen_analysis::encode_wav_float_stereo(
        left,
        Some(right),
        job.rate,
    )?;
    for suffix in 0..10000 {
        let filename = if suffix == 0 {
            format!("{}.wav", job.id)
        } else {
            format!("{}-{suffix}.wav", job.id)
        };
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(job.workspace.join(&filename))
        {
            Ok(mut file) => {
                file.write_all(&data).map_err(|error| error.to_string())?;
                return Ok(filename);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("could not reserve a new capture filename".into())
}

#[cfg(test)]
#[path = "listen_capture_tests.rs"]
mod listen_capture_tests;
