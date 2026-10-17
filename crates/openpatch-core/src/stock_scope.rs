//! Original Rust realization of the pinned Fundamental Scope acquisition.
//! Behavioral reference: GPL-3.0 Fundamental `src/Scope.cpp` at
//! 10dd0160c664770910e5584b7b00498cc48d9ddd. No upstream code or art is
//! embedded. The ordinary OpenPatch `scope` retains its separate behavior.
use crate::host_ports::HostPortSnapshot;
use crate::native_float::{flush_tiny, rack_add, rack_div, rack_mul, rack_sub};
use crate::scope::{ScopePoint, ScopeSnapshot, StockScopeMetadata, SCOPE_BINS};
use crate::{ModuleLightState, Node};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 3] = ["x", "y", "trigger"];
pub(crate) const OUTPUTS: [&str; 2] = ["x", "y"];
pub(crate) const PARAMS: [&str; 8] = [
    "x_scale",
    "x_offset",
    "y_scale",
    "y_offset",
    "time",
    "xy",
    "threshold",
    "trigger_disabled",
];
const DEFAULT_PARAMS: [f32; 8] = [0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0];
const LOW: u8 = 0;
const HIGH: u8 = 1;
const UNINITIALIZED: u8 = 2;

fn configured_time_min() -> f32 {
    -50.0_f32.log2()
}

fn configured_time_max() -> f32 {
    -0.005_f32.log2()
}

pub(crate) fn catalog() -> Value {
    json!({
        "kind":"stock_scope", "title":"STOCK SCOPE", "model":"functional",
        "description":"Pinned Fundamental digital oscilloscope acquisition with independent X/Y polyphony and raw passthrough. The existing Scope remains independent.",
        "inputs":INPUTS,
        "params":{
            "x_scale":{"default":0,"min":0,"max":8,"step":1,"unit":"octaves"},
            "x_offset":{"default":0,"min":-10,"max":10,"unit":"V"},
            "y_scale":{"default":0,"min":0,"max":8,"step":1,"unit":"octaves"},
            "y_offset":{"default":0,"min":-10,"max":10,"unit":"V"},
            "time":{"default":1,"min":configured_time_min(),"max":configured_time_max(),
                "raw_min":-6,"raw_max":8,"unit":"log2(1/s)",
                "description":"Raw source exponent; nominal screen duration is 2^(-time) seconds, while the actual next bin width uses the source polynomial and ceil."},
            "xy":{"default":0,"min":0,"max":1,"step":1,"values":[0,1],"labels":["Time","XY"]},
            "threshold":{"default":0,"min":-10,"max":10,"unit":"V"},
            "trigger_disabled":{"default":1,"min":0,"max":1,"step":1,"values":[0,1],"labels":["Enabled","Disabled"]}
        }
    })
}

pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"] = json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Scope.cpp");
    module["min_patch_version"] = json!(2);
    module["polyphony"]["primary_input"] = Value::Null;
    module["polyphony"]["version_2"] = json!("X and Y have independent raw 0–16 input widths and exact passthrough outputs; trigger scans its own raw lanes when its channel count is positive, otherwise X; no mono broadcast in acquisition");
    module["acquisition_policy"] = json!({
        "bins":256,"lanes_per_axis":16,
        "completed":"one authoritative in-place 256×2×16 min/max store; current partial points are private",
        "trigger":"16 LOW/HIGH/UNINITIALIZED Schmitt states, first high only arms; detector scan occurs only after the completed sweep",
        "initialization":"Engine reset restores eight Params and source onReset clears only completed points; partial points, indices, channel counts, detectors and lights persist",
        "adoption":"same incarnation preserves source acquisition state and held lights even across route edits; receiving raw Params win",
        "bypass":"source process and lamps freeze; no source-defined bypass routes",
        "time":"raw log2 reciprocal-seconds Param; next bin has ceil(exp2_taylor5(-TIME)/256 × sample rate) samples"
    });
    module["light_names"] = json!(["xy", "trigger"]);
    module["light_policy"] = json!("actual source immediate XY/non-disabled-trigger lights are updated on process only; bypass holds their previous values");
    module["legacy_data"] = json!("custom save is null; nonzero integer lissajous/external values set XY/trigger-disabled Params to 1 without clearing them for zero or non-integers");
}

