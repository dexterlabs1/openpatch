//! Actual native graph processing, capture selection and lifetime contract.
use openpatch_core::{Engine, ListenFrame, ListenSink, ModuleHandle, Patch, PortFrame, TapSample};
use serde_json::json;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

struct CallbackAllocator;
thread_local! {
    static TRACKING: Cell<bool> = const { Cell::new(false) };
    static OPERATIONS: Cell<usize> = const { Cell::new(0) };
}
fn record() {
    let _ = TRACKING.try_with(|tracking| {
        if tracking.get() {
            let _ = OPERATIONS.try_with(|count| count.set(count.get() + 1));
        }
    });
}
unsafe impl GlobalAlloc for CallbackAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        record();
        unsafe { System.realloc(ptr, layout, size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record();
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: CallbackAllocator = CallbackAllocator;

struct Sink {
    armed: bool,
    frames: [ListenFrame; 1024],
    accepted: usize,
    attempts: usize,
    capacity: usize,
    disarm_after: Option<usize>,
    observation: Option<(usize, openpatch_core::ListenStatus)>,
}
impl Sink {
    fn new() -> Self {
        Self {
            armed: true,
            frames: [ListenFrame::default(); 1024],
            accepted: 0,
            attempts: 0,
            capacity: 1024,
            disarm_after: None,
            observation: None,
        }
    }
}
impl ListenSink for Sink {
    fn observe_status(&mut self, status: openpatch_core::ListenStatus) {
        self.observation = Some((self.attempts, status));
    }
    fn armed(&self) -> bool {
        self.armed
    }
    fn push(&mut self, frame: ListenFrame) -> bool {
        self.attempts += 1;
        let accepted = self.accepted < self.capacity;
        if accepted {
            self.frames[self.accepted] = frame;
            self.accepted += 1;
        }
        if self.disarm_after == Some(self.attempts) {
            self.armed = false;
        }
        accepted
    }
}
fn patch(right: bool, selected: &str) -> Patch {
    let mut cables = vec![json!({"id":"l","from":"l","to":"listen","input":"left"})];
    if right {
        cables.push(json!({"id":"r","from":"r","to":"listen","input":"right"}));
    }
    serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"l","kind":"poly_constant","params":{"channels":0}},
        {"id":"r","kind":"poly_constant","params":{"channels":0}},
        {"id":"listen","kind":"rackmcp_listen"},{"id":"other","kind":"rackmcp_listen"}],
        "cables":cables,"output":"listen","output_port":selected}))
    .unwrap()
}
fn frame(values: &[f32]) -> PortFrame {
    let mut frame = PortFrame {
        channels: values.len() as u8,
        ..PortFrame::default()
    };
    frame.values[..values.len()].copy_from_slice(values);
    frame
}
fn source(engine: &mut Engine, id: &str, values: &[f32]) {
    let handle = engine.resolve_module_state(id).unwrap();
    assert!(engine.set_host_reference_source(handle, frame(values)));
}
fn listen(engine: &Engine) -> ModuleHandle {
    engine.resolve_module_state("listen").unwrap()
}
fn step(engine: &mut Engine, handle: ModuleHandle, sink: &mut Sink, count: usize) {
    engine.process_poly_block_with_listen_capture(
        &mut [PortFrame::default(); 1024][..count],
        handle,
        sink,
    );
}

#[test]
fn raw_sixteen_channel_outputs_and_capture_have_distinct_source_semantics() {
    let mut engine = patch(true, "left").compile().unwrap();
    let handle = listen(&engine);
    let values = [
        1.0e20, 1.0, -1.0e20, 3.0, -0.0, -4.0, 2.0, 1.0, 0.5, -0.5, 1.0, 2.0, 3.0, -2.0, 4.0, -1.0,
    ];
    source(&mut engine, "l", &values);
    source(&mut engine, "r", &[-7.0, 2.0]);
    let mut sink = Sink::new();
    step(&mut engine, handle, &mut sink, 3);
    let raw = engine.host_module_snapshot(handle).unwrap();
    assert_eq!(raw.outputs[0].channels, 16);
    assert_eq!(
        raw.outputs[0].values.map(f32::to_bits),
        values.map(f32::to_bits)
    );
    // The unused right output stays disconnected but still receives raw writes.
    assert_eq!(raw.outputs[1].channels, 0);
    assert_eq!(&raw.outputs[1].values[..2], &[-7.0, 2.0]);
    let sum = values.iter().fold(0.0f32, |sum, value| sum + value);
    assert_eq!(
        sink.frames[2],
        ListenFrame {
            left: sum,
            right: -5.0
        }
    );
    assert_eq!(engine.listen_status(handle).unwrap().frames_seen, 3);
    assert_eq!(
        engine
            .listen_status(engine.resolve_module_state("other").unwrap())
            .unwrap()
            .frames_seen,
        3
    );
}

