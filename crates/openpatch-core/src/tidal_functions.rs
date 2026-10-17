//! Native six-frame host adapter for the pinned Tides 2018 portable kernels.
//! Source arithmetic lives in kernel/extractor; voltage scaling, block timing,
//! Schmitt thresholds and safe clock startup are explicit native policies.
mod extractor;
mod kernel;

use crate::{parameter, ModuleLightState, Node, PortFrame};
use extractor::RampExtractor;
use kernel::TidalKernel;
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 7] = [
    "pitch",
    "slope_cv",
    "shape_cv",
    "smoothness_cv",
    "shift_cv",
    "trigger",
    "clock",
];
pub(crate) const OUTPUTS: [&str; 4] = ["out1", "out2", "out3", "out4"];
const BLOCK: usize = 6;
const RATIOS: [(f32, i32); 9] = [
    (0.0625, 16),
    (0.125, 8),
    (0.25, 4),
    (0.5, 2),
    (1.0, 1),
    (2.0, 1),
    (4.0, 1),
    (8.0, 1),
    (16.0, 1),
];

pub(crate) fn catalog() -> Value {
    json!({"kind":"tidal_functions","title":"TIDAL FUNCTIONS","model":"functional",
        "tags":["Custom","Modulation","Envelopes","Oscillators"],
    "description":"Four related Tides 2018 functions from a pinned portable source kernel, with explicit native CV and six-frame timing policies.",
    "inputs":INPUTS,"params":{
        "frequency":{"default":2.0,"min":0.001,"max":20000.0,"unit":"Hz"},
        "slope":{"default":0.5,"min":0.0,"max":1.0},
        "shape":{"default":0.5,"min":0.0,"max":1.0},
        "smoothness":{"default":0.5,"min":0.0,"max":1.0},
        "shift":{"default":0.6,"min":0.0,"max":1.0},
        "ramp_mode":{"default":1.0,"min":0.0,"max":2.0,"step":1.0,"values":[0,1,2],"labels":["AD","Loop","AR"]},
        "output_mode":{"default":1.0,"min":0.0,"max":3.0,"step":1.0,"values":[0,1,2,3],"labels":["Gates","Amplitude","Slope phase","Frequency"]},
        "range":{"default":0.0,"min":0.0,"max":1.0,"step":1.0,"values":[0,1],"labels":["Control","Audio"]},
        "clock_ratio":{"default":4.0,"min":0.0,"max":8.0,"step":1.0,"values":[0,1,2,3,4,5,6,7,8],"labels":["1/16","1/8","1/4","1/2","1","2","4","8","16"]}
    }})
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://github.com/pichenettes/eurorack/tree/f4cfa7697095a28d292bff758beb51deb49b5a86/tides2");
    module["polyphony"]["version_2"] = json!("Channel zero only; four related mono outputs. Connected consumers expose one channel; unused host storage is zero.");
    module["polyphony"]["maximum_channels"] = json!(1);
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["auxiliary_inputs"] = json!("Inputs accept up to sixteen channels and read only channel zero; an absent input supplies zero volts.");
    for output in module["output_metadata"]
        .as_array_mut()
        .into_iter()
        .flatten()
    {
        output["maximum_channels"] = json!(1);
    }
    module["processing_policy"] = json!("Fixed six-frame blocks at the host sample rate with exactly six frames of pipeline delay. Scalar controls sample the last frame of each block. No firmware half-speed cadence or DAC calibration is implied.");
    module["cv_policy"] = json!("Pitch is 1 V/oct, bounded to +/-32 octaves before exp2; effective free frequency is bounded to 0.001 Hz..sample_rate/4. Other CV adds voltage/5 to its normalized knob and clamps to 0..1. Nonfinite native inputs read zero.");
    module["clock_policy"] = json!("Native 0.1/1 V Schmitt. First clock sample after extractor reset primes the actual level and suppresses RISING before processing. Clock remains tracked with trigger patched; only the external ramp is withheld from Render when trigger is patched.");
    module["connection_policy"] = json!("Trigger/clock connection changes discard partial and queued blocks, then emit six silent frames while preserving generator phase. Clock connection changes reset the extractor. Ordinary CV connection changes preserve the pipeline.");
    module["initialization_policy"] = json!("Initialize and sample-rate changes create fresh fixed-size generator/extractor/pipeline state. Output-mode edits reset generator filters only. Same-rate adoption preserves bounded runtime with receiving controls.");
    module["light_names"] = json!(["out1", "out2", "out3", "out4"]);
    module["light_policy"] = json!("Four bipolar components from actual delayed generated volts: negative/positive magnitude divided by 8 and clamped to 0..1, independent of output consumers.");
    module["voltage_policy"] = json!("Unity gain from source output floats to native volts. This portable-source adaptation does not claim calibrated hardware fidelity.");
}

