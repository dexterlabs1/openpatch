//! Original functional, unfiltered CV oscillator. Clock acquisition and phase
//! conventions are explicit OpenPatch contracts, not sample-identical Rack DSP.
use crate::{parameter, Node, PortFrame, MAX_CHANNELS};
use serde_json::{json, Value};
use std::f64::consts::TAU;

pub(crate) const INPUTS: [&str; 4] = ["fm", "pwm", "reset", "clock"];
pub(crate) const OUTPUTS: [&str; 4] = ["sine", "triangle", "saw", "square"];
const MIN_FREQUENCY: f64 = 1e-6;
const MAX_FREQUENCY: f64 = 1024.0;

pub(crate) fn catalog() -> Value {
    json!({"kind":"multi_lfo","title":"MULTI LFO","model":"functional","description":"Four phase-coherent, unfiltered CV waveforms. At phase p: sine=sin(2πp), triangle=1−4|p−0.5|, saw=2p−1, square=+1 when p<width otherwise −1. Invert before mapping to ±5 V or 0–10 V. Emit current phase then advance; reset emits phase zero immediately. Clock sets frequency without resetting phase. Original functional implementation; no audio antialiasing or circuit claim.","inputs":INPUTS,"params":{
        "frequency":{"default":2.0,"min":1.0/256.0,"max":1024.0,"unit":"Hz","description":"Free-running Hz; when clock-locked, multiplier is frequency/2 (default 1×)"},
        "fm_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Exponential FM attenuverter, 1 V/oct at +1"},
        "pulse_width":{"default":0.5,"min":0.01,"max":0.99,"unit":"ratio"},
        "pwm_amount":{"default":0.0,"min":-1.0,"max":1.0,"description":"Width += amount × PWM/10 V, then clamp 0.01–0.99"},
        "unipolar":{"default":0.0,"min":0.0,"max":1.0,"step":1.0},
        "invert":{"default":0.0,"min":0.0,"max":1.0,"step":1.0}
    }})
}
pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_2"] = json!("FM input sets voice count, minimum one; auxiliary inputs never create voices; mono broadcasts, missing short-poly lanes are zero");
    module["polyphony"]["primary_input"] = json!("fm");
    module["frequency_policy"] = json!({"formula":"clamp(base * 2^clamp(fm_amount * FM, -64, 64), 0.000001, 1024)","free_base":"frequency","locked_base":"sample_rate / inter_edge_frames * frequency / 2","minimum_hz":MIN_FREQUENCY,"maximum_hz":MAX_FREQUENCY,"nonfinite_inputs":"zero"});
    module["clock_policy"] = json!({"thresholds_v":{"low":0.1,"high":1.0},"acquisition":"first rising edge arms; second locks to inter-edge sample frames; free-run until locked","phase":"clock never resets phase","missing_pulses":"hold last tempo; next rising edge measures the full elapsed interval","zero_channels":"clear detector and acquisition, preserve phase, resume free-running rate","short_poly_missing_lanes":"zero voltage while port remains connected; acquired tempo is held","counter":"saturating u64 elapsed sample frames, reset on each rising edge"});
    module["reset_policy"] = json!({"thresholds_v":{"low":0.1,"high":1.0},"initial_high":"one rising edge","sample_timing":"set phase to zero before output, then advance","clock_measurement":"unchanged"});
    module["state_adoption"] = json!("matching ID/kind/sample rate preserves phases with new parameters; reset and clock state independently retained only for unchanged incoming source ID/output/kind; retired and newly activated voices reset");
    module["timing"] = json!("one sample per cable; outputs share the same phase; clock changes rate without phase reset; reset affects the output sample receiving its rising edge");
    module["functional_reference"] = json!("https://vcvrack.com/Free#vcv-lfo");
    module["compatibility"] = json!("original functional implementation with documented waveform, CV, clock and reset policies; not sample-identical parity");
}
fn finite(value: f32) -> f64 {
    if value.is_finite() {
        f64::from(value)
    } else {
        0.0
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Schmitt {
    high: bool,
}
impl Schmitt {
    fn rising(&mut self, voltage: f64) -> bool {
        if self.high {
            if voltage <= f64::from(0.1_f32) {
                self.high = false;
            }
            false
        } else if voltage >= 1.0 {
            self.high = true;
            true
        } else {
            false
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Clock {
    detector: Schmitt,
    armed: bool,
    elapsed: u64,
    period: Option<u64>,
}
impl Clock {
    fn process(&mut self, voltage: f64, connected: bool) {
        if !connected {
            *self = Self::default();
            return;
        }
        if self.armed {
            self.elapsed = self.elapsed.saturating_add(1);
        }
        if self.detector.rising(voltage) {
            if self.armed {
                self.period = Some(self.elapsed.max(1));
            }
            self.armed = true;
            self.elapsed = 0;
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
struct Voice {
    phase: f64,
    reset: Schmitt,
    clock: Clock,
}
#[derive(Debug)]
pub(crate) struct MultiLfo {
    rate: f64,
    frequency: f64,
    fm_amount: f64,
    pulse_width: f64,
    pwm_amount: f64,
    unipolar: bool,
    invert: bool,
    active: u8,
    voices: [Voice; MAX_CHANNELS],
}
impl MultiLfo {
    pub(crate) fn initialize_state(&mut self) {
        for voice in &mut self.voices {
            voice.phase = 0.0;
            let detector = voice.clock.detector;
            voice.clock = Clock::default();
            voice.clock.detector = detector;
        }
    }
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for key in node.params.keys() {
            if ![
                "frequency",
                "fm_amount",
                "pulse_width",
                "pwm_amount",
                "unipolar",
                "invert",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let boolean = |name| {
            let value = parameter(node, name, 0.0, 0.0, 1.0)?;
            if value.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(value == 1.0)
        };
        Ok(Self {
            rate: f64::from(rate),
            frequency: f64::from(parameter(node, "frequency", 2.0, 1.0 / 256.0, 1024.0)?),
            fm_amount: f64::from(parameter(node, "fm_amount", 0.0, -1.0, 1.0)?),
            pulse_width: f64::from(parameter(node, "pulse_width", 0.5, 0.01, 0.99)?),
            pwm_amount: f64::from(parameter(node, "pwm_amount", 0.0, -1.0, 1.0)?),
            unipolar: boolean("unipolar")?,
            invert: boolean("invert")?,
            active: 0,
            voices: [Voice::default(); MAX_CHANNELS],
        })
    }
    pub(crate) fn adopt(&mut self, old: &Self, reset_unchanged: bool, clock_unchanged: bool) {
        self.voices = old.voices;
        self.active = old.active;
        for voice in &mut self.voices {
            if !reset_unchanged {
                voice.reset = Schmitt::default();
            }
            if !clock_unchanged {
                voice.clock = Clock::default();
            }
        }
    }
    pub(crate) fn process(&mut self, inputs: &[PortFrame; 4], outputs: &mut [PortFrame]) {
        let count = inputs[0].channels.max(1);
        for voice in
            &mut self.voices[self.active.min(count) as usize..self.active.max(count) as usize]
        {
            *voice = Voice::default();
        }
        self.active = count;
        outputs.fill(PortFrame {
            channels: count,
            ..PortFrame::default()
        });
        for (index, voice) in self.voices[..count as usize].iter_mut().enumerate() {
            voice
                .clock
                .process(finite(inputs[3].voice(index, 0.0)), inputs[3].channels > 0);
            if voice.reset.rising(finite(inputs[2].voice(index, 0.0))) {
                voice.phase = 0.0;
            }
            let width = (self.pulse_width
                + self.pwm_amount * finite(inputs[1].voice(index, 0.0)) / 10.0)
                .clamp(0.01, 0.99);
            let p = voice.phase;
            let waves = [
                (TAU * p).sin(),
                1.0 - 4.0 * (p - 0.5).abs(),
                2.0 * p - 1.0,
                if p < width { 1.0 } else { -1.0 },
            ];
            for (output, value) in outputs.iter_mut().zip(waves) {
                let value = if self.invert { -value } else { value };
                output.values[index] =
                    (5.0 * if self.unipolar { value + 1.0 } else { value }) as f32;
            }
            let base = voice
                .clock
                .period
                .map(|period| self.rate / period as f64 * self.frequency / 2.0)
                .unwrap_or(self.frequency);
            let exponent =
                (self.fm_amount * finite(inputs[0].voice(index, 0.0))).clamp(-64.0, 64.0);
            let frequency = (base * exponent.exp2()).clamp(MIN_FREQUENCY, MAX_FREQUENCY);
            voice.phase = (p + frequency / self.rate).fract();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn lfo(params: &[(&str, f32)]) -> MultiLfo {
        MultiLfo::build(
            &Node {
                content: None,
                score: None,
                bypassed: false,
                id: "lfo".into(),
                kind: "multi_lfo".into(),
                params: params.iter().map(|(k, v)| ((*k).into(), *v)).collect(),
            },
            48000,
        )
        .unwrap()
    }
    fn tick(lfo: &mut MultiLfo, inputs: &[PortFrame; 4]) -> [PortFrame; 4] {
        let mut out = [PortFrame::default(); 4];
        lfo.process(inputs, &mut out);
        out
    }
    #[test]
    fn initialize_restarts_phase_and_clock_without_rearming_reset_detector() {
        let mut lfo = lfo(&[]);
        lfo.active = 1;
        lfo.voices[0].phase = 0.7;
        lfo.voices[0].reset.high = true;
        lfo.voices[0].clock.armed = true;
        lfo.voices[0].clock.detector.high = true;
        lfo.voices[0].clock.period = Some(48);
        lfo.voices[0].clock.elapsed = 9;
        lfo.initialize_state();
        assert_eq!(lfo.voices[0].phase, 0.0);
        assert!(lfo.voices[0].reset.high);
        assert!(!lfo.voices[0].clock.armed);
        assert!(lfo.voices[0].clock.detector.high);
        assert_eq!(lfo.voices[0].clock.period, None);
        assert_eq!(lfo.voices[0].clock.elapsed, 0);
        assert_eq!(lfo.active, 1);
        lfo.voices[0].clock.process(10.0, true);
        assert!(!lfo.voices[0].clock.armed);
    }
    #[test]
    fn schmitt_thresholds_are_inclusive_at_representable_port_voltages() {
        let mut detector = Schmitt::default();
        assert!(!detector.rising(finite(0.99)));
        assert!(detector.rising(finite(1.0)));
        assert!(!detector.rising(finite(0.1001)));
        assert!(detector.high);
        assert!(!detector.rising(finite(0.1)));
        assert!(!detector.high);
        assert!(detector.rising(finite(1.0)));
        assert!(!detector.rising(finite(10.0)));
    }
    #[test]
    fn elapsed_counter_saturates_and_reacquires_after_a_long_interval() {
        let mut clock = Clock {
            armed: true,
            elapsed: u64::MAX,
            ..Clock::default()
        };
        clock.process(0.0, true);
        assert_eq!(clock.elapsed, u64::MAX);
        clock.process(10.0, true);
        assert_eq!(clock.period, Some(u64::MAX));
        assert_eq!(clock.elapsed, 0);
        clock.process(0.0, true);
        clock.process(10.0, true);
        assert_eq!(clock.period, Some(2));
        assert_eq!(clock.elapsed, 0);
        clock.process(0.0, false);
        assert!(!clock.armed);
        assert_eq!(clock.period, None);
    }
    #[test]
    fn extreme_and_nonfinite_cv_are_finite_and_frequency_bounded() {
        let mut lfo = lfo(&[("fm_amount", 1.0), ("pwm_amount", 1.0)]);
        let mut i = [PortFrame::default(); 4];
        i[0] = PortFrame::mono(f32::MAX);
        i[1] = PortFrame::mono(f32::MAX);
        let out = tick(&mut lfo, &i);
        assert_eq!(out[3].values[0], 5.0);
        assert_eq!(lfo.voices[0].phase, 1024.0 / 48000.0);
        lfo.voices[0].phase = 0.0;
        i[0].values[0] = -f32::MAX;
        tick(&mut lfo, &i);
        assert_eq!(lfo.voices[0].phase, 1e-6 / 48000.0);
        lfo.voices[0].phase = 0.0;
        i[0].values[0] = f32::NAN;
        i[1].values[0] = f32::NEG_INFINITY;
        i[2] = PortFrame::mono(f32::INFINITY);
        i[3] = PortFrame::mono(f32::NAN);
        let out = tick(&mut lfo, &i);
        assert_eq!(lfo.voices[0].phase, 2.0 / 48000.0);
        assert!(out.iter().flat_map(|f| f.values).all(f32::is_finite));
    }
    #[test]
    fn retired_and_reactivated_lanes_start_with_fresh_phase_and_detectors() {
        let mut lfo = lfo(&[]);
        let mut i = [PortFrame::default(); 4];
        i[0].channels = 4;
        i[2] = PortFrame::mono(10.0);
        i[3] = PortFrame::mono(10.0);
        for _ in 0..17 {
            tick(&mut lfo, &i);
        }
        assert!(lfo.voices[3].phase > 0.0);
        assert!(lfo.voices[3].reset.high);
        assert!(lfo.voices[3].clock.armed);
        i[0].channels = 1;
        tick(&mut lfo, &i);
        assert_eq!(lfo.voices[3].phase, 0.0);
        assert!(!lfo.voices[3].reset.high);
        assert!(!lfo.voices[3].clock.armed);
        i[0].channels = 4;
        let out = tick(&mut lfo, &i);
        assert_eq!(out[0].values[3], 0.0);
        assert!(lfo.voices[3].reset.high);
        assert!(lfo.voices[3].clock.armed);
        assert_eq!(lfo.voices[3].clock.period, None);
    }
    #[test]
    fn adopting_each_detector_is_independent_and_preserves_new_parameters() {
        let mut old = lfo(&[]);
        old.active = 1;
        old.voices[0] = Voice {
            phase: 0.37,
            reset: Schmitt { high: true },
            clock: Clock {
                detector: Schmitt { high: true },
                armed: true,
                elapsed: 17,
                period: Some(100),
            },
        };
        let mut reset_changed = lfo(&[("frequency", 4.0)]);
        reset_changed.adopt(&old, false, true);
        assert_eq!(reset_changed.frequency, 4.0);
        assert_eq!(reset_changed.voices[0].phase, 0.37);
        assert!(!reset_changed.voices[0].reset.high);
        assert_eq!(reset_changed.voices[0].clock.period, Some(100));
        let mut clock_changed = lfo(&[]);
        clock_changed.adopt(&old, true, false);
        assert_eq!(clock_changed.voices[0].phase, 0.37);
        assert!(clock_changed.voices[0].reset.high);
        assert_eq!(clock_changed.voices[0].clock.period, None);
        assert!(!clock_changed.voices[0].clock.armed);
        // Unknown/nonfinite parameters reject on the control thread.
        let n = Node {
            content: None,
            score: None,
            bypassed: false,
            id: "bad".into(),
            kind: "multi_lfo".into(),
            params: BTreeMap::from([("frequency".into(), f32::NAN)]),
        };
        assert!(MultiLfo::build(&n, 48000).is_err());
    }
}