#[test]
fn absent_right_is_zero_without_mono_copy_and_silence_is_still_a_frame() {
    let mut engine = patch(false, "right").compile().unwrap();
    let handle = listen(&engine);
    source(&mut engine, "l", &[4.0]);
    let mut sink = Sink::new();
    let mut output = [PortFrame::default(); 3];
    engine.process_poly_block_with_listen_capture(&mut output, handle, &mut sink);
    assert_eq!(
        sink.frames[2],
        ListenFrame {
            left: 4.0,
            right: 0.0
        }
    );
    assert_eq!(output[2], PortFrame::mono(0.0));
    source(&mut engine, "l", &[0.0]);
    step(&mut engine, handle, &mut sink, 3);
    assert_eq!(sink.accepted, 6);
    assert_eq!(sink.frames[5], ListenFrame::default());
    sink.armed = false;
    step(&mut engine, handle, &mut sink, 5);
    assert_eq!(sink.accepted, 6);
    assert_eq!(engine.listen_status(handle).unwrap().frames_seen, 11);
}

#[test]
fn armed_state_is_read_at_each_sample_and_connection_lamp_publish_at_256() {
    let mut engine = patch(false, "left").compile().unwrap();
    let handle = listen(&engine);
    source(&mut engine, "l", &[2.0]);
    let mut sink = Sink::new();
    sink.disarm_after = Some(255);
    step(&mut engine, handle, &mut sink, 255);
    let status = engine.listen_status(handle).unwrap();
    assert!(!status.capturing && !status.left_connected && !status.right_connected);
    step(&mut engine, handle, &mut sink, 1);
    let status = engine.listen_status(handle).unwrap();
    assert_eq!(status.frames_seen, 256);
    assert!(status.left_connected && !status.right_connected && !status.capturing);
    assert_eq!(sink.accepted, 255);
    let (attempts, observed) = sink.observation.unwrap();
    assert_eq!(attempts, 255);
    assert_eq!(observed, status);
    sink.armed = true;
    step(&mut engine, handle, &mut sink, 255);
    assert!(!engine.listen_status(handle).unwrap().capturing);
    step(&mut engine, handle, &mut sink, 1);
    assert!(engine.listen_status(handle).unwrap().capturing);
    let lights = engine
        .module_lights(engine.resolve_module_lights("listen").unwrap())
        .unwrap();
    assert_eq!(lights.count, 1);
    assert_eq!(lights.brightness[0], [1.0, 0.0]);
}

#[test]
fn full_sink_drops_new_frames_and_later_drain_resumes_without_blocking() {
    let mut engine = patch(false, "left").compile().unwrap();
    let handle = listen(&engine);
    let mut sink = Sink::new();
    sink.capacity = 2;
    step(&mut engine, handle, &mut sink, 7);
    assert_eq!(sink.accepted, 2);
    assert_eq!(engine.listen_status(handle).unwrap().dropped, 5);
    sink.accepted = 0;
    step(&mut engine, handle, &mut sink, 2);
    assert_eq!(sink.accepted, 2);
    assert_eq!(engine.listen_status(handle).unwrap().dropped, 5);
}

