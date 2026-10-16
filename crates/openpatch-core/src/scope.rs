//! Original polyphonic scope acquisition. Display transformations never affect audio.
use crate::{parameter, Node, PortFrame};
use serde_json::{json, Value};
pub const SCOPE_BINS: usize = 256;
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScopePoint {
    pub min: f32,
    pub max: f32,
}
impl Default for ScopePoint {
    fn default() -> Self {
        Self {
            min: f32::INFINITY,
            max: f32::NEG_INFINITY,
        }
    }
}
#[derive(Clone, Debug)]
pub struct ScopeSnapshot {
    pub x: [[ScopePoint; SCOPE_BINS]; 16],
    pub y: [[ScopePoint; SCOPE_BINS]; 16],
    pub x_channels: u8,
    pub y_channels: u8,
    /// Actual external trigger width; zero means trigger normals to X.
    pub trigger_input_channels: u8,
    pub buffer_index: usize,
    pub samples_per_bin: u32,
    pub sample_index: u32,
    pub waiting: bool,
    pub xy: bool,
    pub x_scale: u8,
    pub y_scale: u8,
    pub x_offset: f32,
    pub y_offset: f32,
    pub time: f32,
    pub trigger_enabled: bool,
    pub threshold: f32,
}
impl Default for ScopeSnapshot {
    fn default() -> Self {
        Self {
            x: [[ScopePoint::default(); 256]; 16],
            y: [[ScopePoint::default(); 256]; 16],
            x_channels: 0,
            y_channels: 0,
            trigger_input_channels: 0,
            buffer_index: 0,
            samples_per_bin: 1,
            sample_index: 0,
            waiting: false,
            xy: false,
            x_scale: 0,
            y_scale: 0,
            x_offset: 0.0,
            y_offset: 0.0,
            time: 0.5,
            trigger_enabled: false,
            threshold: 0.0,
        }
    }
}
pub(crate) const INPUTS: [&str; 3] = ["x", "y", "trigger"];
pub(crate) const OUTPUTS: [&str; 2] = ["x", "y"];
pub(crate) fn catalog() -> Value {
    json!({"kind":"scope","title":"SCOPE","model":"functional","inputs":INPUTS,"params":{
        "auto_range":{"default":1,"min":0,"max":1,"step":1,"values":[0,1],"labels":["Manual","Auto"],"display_only":true,"description":"Native display-only automatic voltage range; does not change acquisition, raw measurements or passthrough. Manual range/offset gestures turn it off."},
        "x_scale":{"default":0,"min":0,"max":8,"step":1,"unit":"octaves","description":"20 V/screen divided by 2^scale"},
        "y_scale":{"default":0,"min":0,"max":8,"step":1,"unit":"octaves","description":"20 V/screen divided by 2^scale"},
        "x_offset":{"default":0,"min":-10,"max":10,"unit":"V"},"y_offset":{"default":0,"min":-10,"max":10,"unit":"V"},
        "time":{"default":0.5,"min":0.005,"max":50,"unit":"s","description":"Screen duration; each of 256 bins spans ceil(time*sample_rate/256) samples"},
        "xy":{"default":0,"min":0,"max":1,"step":1,"states":["time","XY"]},"trigger_enabled":{"default":0,"min":0,"max":1,"step":1,"states":["free","triggered"]},"threshold":{"default":0,"min":-10,"max":10,"unit":"V"}},
        "description":"Independent raw 0–16 channel X/Y passthrough and 256-bin minimum/maximum scope. Trigger waits after a complete capture; external trigger normals to X when zero-channel; XY/free-running restart immediately."})
}
pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"]["version_2"]=json!("independent raw X and Y 0–16 channel passthrough; per-lane acquisition, trigger normals to X only when actual trigger channels are zero");
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["auxiliary_inputs"] =
        json!("no mono broadcasting in acquisition; absent lanes contribute no samples");
    module["acquisition_policy"] = json!({"bins":256,"channels_per_axis":16,"trigger":"per-lane Schmitt, low <= threshold, high >= threshold + 0.001 V; first high only arms; any lane rising starts capture including that sample; detectors reset after capture","buffer":"in-place progressive update, buffer_index identifies next bin to complete; partial accumulation is private, previous complete bins remain until overwritten","state_adoption":"same identity/kind/rate and acquisition parameters and incoming routes preserve state without allocation; display scale/offset edits preserve capture; time, trigger, threshold, XY, route edits reset; initialization clears acquisition","bypass":"raw independent passthrough including zero-channel frames; acquisition/display freezes","time":"literal seconds differs from Rack exponential TIME parameter; ceil(time * rate / 256) samples per bin"});
    module["functional_reference"]=json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Scope.cpp");
    module["display_policy"] = json!({"auto_range":"native display-only extension, enabled by default; finite min/max from every active voice and zero determine independent X/Y 1/2/5 voltage spans with headroom and display hysteresis","calibration":"AUTO or MANUAL span and offset are shown in V, mV or µV; measurements and outputs always remain raw volts","manual":"turning auto_range off restores saved x/y_scale and offset values; a manual range/offset gesture atomically turns automatic ranging off","state":"display edits preserve capture and detector state; automatic fit is presentation state, not saved samples or audio gain"});
}
#[derive(Debug)]
pub(crate) struct Scope {
    pub snapshot: Box<ScopeSnapshot>,
    detector: [u8; 16],
    current: [[ScopePoint; 16]; 2],
}
impl Scope {
    pub fn build(node: &Node, rate: u32) -> Result<Self, String> {
        for name in node.params.keys() {
            if ![
                "auto_range",
                "x_scale",
                "y_scale",
                "x_offset",
                "y_offset",
                "time",
                "xy",
                "trigger_enabled",
                "threshold",
            ]
            .contains(&name.as_str())
            {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let integer = |name, default, min, max| -> Result<f32, String> {
            let v = parameter(node, name, default, min, max)?;
            if v.fract() != 0.0 {
                return Err(format!(
                    "node '{}': parameter '{name}' must be an integer",
                    node.id
                ));
            }
            Ok(v)
        };
        let time = parameter(node, "time", 0.5, 0.005, 50.0)?;
        integer("auto_range", 1.0, 0.0, 1.0)?;
        Ok(Self {
            snapshot: Box::new(ScopeSnapshot {
                x: [[ScopePoint::default(); 256]; 16],
                y: [[ScopePoint::default(); 256]; 16],
                x_channels: 0,
                y_channels: 0,
                trigger_input_channels: 0,
                buffer_index: 0,
                samples_per_bin: (time as f64 * rate as f64 / 256.0).ceil().max(1.0) as u32,
                sample_index: 0,
                waiting: false,
                xy: integer("xy", 0.0, 0.0, 1.0)? != 0.0,
                x_scale: integer("x_scale", 0.0, 0.0, 8.0)? as u8,
                y_scale: integer("y_scale", 0.0, 0.0, 8.0)? as u8,
                x_offset: parameter(node, "x_offset", 0.0, -10.0, 10.0)?,
                y_offset: parameter(node, "y_offset", 0.0, -10.0, 10.0)?,
                time,
                trigger_enabled: integer("trigger_enabled", 0.0, 0.0, 1.0)? != 0.0,
                threshold: parameter(node, "threshold", 0.0, -10.0, 10.0)?,
            }),
            detector: [0; 16],
            current: [[ScopePoint::default(); 16]; 2],
        })
    }
    pub fn reset(&mut self) {
        self.snapshot.x.fill([ScopePoint::default(); 256]);
        self.snapshot.y.fill([ScopePoint::default(); 256]);
        self.snapshot.x_channels = 0;
        self.snapshot.y_channels = 0;
        self.snapshot.trigger_input_channels = 0;
        self.snapshot.buffer_index = 0;
        self.snapshot.sample_index = 0;
        self.snapshot.waiting = false;
        self.detector.fill(0);
        self.current.fill([ScopePoint::default(); 16]);
    }
    pub fn adopt(&mut self, old: &Self, routes_unchanged: bool) {
        let s = &mut self.snapshot;
        let o = &old.snapshot;
        if !routes_unchanged
            || s.time != o.time
            || s.xy != o.xy
            || s.trigger_enabled != o.trigger_enabled
            || s.threshold != o.threshold
        {
            return;
        }
        s.x = o.x;
        s.y = o.y;
        s.x_channels = o.x_channels;
        s.y_channels = o.y_channels;
        s.trigger_input_channels = o.trigger_input_channels;
        s.buffer_index = o.buffer_index;
        s.sample_index = o.sample_index;
        s.waiting = o.waiting;
        self.detector = old.detector;
        self.current = old.current;
    }
    pub fn process(&mut self, inputs: &[PortFrame; 3], outputs: &mut [PortFrame]) {
        outputs[0] = inputs[0];
        outputs[1] = inputs[1];
        let s = &mut self.snapshot;
        s.x_channels = inputs[0].channels;
        s.y_channels = inputs[1].channels;
        s.trigger_input_channels = inputs[2].channels;
        if s.buffer_index == 256 {
            s.waiting = s.trigger_enabled && !s.xy;
            if s.waiting {
                let trigger = if inputs[2].channels > 0 {
                    &inputs[2]
                } else {
                    &inputs[0]
                };
                let mut rising = false;
                for lane in 0..usize::from(trigger.channels).min(16) {
                    let v = trigger.values[lane];
                    if v >= s.threshold + 0.001 {
                        rising |= self.detector[lane] == 1;
                        self.detector[lane] = 2;
                    } else if v <= s.threshold {
                        self.detector[lane] = 1;
                    }
                }
                if !rising {
                    return;
                }
            }
            s.buffer_index = 0;
            s.sample_index = 0;
            s.waiting = false;
        }
        for (axis, input) in self.current.iter_mut().zip(&inputs[..2]) {
            for (lane, point) in axis
                .iter_mut()
                .enumerate()
                .take(usize::from(input.channels).min(16))
            {
                let value = input.values[lane];
                point.min = point.min.min(value);
                point.max = point.max.max(value);
            }
        }
        s.sample_index += 1;
        if s.sample_index >= s.samples_per_bin {
            for lane in 0..16 {
                s.x[lane][s.buffer_index] = self.current[0][lane];
                s.y[lane][s.buffer_index] = self.current[1][lane];
            }
            self.current.fill([ScopePoint::default(); 16]);
            s.sample_index = 0;
            s.buffer_index += 1;
            if s.buffer_index == 256 {
                self.detector.fill(0);
                s.waiting = s.trigger_enabled && !s.xy;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    fn scope(params: &[(&str, f32)], rate: u32) -> Scope {
        Scope::build(
            &Node {
                id: "s".into(),
                kind: "scope".into(),
                params: params
                    .iter()
                    .map(|(k, v)| (k.to_string(), *v))
                    .collect::<BTreeMap<_, _>>(),
                bypassed: false,
                content: None,
                score: None,
            },
            rate,
        )
        .unwrap()
    }
    fn tick(s: &mut Scope, x: PortFrame, y: PortFrame, t: PortFrame) -> [PortFrame; 2] {
        let mut o = [PortFrame::default(); 2];
        s.process(&[x, y, t], &mut o);
        o
    }
    #[test]
    fn partial_bins_preserve_history_and_inactive_lanes_add_no_zero() {
        let mut s = scope(&[("time", 0.005)], 102400);
        tick(
            &mut s,
            PortFrame::mono(4.0),
            PortFrame::default(),
            PortFrame::default(),
        );
        assert_eq!(s.snapshot.x[0][0], ScopePoint::default());
        tick(
            &mut s,
            PortFrame::mono(8.0),
            PortFrame::default(),
            PortFrame::default(),
        );
        assert_eq!(s.snapshot.x[0][0], ScopePoint { min: 4.0, max: 8.0 });
        for _ in 1..256 {
            tick(
                &mut s,
                PortFrame::mono(4.0),
                PortFrame::default(),
                PortFrame::default(),
            );
            tick(
                &mut s,
                PortFrame::mono(8.0),
                PortFrame::default(),
                PortFrame::default(),
            );
        }
        tick(
            &mut s,
            PortFrame::default(),
            PortFrame::default(),
            PortFrame::default(),
        );
        assert_eq!(s.snapshot.x[0][0], ScopePoint { min: 4.0, max: 8.0 });
        tick(
            &mut s,
            PortFrame::mono(7.0),
            PortFrame::default(),
            PortFrame::default(),
        );
        assert_eq!(s.snapshot.x[0][0], ScopePoint { min: 7.0, max: 7.0 });
        assert_eq!(s.snapshot.x[1][0], ScopePoint::default());
    }
    #[test]
    fn retiring_lane_and_midbin_adoption_preserve_only_real_samples() {
        let mut old = scope(&[("time", 0.005)], 102400);
        let mut x = PortFrame {
            channels: 16,
            ..Default::default()
        };
        x.values[0] = 3.0;
        x.values[15] = 7.0;
        tick(&mut old, x, PortFrame::default(), PortFrame::default());
        let mut new = scope(
            &[("time", 0.005), ("x_scale", 4.0), ("x_offset", 2.0)],
            102400,
        );
        new.adopt(&old, true);
        let o = tick(
            &mut new,
            PortFrame::mono(-5.0),
            PortFrame::default(),
            PortFrame::default(),
        );
        assert_eq!(o[0], PortFrame::mono(-5.0));
        assert_eq!(
            new.snapshot.x[0][0],
            ScopePoint {
                min: -5.0,
                max: 3.0
            }
        );
        assert_eq!(new.snapshot.x[15][0], ScopePoint { min: 7.0, max: 7.0 });
        for _ in 0..2 {
            tick(
                &mut new,
                PortFrame::default(),
                PortFrame::default(),
                PortFrame::default(),
            );
        }
        assert_eq!(new.snapshot.x[15][1], ScopePoint::default());
    }
    #[test]
    fn external_trigger_overrides_x_and_uses_calibrated_threshold() {
        let mut s = scope(
            &[
                ("time", 0.005),
                ("trigger_enabled", 1.0),
                ("threshold", 2.0),
            ],
            51200,
        );
        for _ in 0..256 {
            tick(
                &mut s,
                PortFrame::mono(0.0),
                PortFrame::default(),
                PortFrame::mono(3.0),
            );
        }
        for x in [0.0, 3.0, 0.0, 3.0] {
            tick(
                &mut s,
                PortFrame::mono(x),
                PortFrame::default(),
                PortFrame::mono(3.0),
            );
            assert!(s.snapshot.waiting);
        }
        tick(
            &mut s,
            PortFrame::mono(0.0),
            PortFrame::default(),
            PortFrame::mono(2.0),
        );
        tick(
            &mut s,
            PortFrame::mono(0.0),
            PortFrame::default(),
            PortFrame::mono(2.0005),
        );
        assert!(s.snapshot.waiting);
        tick(
            &mut s,
            PortFrame::mono(6.0),
            PortFrame::default(),
            PortFrame::mono(2.001),
        );
        assert!(!s.snapshot.waiting);
        assert_eq!(s.snapshot.x[0][0].max, 6.0);
    }
    #[test]
    fn trigger_width_metadata_is_actual_and_adopted_or_reset() {
        let mut s = scope(&[], 48000);
        for channels in [0, 1, 16] {
            tick(
                &mut s,
                PortFrame::mono(1.0),
                PortFrame::default(),
                PortFrame {
                    channels,
                    ..Default::default()
                },
            );
            assert_eq!(s.snapshot.trigger_input_channels, channels);
        }
        let mut next = scope(&[("x_scale", 1.0)], 48000);
        next.adopt(&s, true);
        assert_eq!(next.snapshot.trigger_input_channels, 16);
        next.reset();
        assert_eq!(next.snapshot.trigger_input_channels, 0);
        let mut next = scope(&[("time", 1.0)], 48000);
        next.adopt(&s, true);
        assert_eq!(next.snapshot.trigger_input_channels, 0);
    }
    #[test]
    fn trigger_metadata_freezes_through_bypass_and_clears_on_initialize() {
        let patch:crate::Patch=serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":[{"id":"t","kind":"poly_constant","params":{"channels":16}},{"id":"s","kind":"scope"}],"cables":[{"from":"t","to":"s","input":"trigger"}]})).unwrap();
        let mut old = patch.compile().unwrap();
        let mut frames = [PortFrame::default(); 2];
        old.process_poly_block(&mut frames);
        let h = old.resolve_module_scope("s").unwrap();
        assert_eq!(old.module_scope(h).unwrap().trigger_input_channels, 16);
        let mut patch = patch;
        patch.nodes[1].bypassed = true;
        let mut next = patch.compile().unwrap();
        next.adopt_state_from(&old);
        let h = next.resolve_module_scope("s").unwrap();
        next.process_poly_block(&mut frames);
        assert_eq!(next.module_scope(h).unwrap().trigger_input_channels, 16);
        assert!(next.initialize_module_state(h));
        assert_eq!(next.module_scope(h).unwrap().trigger_input_channels, 0);
        next.process_poly_block(&mut frames);
        assert_eq!(next.module_scope(h).unwrap().trigger_input_channels, 0);
    }
    #[test]
    fn raw_independent_widths_and_minmax() {
        let mut s = scope(&[("time", 0.005)], 102400);
        for n in [0, 1, 16] {
            let mut x = PortFrame {
                channels: n,
                ..Default::default()
            };
            x.values.fill(123.0);
            let y = PortFrame::mono(-14.0);
            let o = tick(&mut s, x, y, PortFrame::default());
            assert_eq!(o, [x, y]);
        }
        s.reset();
        for bin in 0..256 {
            tick(
                &mut s,
                PortFrame::mono(bin as f32),
                PortFrame::mono(-2.0),
                PortFrame::default(),
            );
            tick(
                &mut s,
                PortFrame::mono(-(bin as f32)),
                PortFrame::mono(7.0),
                PortFrame::default(),
            );
        }
        assert_eq!(s.snapshot.buffer_index, 256);
        for bin in 0..256 {
            assert_eq!(
                s.snapshot.x[0][bin],
                ScopePoint {
                    min: -(bin as f32),
                    max: bin as f32
                }
            );
            assert_eq!(
                s.snapshot.y[0][bin],
                ScopePoint {
                    min: -2.0,
                    max: 7.0
                }
            );
        }
    }
    #[test]
    fn trigger_waits_initial_high_then_any_lane_edge_and_includes_sample() {
        let mut s = scope(&[("time", 0.005), ("trigger_enabled", 1.0)], 51200);
        for _ in 0..256 {
            tick(
                &mut s,
                PortFrame::mono(3.0),
                PortFrame::default(),
                PortFrame::mono(10.0),
            );
        }
        tick(
            &mut s,
            PortFrame::mono(4.0),
            PortFrame::default(),
            PortFrame::mono(10.0),
        );
        assert!(s.snapshot.waiting);
        tick(
            &mut s,
            PortFrame::mono(4.0),
            PortFrame::default(),
            PortFrame::mono(0.0),
        );
        assert!(s.snapshot.waiting);
        let mut t = PortFrame {
            channels: 16,
            ..Default::default()
        };
        tick(&mut s, PortFrame::mono(4.0), PortFrame::default(), t);
        t.values[15] = 0.001;
        tick(&mut s, PortFrame::mono(8.0), PortFrame::default(), t);
        assert!(!s.snapshot.waiting);
        assert_eq!(s.snapshot.buffer_index, 1);
        assert_eq!(s.snapshot.x[0][0].max, 8.0);
    }
    #[test]
    fn trigger_normal_and_xy_free_run() {
        for xy in [false, true] {
            let mut s = scope(
                &[
                    ("time", 0.005),
                    ("trigger_enabled", 1.0),
                    ("xy", f32::from(xy)),
                ],
                51200,
            );
            for _ in 0..256 {
                tick(
                    &mut s,
                    PortFrame::mono(0.0),
                    PortFrame::default(),
                    PortFrame::default(),
                );
            }
            tick(
                &mut s,
                PortFrame::mono(0.0),
                PortFrame::default(),
                PortFrame::default(),
            );
            tick(
                &mut s,
                PortFrame::mono(1.0),
                PortFrame::default(),
                PortFrame::default(),
            );
            assert_eq!(s.snapshot.buffer_index, if xy { 2 } else { 1 });
        }
    }
    #[test]
    fn adoption_copies_state_but_preserves_display_configuration_and_reset_changes() {
        let mut old = scope(&[], 48000);
        tick(
            &mut old,
            PortFrame::mono(3.0),
            PortFrame::default(),
            PortFrame::default(),
        );
        let mut new = scope(&[("x_scale", 4.0), ("x_offset", 2.0)], 48000);
        new.adopt(&old, true);
        assert_eq!(new.snapshot.sample_index, 1);
        assert_eq!(new.snapshot.x_scale, 4);
        assert_eq!(new.snapshot.x_offset, 2.0);
        for params in [
            vec![("time", 1.0)],
            vec![("xy", 1.0)],
            vec![("threshold", 1.0)],
            vec![("trigger_enabled", 1.0)],
        ] {
            let mut new = scope(&params, 48000);
            new.adopt(&old, true);
            assert_eq!(new.snapshot.sample_index, 0);
        }
        new.reset();
        assert_eq!(new.snapshot.sample_index, 0);
        assert_eq!(new.snapshot.x[0][0], ScopePoint::default());
    }
}
