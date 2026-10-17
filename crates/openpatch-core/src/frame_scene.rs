//! Source integer keyframing with an explicit ideal native VCA/mix path.
// The complete source authoring API is retained for the independent raw oracle.
#[allow(dead_code)]
mod kernel;
mod native_gain;

use crate::{parameter, FrameSceneData, ModuleContent, ModuleLightState, Node, PortFrame};
use kernel::{ChannelSettings, EasingCurve, FrameKernel, Keyframe};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 6] = ["all", "in1", "in2", "in3", "in4", "frame_cv"];
pub(crate) const OUTPUTS: [&str; 6] = ["out1", "out2", "out3", "out4", "mix", "step"];

pub(crate) fn catalog() -> Value {
    let mut value = json!({"kind":"frame_scene","title":"FRAME SCENE","model":"functional",
    "tags":["Custom","Mixing","Modulation"],"inputs":INPUTS,"width_hp":36,
    "description":"Animate four gain/CV tracks across up to 64 stored keyframes. Six easing curves, adjustable response, normalized mixing and a frame-crossing pulse.",
    "content_schema":FrameSceneData::content_schema(),"params":{
        "frame":{"default":0.0,"min":0.0,"max":1.0},
        "modulation":{"default":1.0,"min":-1.0,"max":1.0},
        "offset":{"default":10.0,"min":0.0,"max":10.0,"values":[0,5,10],"labels":["Off","+5 V","+10 V"]}
    }});
    for (i, default) in [1.0, 0.75, 0.5, 0.25].into_iter().enumerate() {
        value["params"][format!("level{}", i + 1)] = json!({"default":default,"min":0.0,"max":1.0,"description":"Immediate level when no keyframes exist. Stored frame values are edited by stable ID through edit_frame_scene/content."});
        value["params"][format!("easing{}", i + 1)] = json!({"default":1,"min":0,"max":5,"step":1,"values":[0,1,2,3,4,5],"labels":["Step","Linear","Accelerating","Decelerating","Smooth","Bounce"]});
        value["params"][format!("response{}", i + 1)] = json!({"default":0.0,"min":0.0,"max":1.0,"description":"Linear-to-exponential source DAC response; applied by the declared ideal inverse VCA transfer."});
    }
    value
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/frames");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!("Six mono outputs; inputs accept up to 16 channels, read channel zero, and use physical cable presence for normaling.");
    for output in module["output_metadata"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        output["maximum_channels"] = json!(1);
    }
    module["frame_policy"] = json!("round(clamp(frame + channel0(frame_cv)/10 * modulation,0,1)*65535), evaluated per sample when the quantized timestamp changes; keyframe content compiled to fixed 64-entry storage.");
    module["analog_policy"] = json!("Ideal unity-range VCA transfer H(sourceDAC(level,response)). H is the monotone inverse of the source response-zero compensation curve: first gain whose DAC code<=d, exact zero/full endpoints, quantized16-bit gain. Not measured analog circuitry or DAC volts.");
    module["normaling_policy"] = json!("Unpatched ALL normalizes to offset (0/5/10 V); each unpatched channel normalizes to ALL. Any physical cable breaks its normalization, including a zero-channel source. MIX sums only individual outputs without a physical cable; inspecting/tapping/selecting an output does not change MIX.");
    module["step_policy"] = json!("5 V pulse for ceil(sample_rate/1000) frames when scanning across any authored timestamp in either direction, excluding the previous endpoint and including the new one. Jumps across several keys produce one pulse; first observation and content replacement do not invent crossings.");
    module["safety_policy"] = json!("Native guarded endpoints/removal avoid source array-overrun domains. Curved scale 65536 uses the final table entry without reading beyond it. Native wrapping extends undefined signed-overflow bounce arithmetic; source equivalence only covers sanitizer-audited defined cases. Inputs and mix bound to +/-1000000 V.");
    module["light_names"] = json!([
        "gain1",
        "gain2",
        "gain3",
        "gain4",
        "frame_red",
        "frame_green",
        "frame_blue",
        "step"
    ]);
    module["light_policy"] = json!("Actual ideal transfer gains, evaluated source palette RGB and step pulse. waveform_position is the actual sampled frame position; unavailable before processing.");
    module["initialization_policy"] = json!("Initialize resets runtime observations/pulse while retaining authored keyframes and controls. Same-rate adoption retains pulse/scan position only when scene keys are unchanged; new controls always apply. Saved reopen starts fresh. Bypass emits mono zero and freezes runtime.");
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

#[derive(Clone, Debug, Default)]
struct Runtime {
    last_timestamp: Option<u16>,
    evaluated_timestamp: Option<u16>,
    step_remaining: u32,
    lights: [[f32; 2]; 8],
}

#[derive(Debug)]
pub(crate) struct FrameScene {
    sample_rate: u32,
    frame: f32,
    modulation: f32,
    offset: f32,
    kernel: FrameKernel,
    runtime: Runtime,
}

impl FrameScene {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("Frame Scene sample rate outside 8000..192000".into());
        }
        for key in node.params.keys() {
            if ![
                "frame",
                "modulation",
                "offset",
                "level1",
                "level2",
                "level3",
                "level4",
                "easing1",
                "easing2",
                "easing3",
                "easing4",
                "response1",
                "response2",
                "response3",
                "response4",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("Frame Scene unknown parameter '{key}'"));
            }
        }
        crate::validate_content(node)?;
        let mut kernel = FrameKernel::new();
        if let Some(ModuleContent::FrameScene(data)) = &node.content {
            let mut keys = [Keyframe::default(); 64];
            for (slot, key) in keys.iter_mut().zip(&data.keyframes) {
                *slot = Keyframe {
                    id: key.id,
                    timestamp: key.timestamp,
                    values: key.values,
                };
            }
            if !kernel.load_keyframes(&keys[..data.keyframes.len()]) {
                return Err("Invalid Frame Scene keyframes".into());
            }
        }
        for (i, default) in [1.0, 0.75, 0.5, 0.25].into_iter().enumerate() {
            let level = parameter(node, &format!("level{}", i + 1), default, 0.0, 1.0)?;
            kernel.set_immediate(i, (level * 65535.0).round() as u16);
            let easing = parameter(node, &format!("easing{}", i + 1), 1.0, 0.0, 5.0)?;
            if easing.fract() != 0.0 {
                return Err("Frame Scene easing must be an integer 0..5".into());
            }
            let response = parameter(node, &format!("response{}", i + 1), 0.0, 0.0, 1.0)?;
            kernel.set_settings(
                i,
                ChannelSettings {
                    easing_curve: EasingCurve::from_index(easing as u8),
                    response: (response * 255.0).round() as u8,
                },
            );
        }
        let offset = parameter(node, "offset", 10.0, 0.0, 10.0)?;
        if ![0.0, 5.0, 10.0].contains(&offset) {
            return Err("Frame Scene offset must be 0,5 or 10 V".into());
        }
        Ok(Self {
            sample_rate,
            frame: parameter(node, "frame", 0.0, 0.0, 1.0)?,
            modulation: parameter(node, "modulation", 1.0, -1.0, 1.0)?,
            offset,
            kernel,
            runtime: Runtime::default(),
        })
    }

    pub(crate) fn adopt(&mut self, old: &Self) {
        let same_keys = self.kernel.num_keyframes() == old.kernel.num_keyframes()
            && (0..self.kernel.num_keyframes())
                .all(|i| self.kernel.keyframe(i) == old.kernel.keyframe(i));
        if self.sample_rate == old.sample_rate && same_keys {
            self.runtime.clone_from(&old.runtime);
            self.runtime.evaluated_timestamp = None;
        }
    }

    pub(crate) fn initialize_state(&mut self) {
        self.runtime = Runtime::default();
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut lights = ModuleLightState {
            count: 8,
            waveform_position: self.runtime.last_timestamp.map(|t| t as f32 / 65535.0),
            ..Default::default()
        };
        lights.brightness[..8].copy_from_slice(&self.runtime.lights);
        lights
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 6],
        physical_inputs: u16,
        physical_outputs: u32,
        demand: u16,
        outputs: &mut [PortFrame],
    ) {
        let frame =
            (self.frame + finite(inputs[5].channel(0)) / 10.0 * self.modulation).clamp(0.0, 1.0);
        let timestamp = (frame * 65535.0).round() as u16;
        if self.runtime.evaluated_timestamp != Some(timestamp) {
            self.kernel.evaluate(timestamp);
            self.runtime.evaluated_timestamp = Some(timestamp);
        }
        if let Some(previous) = self.runtime.last_timestamp {
            if previous != timestamp
                && (0..self.kernel.num_keyframes()).any(|i| {
                    let key = self.kernel.keyframe(i).unwrap().timestamp;
                    if previous < timestamp {
                        previous < key && key <= timestamp
                    } else {
                        timestamp <= key && key < previous
                    }
                })
            {
                self.runtime.step_remaining = self.sample_rate.div_ceil(1000);
            }
        }
        self.runtime.last_timestamp = Some(timestamp);
        let evaluated = self.kernel.output();
        let all = if physical_inputs & 1 != 0 {
            finite(inputs[0].channel(0))
        } else {
            self.offset
        }
        .clamp(-1e6, 1e6);
        let mut values = [0.0; 6];
        let mut mix = 0.0_f64;
        for i in 0..4 {
            let gain = native_gain::IDEAL_GAIN[evaluated.dac_code[i] as usize] as f32 / 65535.0;
            let signal = if physical_inputs & (1 << (i + 1)) != 0 {
                finite(inputs[i + 1].channel(0))
            } else {
                all
            }
            .clamp(-1e6, 1e6);
            values[i] = signal * gain;
            self.runtime.lights[i] = [0.0, gain];
            if physical_outputs & (1 << i) == 0 {
                mix += f64::from(values[i]);
            }
        }
        values[4] = mix.clamp(-1e6, 1e6) as f32;
        values[5] = if self.runtime.step_remaining > 0 {
            5.0
        } else {
            0.0
        };
        self.runtime.step_remaining = self.runtime.step_remaining.saturating_sub(1);
        for (i, color) in evaluated.color.iter().enumerate() {
            self.runtime.lights[i + 4] = [0.0, *color as f32 / 255.0];
        }
        self.runtime.lights[7] = [0.0, values[5] / 5.0];
        for (i, output) in outputs.iter_mut().enumerate() {
            *output = if demand & (1 << i) != 0 {
                PortFrame::mono(values[i])
            } else {
                PortFrame::default()
            };
        }
    }
}