pub(crate) fn annotate_bypass(module: &mut Value) {
    module["bypass"]["output_policy"] = json!("No source configBypass routes or Scope output writes; connected outputs follow canonical host bypass-transition lifecycle and disconnected raw storage remains");
    module["bypass"]["state_policy"] = json!("completed and partial Points, counters, retained widths, all sixteen tri-state Schmitt detectors and both source lights freeze while bypassed");
    module["bypass"]["compatibility"] = json!("Fundamental Scope has no configBypass route; common native host lifecycle handles physical output transitions");
}

/// Lossless source Point at an explicit diagnostic read boundary.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct StockScopePointBits {
    pub min_bits: u32,
    pub max_bits: u32,
}

impl From<ScopePoint> for StockScopePointBits {
    fn from(point: ScopePoint) -> Self {
        Self {
            min_bits: point.min.to_bits(),
            max_bits: point.max.to_bits(),
        }
    }
}

/// A complete passive diagnostic copy of the actual model's source state.
/// Creating this explicitly may allocate; audio processing and scope
/// publication use the authoritative boxed `ScopeSnapshot` instead.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct StockScopeDiagnosticBits {
    pub parameter_bits: [u32; 8],
    pub light_bits: [u32; 2],
    pub point_buffer: Vec<[[StockScopePointBits; 16]; 2]>,
    pub current_points: [[StockScopePointBits; 16]; 2],
    pub channels_x: u8,
    pub channels_y: u8,
    pub buffer_index: u16,
    pub frame_index: i32,
    pub triggers: [u8; 16],
    pub sample_rate: u32,
    pub sample_time_bits: u32,
}

#[derive(Debug)]
pub(crate) struct StockScope {
    /// The sole persistent completed 256 × 2 × 16 source Point store.
    pub(crate) snapshot: Box<ScopeSnapshot>,
    parameters: [f32; 8],
    current_points: [[ScopePoint; 16]; 2],
    triggers: [u8; 16],
    frame_index: i32,
    lights: [f32; 2],
    sample_rate: u32,
    sample_time: f32,
}

/// Source-ordered fifth-order exponential with Rack's exponent-bit floor.
/// The normal parity matrix bounds TIME to [-6, 8]. Rust's saturating float
/// cast makes excluded extreme arguments deterministic, without claiming C++
/// source equivalence there.
#[allow(clippy::excessive_precision)]
fn exp2_taylor5(x: f32) -> f32 {
    let shifted = rack_add(x, 127.0);
    let integer = shifted as i32;
    let fraction = rack_sub(shifted, integer as f32);
    let scale = f32::from_bits((integer as u32).wrapping_shl(23));
    let coefficients = [
        1.0_f32,
        0.69315169353961_f32,
        0.2401595990753_f32,
        0.055817908652_f32,
        0.008991698010_f32,
        0.001879100722_f32,
    ];
    let mut polynomial = coefficients[5];
    for coefficient in coefficients[..5].iter().rev() {
        polynomial = rack_add(*coefficient, rack_mul(polynomial, fraction));
    }
    rack_mul(scale, polynomial)
}

fn enabled(value: f32) -> bool {
    flush_tiny(value) != 0.0
}

/// C++ `std::min`/`std::max` retain their first operand on an FZ-equal tie.
/// Keep the selected original raw bits, including signed subnormals and zero.
fn accumulate(point: &mut ScopePoint, value: f32) {
    if flush_tiny(value) < flush_tiny(point.min) {
        point.min = value;
    }
    if flush_tiny(point.max) < flush_tiny(value) {
        point.max = value;
    }
}

fn trigger_step(state: &mut u8, input: f32) -> bool {
    let value = flush_tiny(input);
    if *state == LOW && value >= 1.0 {
        *state = HIGH;
        true
    } else {
        if *state == HIGH && value <= 0.0 {
            *state = LOW;
        } else if *state == UNINITIALIZED && value >= 1.0 {
            *state = HIGH;
        } else if *state == UNINITIALIZED && value <= 0.0 {
            *state = LOW;
        }
        false
    }
}

