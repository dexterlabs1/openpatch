//! Control/audio boundary for the optional original Rack Plaits child process.
//! This module contains no Rack code. Hosts attach a nonblocking live endpoint.
use crate::{parameter, Node, PortFrame};
use serde_json::{json, Value};

pub const RACK_PLAITS_INPUTS: [&str; 7] = [
    "pitch",
    "trigger",
    "harmonics",
    "timbre",
    "morph",
    "fm",
    "level",
];
pub const RACK_PLAITS_LATENCY: u64 = 4096;
pub const RACK_PLAITS_ENGINES: [&str; 16] = [
    "Virtual analog",
    "Waveshaping",
    "FM",
    "Granular formant",
    "Harmonic",
    "Wavetable",
    "Chords",
    "Speech",
    "Swarm",
    "Filtered noise",
    "Particle noise",
    "String",
    "Modal",
    "Bass drum",
    "Snare drum",
    "Hi-hat",
];

/// The audio thread may call only these bounded, allocation-free methods.
/// The host owns all child-process I/O and lifetime on its control/background
/// threads. Activate switches sole-producer ownership at graph adoption.
pub trait RackPlaitsEndpoint: std::fmt::Debug + Send {
    fn activate(&mut self);
    fn process(&mut self, controls: &[f32; 6], inputs: &[PortFrame], outputs: &mut [PortFrame]);
}

pub(crate) fn catalog() -> Value {
    json!({"kind":"rack_plaits", "title":"RACK PLAITS", "model":"external",
        "description":"Original Audible Instruments Plaits binary in an isolated optional Rack worker. Sixteen original engines; polyphonic CV; Main and Aux outputs. Requires configured local Rack worker. Experimental live bridge adds 4096 samples of latency. Offline rendering requires an attached host endpoint.",
        "inputs":RACK_PLAITS_INPUTS,"params":{
            "engine":{"default":4,"min":0,"max":15,"step":1,"values":[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15],"labels":RACK_PLAITS_ENGINES},
            "frequency":{"default":0,"min":-4,"max":4,"unit":"oct"},
            "harmonics":{"default":0.3,"min":0,"max":1},
            "timbre":{"default":0.35,"min":0,"max":1},
            "morph":{"default":0.5,"min":0,"max":1},
            "decay":{"default":0.6,"min":0,"max":1},
            "level":{"default":0.8,"min":0,"max":1},
            "fm_amount":{"default":1,"min":-1,"max":1},
            "timbre_amount":{"default":1,"min":-1,"max":1},
            "morph_amount":{"default":1,"min":-1,"max":1}
        }, "external_runtime":true, "latency_samples":RACK_PLAITS_LATENCY})
}

#[derive(Debug)]
pub(crate) struct RackPlaits {
    controls: [f32; 6],
    level: f32,
    cv_amounts: [f32; 3],
    pub(crate) endpoint: Option<Box<dyn RackPlaitsEndpoint>>,
}
impl RackPlaits {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        let names = [
            "engine",
            "frequency",
            "harmonics",
            "timbre",
            "morph",
            "decay",
            "level",
            "fm_amount",
            "timbre_amount",
            "morph_amount",
        ];
        for key in node.params.keys() {
            if !names.contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let engine = parameter(node, "engine", 4.0, 0.0, 15.0)?;
        if engine.fract() != 0.0 {
            return Err("Rack Plaits engine must be an integer".into());
        }
        Ok(Self {
            controls: [
                engine,
                parameter(node, "frequency", 0.0, -4.0, 4.0)?,
                parameter(node, "harmonics", 0.3, 0.0, 1.0)?,
                parameter(node, "timbre", 0.35, 0.0, 1.0)?,
                parameter(node, "morph", 0.5, 0.0, 1.0)?,
                parameter(node, "decay", 0.6, 0.0, 1.0)?,
            ],
            level: parameter(node, "level", 0.8, 0.0, 1.0)?,
            cv_amounts: [
                parameter(node, "fm_amount", 1.0, -1.0, 1.0)?,
                parameter(node, "timbre_amount", 1.0, -1.0, 1.0)?,
                parameter(node, "morph_amount", 1.0, -1.0, 1.0)?,
            ],
            endpoint: None,
        })
    }
    pub(crate) fn adopt(&mut self) {
        if let Some(endpoint) = &mut self.endpoint {
            endpoint.activate();
        }
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame], outputs: &mut [PortFrame]) {
        outputs.fill(PortFrame::default());
        if let Some(endpoint) = &mut self.endpoint {
            let mut modulated: [PortFrame; 7] =
                std::array::from_fn(|i| inputs.get(i).copied().unwrap_or_default());
            for (index, amount) in [5, 3, 4].into_iter().zip(self.cv_amounts) {
                for value in &mut modulated[index].values {
                    *value *= amount;
                }
            }
            endpoint.process(&self.controls, &modulated, outputs);
            for output in outputs {
                for value in &mut output.values {
                    *value *= self.level;
                }
            }
        }
    }
}
