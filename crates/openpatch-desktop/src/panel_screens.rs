//! Bounded, passive module displays. Preparation and reading stay on the UI
//! thread. The callback samples already-resolved outputs into fixed histories.
use eframe::egui::{pos2, Rect};
use openpatch_core::{Engine, Patch, ResolvedTap, TapSample};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub(crate) const MAX_SCREENS: usize = 64;
pub(crate) const TAPS: usize = 2 + MAX_SCREENS * 2;
pub(crate) const POINTS: usize = 128;
pub(crate) const SPAN: usize = 8;

pub(crate) fn geometry(kind: &str, width: f32) -> Option<Rect> {
    let (top, bottom) = match kind {
        "analog_vco" | "ladder_vcf" | "oscillator" | "lowpass" | "adsr" | "lfo" | "gate"
        | "gain" | "analog_vca" | "constant" | "attenuverter" | "crossfade" | "stock_vcf" => {
            (258.0, 307.0)
        }
        "bass_glitch" => (284.0, 312.0),
        "diffuse_reverb"
        | "tape_delay"
        | "stereo_ensemble"
        | "stereo_delay"
        | "stereo_reverb"
        | "sidechain_compressor"
        | "drum_voice" => (250.0, 312.0),
        "macro_voice" => (279.0, 310.0),
        "multi_vco" | "multi_lfo" => (238.0, 264.0),
        "audio_stereo" => (183.0, 215.0),
        "output" | "audio_out" => (102.0, 299.0),
        _ => return None,
    };
    Some(Rect::from_min_max(
        pos2(18.0, top),
        pos2(width - 18.0, bottom),
    ))
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Trace {
    pub values: [[f32; POINTS]; 2],
    pub cursor: usize,
    pub count: usize,
    pub active: [bool; 2],
    pub channels: u8,
    tick: usize,
    stride: usize,
}
impl Default for Trace {
    fn default() -> Self {
        Self {
            values: [[0.0; POINTS]; 2],
            cursor: 0,
            count: 0,
            active: [false; 2],
            channels: 1,
            tick: 0,
            stride: SPAN,
        }
    }
}
impl Trace {
    fn push(&mut self, samples: &[TapSample]) {
        self.tick += 1;
        if self.tick < self.stride {
            return;
        }
        self.tick = 0;
        for (lane, sample) in samples.iter().take(2).enumerate() {
            self.active[lane] = sample.active && sample.value.is_finite();
            self.values[lane][self.cursor] = if self.active[lane] { sample.value } else { 0.0 };
        }
        self.cursor = (self.cursor + 1) % POINTS;
        self.count = (self.count + 1).min(POINTS);
    }
}
#[derive(Clone)]
struct Packet {
    revision: u64,
    frame: u64,
    at: Instant,
    traces: Box<[Trace]>,
}
#[derive(Debug)]
pub(crate) struct Frame {
    pub revision: u64,
    pub traces: BTreeMap<String, Trace>,
}
pub(crate) struct Capture {
    pub taps: [Option<ResolvedTap>; TAPS],
    input: triple_buffer::Input<Packet>,
    traces: Box<[Trace]>,
}
struct Consumer {
    output: triple_buffer::Output<Packet>,
    ids: Vec<String>,
    cached: Option<Arc<Frame>>,
    last_frame: u64,
}
#[derive(Clone)]
pub(crate) struct Reader(Arc<Mutex<Consumer>>);

pub(crate) fn prepare(patch: &Patch, engine: &Engine) -> (Capture, Reader) {
    let catalog = openpatch_core::catalog();
    let mut taps = [None; TAPS];
    let mut ids = Vec::new();
    let mut traces = Vec::new();
    for node in &patch.nodes {
        if ids.len() == MAX_SCREENS {
            break;
        }
        if geometry(&node.kind, 300.0).is_none() {
            continue;
        }
        let Some(module) = catalog["modules"]
            .as_array()
            .and_then(|v| v.iter().find(|m| m["kind"] == node.kind))
        else {
            continue;
        };
        let Some(port) = module["outputs"].as_array().and_then(|v| {
            v.iter().filter_map(|v| v.as_str()).find(|p| {
                patch
                    .cables
                    .iter()
                    .any(|c| c.from == node.id && c.output == *p)
                    || patch.output.as_deref() == Some(node.id.as_str())
            })
        }) else {
            continue;
        };
        let Ok(tap) = engine.resolve_tap(&node.id, port, 0) else {
            continue;
        };
        let index = ids.len();
        taps[2 + index * 2] = Some(tap);
        let stereo = node.kind == "audio_stereo"
            || matches!(
                node.kind.as_str(),
                "diffuse_reverb"
                    | "tape_delay"
                    | "stereo_ensemble"
                    | "bass_glitch"
                    | "stereo_delay"
                    | "stereo_reverb"
                    | "sidechain_compressor"
            );
        if stereo {
            taps[3 + index * 2] = if node.kind == "audio_stereo" {
                engine.resolve_tap(&node.id, port, 1).ok()
            } else {
                engine
                    .resolve_tap(&node.id, if port == "right" { "left" } else { "right" }, 0)
                    .ok()
            };
        }
        let mut trace = Trace {
            channels: if stereo { 2 } else { 1 },
            ..Trace::default()
        };
        if matches!(node.kind.as_str(), "lfo" | "multi_lfo" | "gate") {
            let frequency = node
                .params
                .get("frequency")
                .copied()
                .unwrap_or(1.0)
                .max(0.01);
            trace.stride = ((patch.sample_rate as f32 / (frequency * POINTS as f32 * 1.8))
                as usize)
                .clamp(SPAN, patch.sample_rate as usize / 20);
        }
        traces.push(trace);
        ids.push(node.id.clone());
    }
    let initial = Packet {
        revision: 0,
        frame: 0,
        at: Instant::now(),
        traces: traces.clone().into_boxed_slice(),
    };
    let (input, output) = triple_buffer::triple_buffer(&initial);
    (
        Capture {
            taps,
            input,
            traces: traces.into_boxed_slice(),
        },
        Reader(Arc::new(Mutex::new(Consumer {
            output,
            ids,
            cached: None,
            last_frame: 0,
        }))),
    )
}
impl Capture {
    pub fn enabled(&self) -> bool {
        !self.traces.is_empty()
    }
    pub fn observe(&mut self, samples: &[[TapSample; TAPS]]) {
        for sample in samples {
            for (i, trace) in self.traces.iter_mut().enumerate() {
                trace.push(&sample[2 + i * 2..4 + i * 2]);
            }
        }
    }
    pub fn publish(&mut self, revision: u64, frame: u64) {
        let packet = self.input.input_buffer_mut();
        packet.revision = revision;
        packet.frame = frame;
        packet.at = Instant::now();
        packet.traces.copy_from_slice(&self.traces);
        self.input.publish();
    }
}
impl Reader {
    pub fn read(&self, revision: u64) -> Option<Arc<Frame>> {
        let mut c = self.0.lock().ok()?;
        c.output.update();
        let packet = c.output.output_buffer();
        if packet.frame == 0
            || packet.revision != revision
            || packet.at.elapsed() > Duration::from_millis(250)
        {
            return None;
        }
        if c.last_frame != packet.frame {
            let frame = Arc::new(Frame {
                revision,
                traces: c
                    .ids
                    .iter()
                    .cloned()
                    .zip(packet.traces.iter().copied())
                    .collect(),
            });
            c.last_frame = packet.frame;
            c.cached = Some(frame);
        }
        c.cached.clone()
    }
}