#[test]
fn bypass_routes_raw_audio_and_freezes_capture_counters_hints_and_divider() {
    let patch = patch(true, "left");
    let mut old = patch.compile().unwrap();
    let handle = listen(&old);
    source(&mut old, "l", &[3.0, -1.0]);
    source(&mut old, "r", &[6.0]);
    let mut sink = Sink::new();
    step(&mut old, handle, &mut sink, 255);
    let before = old.listen_status(handle).unwrap();
    let mut bypass_patch = patch.clone();
    bypass_patch.nodes[2].bypassed = true;
    let mut bypassed = bypass_patch.compile().unwrap();
    bypassed.adopt_state_from(&old);
    // Reference source frames are runtime probe controls, not persisted params;
    // the receiving graph owns them and must receive the same held source.
    source(&mut bypassed, "l", &[3.0, -1.0]);
    source(&mut bypassed, "r", &[6.0]);
    let bypass_handle = listen(&bypassed);
    step(&mut bypassed, bypass_handle, &mut sink, 9);
    assert_eq!(bypassed.listen_status(bypass_handle), Some(before));
    assert_eq!(sink.accepted, 255);
    let raw = bypassed.host_module_snapshot(bypass_handle).unwrap();
    assert_eq!(raw.outputs[0].channels, 2);
    assert_eq!(&raw.outputs[0].values[..2], &[3.0, -1.0]);
    let mut resumed = patch.compile().unwrap();
    resumed.adopt_state_from(&bypassed);
    source(&mut resumed, "l", &[3.0, -1.0]);
    source(&mut resumed, "r", &[6.0]);
    let resumed_handle = listen(&resumed);
    step(&mut resumed, resumed_handle, &mut sink, 1);
    let after = resumed.listen_status(resumed_handle).unwrap();
    assert_eq!(after.frames_seen, 256);
    assert!(after.capturing && after.left_connected && after.right_connected);
}

#[test]
fn handles_are_generation_fenced_and_state_adopts_across_rate_changes_only_for_same_incarnation() {
    let patch = patch(false, "left");
    let mut old = patch.compile().unwrap();
    let old_handle = listen(&old);
    let mut sink = Sink::new();
    step(&mut old, old_handle, &mut sink, 255);
    let mut changed = patch.clone();
    changed.sample_rate = 96000;
    let mut current = changed.compile().unwrap();
    current.adopt_state_from(&old);
    let handle = listen(&current);
    assert_eq!(old.sample_rate(), 48000);
    assert_eq!(current.sample_rate(), 96000);
    assert_eq!(current.listen_status(handle), old.listen_status(old_handle));
    assert_eq!(current.listen_status(old_handle), None);
    let before = sink.accepted;
    step(&mut current, old_handle, &mut sink, 1);
    assert_eq!(sink.accepted, before);
    assert_eq!(current.listen_status(handle).unwrap().frames_seen, 256);
    assert!(!current.listen_status(handle).unwrap().capturing);
    let mut fresh = changed.compile().unwrap();
    let fresh_handle = listen(&fresh);
    fresh.adopt_state_from_excluding(&current, &[fresh_handle]);
    assert_eq!(fresh.listen_status(fresh_handle).unwrap().frames_seen, 0);
    assert!(!fresh.listen_status(fresh_handle).unwrap().capturing);
    assert_eq!(
        fresh.listen_status(fresh.resolve_module_state("l").unwrap()),
        None
    );
}

#[test]
fn audio_scope_capture_overflow_and_adoption_touch_no_heap() {
    let patch = patch(true, "left");
    let mut first = patch.compile().unwrap();
    let first_handle = listen(&first);
    let mut second = patch.compile().unwrap();
    let second_handle = listen(&second);
    let taps = [Some(second.resolve_tap("listen", "left", 0).unwrap())];
    let mut samples = [[TapSample::default(); 1]; 513];
    let mut audio = [[0.0; 2]; 513];
    let mut sink = Sink::new();
    sink.capacity = 3;
    OPERATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
    first.process_audio_block_with_listen_capture(&mut audio, first_handle, &mut sink);
    second.adopt_state_from(&first);
    second.process_audio_block_with_taps_and_listen_capture(
        &mut audio,
        &taps,
        &mut samples,
        second_handle,
        &mut sink,
    );
    TRACKING.with(|tracking| tracking.set(false));
    assert_eq!(OPERATIONS.with(Cell::get), 0);
    assert_eq!(second.listen_status(second_handle).unwrap().dropped, 1023);
}

#[test]
fn catalogue_and_compiler_require_version_two_and_no_parameters() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["kind"] == "rackmcp_listen")
        .unwrap();
    assert_eq!(module["min_patch_version"], 2);
    assert_eq!(module["inputs"], json!(["left", "right"]));
    assert_eq!(module["outputs"], json!(["left", "right"]));
    assert_eq!(module["params"], json!({}));
    let mut invalid = patch(false, "left");
    invalid.nodes[2].params.insert("gain".into(), 1.0);
    assert!(invalid.compile().is_err());
    invalid.nodes[2].params.clear();
    invalid.version = 1;
    assert!(invalid.compile().is_err());
}