impl StockScope {
    pub(crate) fn build(node: &Node, sample_rate: u32) -> Result<Self, String> {
        if sample_rate == 0 {
            return Err("stock_scope sample rate must be positive".into());
        }
        for key in node.params.keys() {
            if !PARAMS.contains(&key.as_str()) {
                return Err(format!("node '{}': unknown parameter '{key}'", node.id));
            }
        }
        let mut parameters = DEFAULT_PARAMS;
        for (index, name) in PARAMS.iter().enumerate() {
            if let Some(&value) = node.params.get(*name) {
                if !Self::configured_parameter(index, value) {
                    return Err(format!(
                        "node '{}': parameter '{name}' is outside its configured range or step",
                        node.id
                    ));
                }
                parameters[index] = value;
            }
        }
        let sample_time = rack_div(1.0, sample_rate as f32);
        let mut result = Self {
            snapshot: Box::new(ScopeSnapshot::default()),
            parameters,
            current_points: [[ScopePoint::default(); 16]; 2],
            triggers: [UNINITIALIZED; 16],
            frame_index: 0,
            lights: [0.0; 2],
            sample_rate,
            sample_time,
        };
        result.refresh_projection();
        Ok(result)
    }

    fn configured_parameter(index: usize, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        let (minimum, maximum, snapped) = match index {
            0 | 2 => (0.0, 8.0, true),
            1 | 3 | 6 => (-10.0, 10.0, false),
            4 => (configured_time_min(), configured_time_max(), false),
            5 | 7 => (0.0, 1.0, true),
            _ => return false,
        };
        (minimum..=maximum).contains(&value) && (!snapped || value.fract() == 0.0)
    }

    fn safe_raw_parameter(index: usize, value: f32) -> bool {
        value.is_finite() && (index != 4 || (-6.0..=8.0).contains(&value))
    }

    /// Diagnostic immediate raw Param write; normal authored edits are bounded
    /// by catalog quantities in the host Session.
    #[doc(hidden)]
    pub(crate) fn set_parameter_raw(&mut self, index: usize, value: f32) -> bool {
        if index >= PARAMS.len() || !Self::safe_raw_parameter(index, value) {
            return false;
        }
        self.parameters[index] = value;
        self.refresh_projection();
        true
    }

    pub(crate) fn scope_snapshot(&self) -> &ScopeSnapshot {
        &self.snapshot
    }

    pub(crate) fn diagnostic_bits(&self) -> StockScopeDiagnosticBits {
        let mut completed = Vec::with_capacity(SCOPE_BINS);
        for bin in 0..SCOPE_BINS {
            let mut row = [[StockScopePointBits::default(); 16]; 2];
            for lane in 0..16 {
                row[0][lane] = self.snapshot.x[lane][bin].into();
                row[1][lane] = self.snapshot.y[lane][bin].into();
            }
            completed.push(row);
        }
        StockScopeDiagnosticBits {
            parameter_bits: self.parameters.map(f32::to_bits),
            light_bits: self.lights.map(f32::to_bits),
            point_buffer: completed,
            current_points: self.current_points.map(|axis| axis.map(Into::into)),
            channels_x: self.snapshot.x_channels,
            channels_y: self.snapshot.y_channels,
            buffer_index: self.snapshot.buffer_index as u16,
            frame_index: self.frame_index,
            triggers: self.triggers,
            sample_rate: self.sample_rate,
            sample_time_bits: self.sample_time.to_bits(),
        }
    }

    pub(crate) fn legacy_data(&self) -> Value {
        Value::Null
    }

    /// Rack's custom legacy loader uses `json_integer_value`, which returns
    /// zero for missing, boolean, float, string and null values.
    pub(crate) fn load_legacy_data(&mut self, data: &Value) {
        if data.get("lissajous").and_then(Value::as_i64).unwrap_or(0) != 0 {
            self.parameters[5] = 1.0;
        }
        if data.get("external").and_then(Value::as_i64).unwrap_or(0) != 0 {
            self.parameters[7] = 1.0;
        }
        self.refresh_projection();
    }