fn finite(value: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Schmitt {
    high: bool,
}
impl Schmitt {
    fn flags(&mut self, voltage: f32, prime: bool) -> u8 {
        let was_high = self.high;
        let voltage = finite(voltage);
        if voltage >= 1.0 {
            self.high = true;
        } else if voltage <= 0.1 {
            self.high = false;
        }
        u8::from(self.high)
            | if !prime && !was_high && self.high {
                2
            } else {
                0
            }
            | if !prime && was_high && !self.high {
                4
            } else {
                0
            }
    }
}

#[derive(Clone, Debug)]
struct Runtime {
    generator: TidalKernel,
    extractor: RampExtractor,
    cursor: usize,
    gates: [u8; BLOCK],
    clocks: [u8; BLOCK],
    queued: [[f32; 4]; BLOCK],
    trigger: Schmitt,
    clock: Schmitt,
    connections: Option<u8>,
    prime_detectors: bool,
    lights: [[f32; 2]; 4],
}
impl Runtime {
    fn new(sample_rate: u32) -> Self {
        Self {
            generator: TidalKernel::new(),
            extractor: RampExtractor::new(sample_rate as f32, 40.0 / sample_rate as f32),
            cursor: 0,
            gates: [0; BLOCK],
            clocks: [0; BLOCK],
            queued: [[0.0; 4]; BLOCK],
            trigger: Schmitt::default(),
            clock: Schmitt::default(),
            connections: None,
            prime_detectors: true,
            lights: [[0.0; 2]; 4],
        }
    }

    fn discard_pipeline(&mut self) {
        self.cursor = 0;
        self.gates.fill(0);
        self.clocks.fill(0);
        self.queued.fill([0.0; 4]);
        self.prime_detectors = true;
    }
}

#[derive(Debug)]
pub(crate) struct TidalFunctions {
    sample_rate: u32,
    frequency: f32,
    controls: [f32; 4],
    ramp_mode: u8,
    output_mode: u8,
    range: u8,
    clock_ratio: usize,
    runtime: Runtime,
}

impl TidalFunctions {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&sample_rate) {
            return Err("tidal_functions sample rate must be in 8000..=192000 Hz".into());
        }
        for key in node.params.keys() {
            if ![
                "frequency",
                "slope",
                "shape",
                "smoothness",
                "shift",
                "ramp_mode",
                "output_mode",
                "range",
                "clock_ratio",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let discrete = |name, default, maximum| -> Result<u8, String> {
            let value = parameter(node, name, default, 0.0, maximum)?;
            if value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(value as u8)
        };
        Ok(Self {
            sample_rate,
            frequency: parameter(node, "frequency", 2.0, 0.001, 20000.0)?,
            controls: [
                parameter(node, "slope", 0.5, 0.0, 1.0)?,
                parameter(node, "shape", 0.5, 0.0, 1.0)?,
                parameter(node, "smoothness", 0.5, 0.0, 1.0)?,
                parameter(node, "shift", 0.6, 0.0, 1.0)?,
            ],
            ramp_mode: discrete("ramp_mode", 1.0, 2.0)?,
            output_mode: discrete("output_mode", 1.0, 3.0)?,
            range: discrete("range", 0.0, 1.0)?,
            clock_ratio: discrete("clock_ratio", 4.0, 8.0)? as usize,
            runtime: Runtime::new(sample_rate),
        })
    }

    pub(crate) fn adopt(&mut self, previous: &Self) {
        if self.sample_rate == previous.sample_rate {
            self.runtime.clone_from(&previous.runtime);
            if self.output_mode != previous.output_mode {
                self.runtime.generator.reset_filter();
            }
        }
    }

    pub(crate) fn initialize_state(&mut self) {
        self.runtime = Runtime::new(self.sample_rate);
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut result = ModuleLightState {
            count: 4,
            ..ModuleLightState::default()
        };
        result.brightness[..4].copy_from_slice(&self.runtime.lights);
        result
    }

    pub(crate) fn process(
        &mut self,
        inputs: &[PortFrame; 7],
        connected_mask: u16,
        outputs: &mut [PortFrame],
    ) {
        let connections =
            u8::from(inputs[5].channels != 0) | (u8::from(inputs[6].channels != 0) << 1);
        if self.runtime.connections != Some(connections) {
            if self
                .runtime
                .connections
                .is_none_or(|previous| previous & 2 != connections & 2)
            {
                self.runtime.extractor.reset();
            }
            self.runtime.discard_pipeline();
            self.runtime.connections = Some(connections);
        }
        let index = self.runtime.cursor;
        let generated = self.runtime.queued[index];
        for (channel, output) in outputs.iter_mut().enumerate() {
            *output = PortFrame::default();
            if channel < 4 {
                let voltage = generated[channel];
                if connected_mask & (1 << channel) != 0 {
                    *output = PortFrame::mono(voltage);
                }
                self.runtime.lights[channel] = [
                    (-voltage / 8.0).clamp(0.0, 1.0),
                    (voltage / 8.0).clamp(0.0, 1.0),
                ];
            }
        }
        let prime = self.runtime.prime_detectors;
        self.runtime.gates[index] = self.runtime.trigger.flags(
            if connections & 1 != 0 {
                inputs[5].channel(0)
            } else {
                0.0
            },
            prime,
        );
        self.runtime.clocks[index] = self.runtime.clock.flags(
            if connections & 2 != 0 {
                inputs[6].channel(0)
            } else {
                0.0
            },
            prime,
        );
        self.runtime.prime_detectors = false;
        self.runtime.cursor += 1;
        if self.runtime.cursor == BLOCK {
            self.render_block(inputs, connections);
            self.runtime.cursor = 0;
        }
    }

    fn render_block(&mut self, inputs: &[PortFrame; 7], connections: u8) {
        let rate = self.sample_rate as f32;
        let pitch = finite(inputs[0].channel(0)).clamp(-32.0, 32.0);
        let mut frequency = (self.frequency * pitch.exp2()).clamp(0.001, rate * 0.25) / rate;
        let controls: [f32; 4] = std::array::from_fn(|i| {
            (self.controls[i] + finite(inputs[i + 1].channel(0)) / 5.0).clamp(0.0, 1.0)
        });
        let mut ramp = [0.0; BLOCK];
        if connections & 2 != 0 {
            let (ratio, denominator) = RATIOS[self.clock_ratio];
            frequency = self.runtime.extractor.process(
                self.range == 1,
                self.range == 1 && self.ramp_mode == 2,
                ratio,
                denominator,
                &self.runtime.clocks,
                &mut ramp,
            );
        }
        self.runtime.generator.render(
            self.ramp_mode,
            self.output_mode,
            self.range,
            frequency,
            controls[0],
            controls[1],
            controls[2],
            controls[3],
            &self.runtime.gates,
            if connections == 2 { Some(&ramp) } else { None },
            &mut self.runtime.queued,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(params: &[(&str, f32)], rate: u32) -> TidalFunctions {
        TidalFunctions::build(
            &Node {
                id: "tidal".into(),
                kind: "tidal_functions".into(),
                params: params
                    .iter()
                    .map(|(key, value)| ((*key).into(), *value))
                    .collect(),
                content: None,
                score: None,
                rackmcp_sequence: None,
                bypassed: false,
            },
            rate,
        )
        .unwrap()
    }
    fn step(model: &mut TidalFunctions, inputs: &[PortFrame; 7], mask: u16) -> [PortFrame; 4] {
        let mut outputs = [PortFrame::default(); 4];
        model.process(inputs, mask, &mut outputs);
        outputs
    }

    #[test]
    fn six_frame_delay_and_unity_source_volts_are_exact() {
        let mut model = model(&[], 48000);
        let inputs = [PortFrame::default(); 7];
        let mut kernel = TidalKernel::new();
        let mut expected = [[0.0; 4]; BLOCK];
        kernel.render(
            1,
            1,
            0,
            2.0 / 48000.0,
            0.5,
            0.5,
            0.5,
            0.6,
            &[0; BLOCK],
            None,
            &mut expected,
        );
        for _ in 0..BLOCK {
            assert_eq!(step(&mut model, &inputs, 15), [PortFrame::mono(0.0); 4]);
        }
        for sample in expected {
            let actual = step(&mut model, &inputs, 15);
            for channel in 0..4 {
                assert_eq!(
                    actual[channel].channel(0).to_bits(),
                    sample[channel].to_bits()
                );
            }
        }
    }

    #[test]
    fn scalar_controls_use_block_endpoint_and_five_volt_cv() {
        let mut first = model(&[], 48000);
        let mut second = model(&[], 48000);
        let mut inputs = [PortFrame::default(); 7];
        inputs[1] = PortFrame::mono(2.5);
        for _ in 0..BLOCK - 1 {
            step(&mut first, &inputs, 15);
            step(&mut second, &[PortFrame::default(); 7], 15);
        }
        inputs[1] = PortFrame::mono(-2.5);
        step(&mut first, &inputs, 15);
        step(&mut second, &inputs, 15);
        assert_eq!(
            first.runtime.queued, second.runtime.queued,
            "only the final CV sample defines block controls"
        );
        let mut kernel = TidalKernel::new();
        let mut expected = [[0.0; 4]; BLOCK];
        kernel.render(
            1,
            1,
            0,
            2.0 / 48000.0,
            0.0,
            0.5,
            0.5,
            0.6,
            &[0; BLOCK],
            None,
            &mut expected,
        );
        assert_eq!(first.runtime.queued, expected);
    }

    #[test]
    fn first_high_clock_and_reconnect_are_finite_for_control_and_audio_modes() {
        for range in [0.0, 1.0] {
            let mut model = model(&[("range", range), ("ramp_mode", 2.0)], 48000);
            let mut inputs = [PortFrame::default(); 7];
            inputs[6] = PortFrame::mono(10.0);
            for frame in 0..96 {
                if frame == 30 {
                    inputs[6] = PortFrame::default();
                }
                if frame == 33 {
                    inputs[6] = PortFrame::mono(10.0);
                }
                if frame > 40 {
                    inputs[6] = PortFrame::mono(if frame % 12 < 6 { 10.0 } else { 0.0 });
                }
                assert!(step(&mut model, &inputs, 15)
                    .iter()
                    .all(|output| output.values.iter().all(|value| value.is_finite())));
            }
        }
    }

    #[test]
    fn native_clock_start_is_finite_across_rates_modes_and_all_ratios() {
        for rate in [8000, 48000, 192000] {
            for range in [0.0, 1.0] {
                for ramp_mode in [0.0, 1.0, 2.0] {
                    for output_mode in [0.0, 1.0, 2.0, 3.0] {
                        for ratio in 0..9 {
                            let mut model = model(
                                &[
                                    ("range", range),
                                    ("ramp_mode", ramp_mode),
                                    ("output_mode", output_mode),
                                    ("clock_ratio", ratio as f32),
                                ],
                                rate,
                            );
                            let mut inputs = [PortFrame::default(); 7];
                            for frame in 0..96 {
                                inputs[5] = PortFrame::mono(if frame % 7 < 3 { 10.0 } else { 0.0 });
                                inputs[6] =
                                    PortFrame::mono(if frame % 2 == 0 { 10.0 } else { 0.0 });
                                assert!(step(&mut model, &inputs, 15).iter().all(|out| out.values.iter().all(|v| v.is_finite())),
                                    "rate={rate},range={range},ramp={ramp_mode},output={output_mode},ratio={ratio},frame={frame}");
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn patched_clock_still_controls_frequency_when_trigger_is_patched() {
        for range in [0.0, 1.0] {
            let mut slow = model(&[("frequency", 2.0), ("range", range)], 48000);
            let mut fast = model(&[("frequency", 1000.0), ("range", range)], 48000);
            let mut unclocked = model(&[("frequency", 2.0), ("range", range)], 48000);
            let mut differs_from_unclocked = false;
            for frame in 0..1500 {
                let mut inputs = [PortFrame::default(); 7];
                inputs[5] = PortFrame::mono(if frame % 211 < 19 { 10.0 } else { 0.0 });
                inputs[6] = PortFrame::mono(if frame % 96 < 24 { 10.0 } else { 0.0 });
                let a = step(&mut slow, &inputs, 15);
                let b = step(&mut fast, &inputs, 15);
                assert_eq!(
                    a, b,
                    "clock overrides the free Hz knob even with trigger patched"
                );
                inputs[6] = PortFrame::default();
                differs_from_unclocked |= a != step(&mut unclocked, &inputs, 15);
            }
            assert!(differs_from_unclocked);
        }
    }

    #[test]
    fn trigger_clock_connection_changes_flush_but_cv_connections_preserve_pipeline() {
        let mut model = model(&[], 48000);
        let mut inputs = [PortFrame::default(); 7];
        for _ in 0..10 {
            step(&mut model, &inputs, 15);
        }
        let cursor = model.runtime.cursor;
        inputs[1] = PortFrame::mono(0.0);
        step(&mut model, &inputs, 15);
        assert_eq!(model.runtime.cursor, cursor + 1);
        inputs[5] = PortFrame::mono(10.0);
        for _ in 0..BLOCK {
            assert_eq!(step(&mut model, &inputs, 15), [PortFrame::mono(0.0); 4]);
        }
        assert_eq!(
            model.runtime.gates[0], 1,
            "connected high primes without inventing an edge"
        );
    }

    #[test]
    fn adoption_preserves_pipeline_new_controls_win_and_initialize_is_fresh() {
        let mut previous = model(&[], 48000);
        let inputs = [PortFrame::default(); 7];
        for _ in 0..17 {
            step(&mut previous, &inputs, 15);
        }
        let mut current = model(&[], 48000);
        current.adopt(&previous);
        for _ in 0..24 {
            assert_eq!(
                step(&mut current, &inputs, 15),
                step(&mut previous, &inputs, 15)
            );
        }
        let mut edited = model(&[("frequency", 200.0)], 48000);
        edited.adopt(&previous);
        assert_eq!(edited.frequency, 200.0);
        edited.initialize_state();
        let mut fresh = model(&[("frequency", 200.0)], 48000);
        for _ in 0..24 {
            assert_eq!(
                step(&mut edited, &inputs, 15),
                step(&mut fresh, &inputs, 15)
            );
        }
        let mut changed_rate = model(&[], 96000);
        changed_rate.adopt(&previous);
        assert_eq!(changed_rate.runtime.cursor, 0);
    }

    #[test]
    fn output_masks_padding_nonfinite_cv_and_channel_zero_are_explicit() {
        let mut model = model(&[], 48000);
        let mut inputs = [PortFrame::default(); 7];
        inputs[0] = PortFrame::mono(f32::MAX);
        inputs[1] = PortFrame::mono(f32::NAN);
        inputs[2] = PortFrame::mono(f32::INFINITY);
        inputs[0].channels = 16;
        inputs[0].values[1] = f32::NAN;
        for _ in 0..48 {
            let outputs = step(&mut model, &inputs, 5);
            for (index, output) in outputs.iter().enumerate() {
                assert_eq!(output.channels, u8::from(index % 2 == 0));
                assert!(output.values.iter().all(|value| value.is_finite()));
                assert_eq!(output.values[1..], [0.0; 15]);
            }
        }
    }
}