    /// `Scope::onReset`: clear only completed Points. All partial acquisition,
    /// counters, widths, detector states and lamp values deliberately survive.
    pub(crate) fn source_reset(&mut self) {
        self.snapshot.x.fill([ScopePoint::default(); SCOPE_BINS]);
        self.snapshot.y.fill([ScopePoint::default(); SCOPE_BINS]);
    }

    /// Remove/recreate of an actual module makes acquisition cold while the
    /// receiving patch's raw parameter values remain its controls.
    pub(crate) fn recreate(&mut self) {
        self.source_reset();
        self.current_points = [[ScopePoint::default(); 16]; 2];
        self.triggers = [UNINITIALIZED; 16];
        self.frame_index = 0;
        self.snapshot.buffer_index = 0;
        self.snapshot.sample_index = 0;
        self.snapshot.x_channels = 0;
        self.snapshot.y_channels = 0;
        self.snapshot.trigger_input_channels = 0;
        self.lights = [0.0; 2];
        if let Some(metadata) = &mut self.snapshot.stock {
            metadata.trigger_input_connected = false;
        }
        self.refresh_projection();
    }

    /// Same-incarnation graph adoption preserves the actual source histories
    /// even when routes change. The newly built model's raw Params win.
    pub(crate) fn adopt(&mut self, old: &Self) {
        self.snapshot.x = old.snapshot.x;
        self.snapshot.y = old.snapshot.y;
        self.snapshot.x_channels = old.snapshot.x_channels;
        self.snapshot.y_channels = old.snapshot.y_channels;
        self.snapshot.trigger_input_channels = old.snapshot.trigger_input_channels;
        self.snapshot.buffer_index = old.snapshot.buffer_index;
        self.snapshot.sample_index = old.snapshot.sample_index;
        self.current_points = old.current_points;
        self.triggers = old.triggers;
        self.frame_index = old.frame_index;
        self.lights = old.lights;
        let trigger_input_connected = old
            .snapshot
            .stock
            .as_ref()
            .is_some_and(|metadata| metadata.trigger_input_connected);
        self.refresh_projection();
        if let Some(metadata) = &mut self.snapshot.stock {
            metadata.trigger_input_connected = trigger_input_connected;
        }
    }

    /// Actual Engine rate event updates timing in place without clearing data.
    pub(crate) fn set_sample_rate(&mut self, sample_rate: u32) -> Result<(), String> {
        if sample_rate == 0 {
            return Err("stock_scope sample rate must be positive".into());
        }
        self.sample_rate = sample_rate;
        self.sample_time = rack_div(1.0, sample_rate as f32);
        self.refresh_projection();
        Ok(())
    }

    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut result = ModuleLightState {
            count: 2,
            ..ModuleLightState::default()
        };
        result.brightness[0][0] = self.lights[0];
        result.brightness[1][0] = self.lights[1];
        result
    }

    /// The common host owns cable transfer, raw port initialization and
    /// bypass. It calls this only for a source-executed (non-bypassed) frame.
    pub(crate) fn process(
        &mut self,
        inputs: &[HostPortSnapshot; 16],
        input_presence: u16,
        outputs: &mut [HostPortSnapshot],
    ) {
        let xy = enabled(self.parameters[5]);
        let trigger_enabled = !enabled(self.parameters[7]);
        self.lights = [
            f32::from(u8::from(xy)),
            f32::from(u8::from(trigger_enabled)),
        ];

        if self.snapshot.buffer_index >= SCOPE_BINS {
            let mut triggered = xy || !trigger_enabled;
            if !triggered {
                // SDK Input::isConnected() tests channels > 0. A separate
                // physical-presence bit must not select a zero-channel input.
                let trigger_input = if inputs[2].channels > 0 {
                    &inputs[2]
                } else {
                    &inputs[0]
                };
                let threshold = self.parameters[6];
                let threshold_high = rack_add(threshold, 0.001);
                for lane in 0..usize::from(trigger_input.channels) {
                    let voltage = trigger_input.values[lane];
                    let scaled = rack_add(
                        0.0,
                        rack_mul(
                            rack_div(
                                rack_sub(voltage, threshold),
                                rack_sub(threshold_high, threshold),
                            ),
                            rack_sub(1.0, 0.0),
                        ),
                    );
                    if trigger_step(&mut self.triggers[lane], scaled) {
                        triggered = true;
                    }
                }
            }
            if triggered {
                self.triggers.fill(UNINITIALIZED);
                self.snapshot.buffer_index = 0;
                self.frame_index = 0;
            }
        }

        self.snapshot.x_channels = inputs[0].channels;
        self.snapshot.y_channels = inputs[1].channels;
        self.snapshot.trigger_input_channels = inputs[2].channels;
        for axis in 0..2 {
            outputs[axis].set_channels(inputs[axis].channels);
            for lane in 0..usize::from(outputs[axis].channels) {
                outputs[axis].values[lane] = inputs[axis].values[lane];
            }
        }

        if self.snapshot.buffer_index < SCOPE_BINS {
            let negative_time = f32::from_bits(self.parameters[4].to_bits() ^ 0x8000_0000);
            let delta_time = rack_div(exp2_taylor5(negative_time), SCOPE_BINS as f32);
            let frame_count = rack_mul(delta_time, self.sample_rate as f32).ceil() as i32;
            for lane in 0..usize::from(inputs[0].channels) {
                accumulate(&mut self.current_points[0][lane], inputs[0].values[lane]);
            }
            for lane in 0..usize::from(inputs[1].channels) {
                accumulate(&mut self.current_points[1][lane], inputs[1].values[lane]);
            }
            self.frame_index += 1;
            if self.frame_index >= frame_count {
                self.frame_index = 0;
                let bin = self.snapshot.buffer_index;
                for lane in 0..16 {
                    self.snapshot.x[lane][bin] = self.current_points[0][lane];
                    self.snapshot.y[lane][bin] = self.current_points[1][lane];
                }
                self.current_points = [[ScopePoint::default(); 16]; 2];
                self.snapshot.buffer_index += 1;
            }
        }
        self.snapshot.sample_index = self.frame_index as u32;
        self.refresh_projection();
        if let Some(metadata) = &mut self.snapshot.stock {
            metadata.trigger_input_connected = input_presence & (1 << 2) != 0;
        }
    }

    fn next_bin_samples(&self) -> u32 {
        let negative_time = f32::from_bits(self.parameters[4].to_bits() ^ 0x8000_0000);
        let seconds_per_bin = rack_div(exp2_taylor5(negative_time), SCOPE_BINS as f32);
        rack_mul(seconds_per_bin, self.sample_rate as f32).ceil() as u32
    }

    fn refresh_projection(&mut self) {
        let nominal = (-self.parameters[4]).exp2();
        let next_bin_samples = self.next_bin_samples();
        let trigger_input_connected = self
            .snapshot
            .stock
            .as_ref()
            .is_some_and(|metadata| metadata.trigger_input_connected);
        self.snapshot.samples_per_bin = next_bin_samples;
        self.snapshot.sample_index = self.frame_index as u32;
        self.snapshot.waiting = self.snapshot.buffer_index >= SCOPE_BINS
            && !enabled(self.parameters[5])
            && !enabled(self.parameters[7]);
        // ScopeDisplay's `isLissajous()` uses `> 0`, whereas process() uses
        // float-to-bool. A diagnostic negative raw mode still drives source
        // acquisition, but the source display stays in its time layout.
        self.snapshot.xy = flush_tiny(self.parameters[5]) > 0.0;
        self.snapshot.x_scale = self.parameters[0].round().clamp(0.0, 8.0) as u8;
        self.snapshot.y_scale = self.parameters[2].round().clamp(0.0, 8.0) as u8;
        self.snapshot.x_offset = self.parameters[1];
        self.snapshot.y_offset = self.parameters[3];
        self.snapshot.time = nominal;
        self.snapshot.trigger_enabled = !enabled(self.parameters[7]);
        self.snapshot.threshold = self.parameters[6];
        self.snapshot.stock = Some(StockScopeMetadata {
            parameter_bits: self.parameters.map(f32::to_bits),
            light_bits: self.lights.map(f32::to_bits),
            sample_rate: self.sample_rate,
            sample_time_bits: self.sample_time.to_bits(),
            trigger_input_connected,
            nominal_screen_seconds: nominal,
            next_bin_samples,
        });
    }
}
