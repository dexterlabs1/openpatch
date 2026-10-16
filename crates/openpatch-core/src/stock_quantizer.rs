//! Original fixed-storage realization of the stock Quantizer behavioral contract.
//! The pinned Fundamental model defines half-semitone bins and integer-distance
//! ties. No upstream implementation, table, or artwork is embedded here.
use crate::{parameter, ModuleLightState, Node, PortFrame, SavedModuleState, MAX_CHANNELS};
use serde_json::{json, Value};
pub(crate) const INPUTS: [&str; 1] = ["pitch"];
pub(crate) const OUTPUTS: [&str; 1] = ["pitch"];
const ALL: u16 = 0x0fff;
pub(crate) fn catalog() -> Value {
    json!({"kind":"stock_quantizer","title":"STOCK QUANTIZER","model":"functional","description":"Stock half-semitone lookup quantizer with durable twelve-note bank, source-order f32 pitch arithmetic, true pitch bypass and actual playing-note display metadata. Generic quantizer remains independent.","inputs":INPUTS,"params":{
        "offset":{"default":0.0,"min":-1.0,"max":1.0,"unit":"V","description":"Pre-offset, displayed as12 semitones per volt"},
        "scale_mask":{"default":4095.0,"min":0.0,"max":4095.0,"step":1.0,"unit":"12-bit mask","hidden":true,"description":"Durable enabled-note bank, C through B; zero selects chromatic fallback"}
    }})
}
pub(crate) fn annotate(module: &mut Value) {
    module["functional_reference"]=json!("https://raw.githubusercontent.com/VCVRack/Fundamental/10dd0160c664770910e5584b7b00498cc48d9ddd/src/Quantizer.cpp");
    module["polyphony"]["primary_input"] = json!("pitch");
    module["polyphony"]["version_2"]=json!("max(input width,1), at most16; no mono broadcast; source reads raw input slots, including slot0 for empty input; disconnected outputs report0 channels while display still processes");
    module["polyphony"]["auxiliary_inputs"] = json!("no auxiliary inputs or mono broadcast");
    module["polyphony"]["retired_voices"]=json!("no per-voice runtime; actual playing union is replaced across current voices every processed frame");
    module["randomization_policy"]=json!("OpenPatch seeded uniform selection among4096 masks, equivalent to12 independent fair note bits without endpoint bias; offset uses normal uniform rawV control randomization; RNG stream is not Rack RNG equivalence");
    module["quantization_policy"]=json!("f32 input + offset, floor(f32(pitch*24)), Euclidean octave/bin decomposition,24-entry enabled-note table, integer note converted to f32 then divided by12; table targets floor((bin+1)/2), nearest enabled integer note with lower ties; empty bank quantizes chromatically, never passes through");
    module["display_policy"]=json!("NUM_LIGHTS0: count0 and zero brightness, optional playing_notes bitmask is actual union across processed voices; None before first process, Some thereafter even disconnected; reset,bank edits,legacy load,rotation and bypass retain last observation until next process");
    module["initialization_policy"]=json!("Initialize enables all twelve notes, recomputes ranges, and restores offset through host parameter defaults; does not clear last playing-note observation");
    module["state_adoption"]=json!("same identity/kind/rate preserves last playing observation; receiving offset and enabled bank win; explicit full saved-bank assignment after adoption may replace bank");
    module["arithmetic_policy"]=json!("exact normal source domain requires safely representable source int conversions and octave arithmetic; nonfinite input becomes0; outside that domain Rust-only safety uses defined saturating f32-to-i32 conversion and i64 Euclidean math, with no arbitrary CV amplitude clamp; inactive/disconnected raw output storage follows host zero padding and is excluded from SDK equivalence");
    module["bypass"]["routes"] = json!([{"input":"pitch","output":"pitch"}]);
    module["bypass"]["output_policy"]=json!("actual connected context: raw active pitch slots, no offset/quantization; connected empty input gives mono+0, disconnected output stays0 channels; display observation freezes");
}
/// Rotate a twelve-bit enabled bank by signed semitones; positive shifts up.
pub fn rotate_stock_quantizer_mask(mask: u16, semitones: i32) -> u16 {
    let shift = semitones.rem_euclid(12) as u32;
    let mask = mask & ALL;
    if shift == 0 {
        mask
    } else {
        ((mask << shift) | (mask >> (12 - shift))) & ALL
    }
}
fn ranges(mask: u16) -> [i32; 24] {
    std::array::from_fn(|bin| {
        let target = bin.div_ceil(2) as i32;
        let mut best = 0;
        let mut distance = i32::MAX;
        for note in -12_i32..=24 {
            if mask != 0 && mask & (1 << note.rem_euclid(12)) == 0 {
                continue;
            }
            let candidate = (target - note).abs();
            if candidate < distance {
                best = note;
                distance = candidate;
            }
        }
        best
    })
}
/// Exact production state, with no fabricated initial playing-note observation.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct StockQuantizerSnapshot {
    pub enabled_mask: u16,
    pub ranges: [i32; 24],
    pub playing_notes: Option<u16>,
    pub offset: f32,
}
#[derive(Debug)]
pub(crate) struct StockQuantizer {
    sample_rate: u32,
    state: StockQuantizerSnapshot,
}
impl StockQuantizer {
    pub(crate) fn build(node: &Node, rate: u32) -> Result<Self, String> {
        if !(8000..=192000).contains(&rate) {
            return Err("stock_quantizer sample rate must be8000..=192000 Hz".into());
        }
        for name in node.params.keys() {
            if !["offset", "scale_mask"].contains(&name.as_str()) {
                return Err(format!("node '{}': unknown parameter '{name}'", node.id));
            }
        }
        let mask = parameter(node, "scale_mask", 4095.0, 0.0, 4095.0)?;
        if mask.fract() != 0.0 {
            return Err("stock_quantizer scale_mask must be an integer".into());
        }
        let mask = mask as u16;
        Ok(Self {
            sample_rate: rate,
            state: StockQuantizerSnapshot {
                enabled_mask: mask,
                ranges: ranges(mask),
                playing_notes: None,
                offset: parameter(node, "offset", 0.0, -1.0, 1.0)?,
            },
        })
    }
    pub(crate) fn set_saved_state(&mut self, saved: SavedModuleState) -> bool {
        if let SavedModuleState::StockQuantizerNotes(mask @ 0..=ALL) = saved {
            self.set_mask(mask);
            true
        } else {
            false
        }
    }
    fn set_mask(&mut self, mask: u16) {
        self.state.enabled_mask = mask;
        self.state.ranges = ranges(mask);
    }
    pub(crate) fn initialize_state(&mut self) {
        self.set_mask(ALL);
    }
    pub(crate) fn adopt(&mut self, old: &Self) {
        if self.sample_rate == old.sample_rate {
            self.state.playing_notes = old.state.playing_notes;
        }
    }
    pub(crate) fn saved_state(&self) -> SavedModuleState {
        SavedModuleState::StockQuantizerNotes(self.state.enabled_mask)
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            playing_notes: self.state.playing_notes,
            ..Default::default()
        }
    }
    fn set_offset(&mut self, value: f32) -> Result<(), String> {
        if !value.is_finite() || !(-1.0..=1.0).contains(&value) {
            return Err("stock_quantizer offset must be finite and in[-1,1] V".into());
        }
        self.state.offset = value;
        Ok(())
    }
    fn rotate(&mut self, delta: i32) {
        self.set_mask(rotate_stock_quantizer_mask(self.state.enabled_mask, delta));
    }
    fn load_legacy_data(&mut self, data: &Value) {
        let mut mask = self.state.enabled_mask;
        if let Some(notes) = data.get("enabledNotes").and_then(Value::as_array) {
            for (i, value) in notes.iter().take(12).enumerate() {
                if value.as_bool() == Some(true) {
                    mask |= 1 << i;
                } else {
                    mask &= !(1 << i);
                }
            }
        }
        self.set_mask(mask);
    }
    pub(crate) fn process(
        &mut self,
        input: &PortFrame,
        connected: bool,
        bypassed: bool,
    ) -> PortFrame {
        let active = usize::from(input.channels).min(MAX_CHANNELS);
        let channels = active.max(1);
        let mut output = PortFrame {
            channels: if connected { channels as u8 } else { 0 },
            ..Default::default()
        };
        if bypassed {
            if connected {
                output.values[..active].copy_from_slice(&input.values[..active]);
            }
            return output;
        }
        let mut playing = 0;
        for lane in 0..channels {
            let raw = input.values[lane];
            let raw = if raw.is_finite() { raw } else { 0.0 };
            let pitch = raw + self.state.offset;
            // Rust defines saturation for out-of-range float-to-int casts. i64
            // Euclidean arithmetic also covers the signed i32 endpoint safely.
            // This extension is not numerical SDK parity outside its int domain.
            let bin = (pitch * 24.0).floor() as i32;
            let octave = i64::from(bin).div_euclid(24);
            let index = bin.rem_euclid(24) as usize;
            let note = i64::from(self.state.ranges[index]) + octave * 12;
            playing |= 1 << note.rem_euclid(12);
            if connected {
                output.values[lane] = note as f32 / 12.0;
            }
        }
        self.state.playing_notes = Some(playing);
        output
    }
}
/// Reference wrapper around the same production processor used by Engine.
#[doc(hidden)]
#[derive(Debug)]
pub struct StockQuantizerProbe(StockQuantizer);
impl StockQuantizerProbe {
    pub fn new(node: &Node, rate: u32) -> Result<Self, String> {
        StockQuantizer::build(node, rate).map(Self)
    }
    pub fn step(&mut self, input: &PortFrame, output_connected: bool, bypassed: bool) -> PortFrame {
        self.0.process(input, output_connected, bypassed)
    }
    pub fn snapshot(&self) -> StockQuantizerSnapshot {
        self.0.state
    }
    pub fn initialize(&mut self) {
        self.0.initialize_state();
    }
    pub fn adopt(&mut self, old: &Self) {
        self.0.adopt(&old.0);
    }
    pub fn set_saved_state(&mut self, state: SavedModuleState) -> bool {
        self.0.set_saved_state(state)
    }
    pub fn set_offset(&mut self, value: f32) -> Result<(), String> {
        self.0.set_offset(value)
    }
    pub fn rotate(&mut self, delta: i32) {
        self.0.rotate(delta);
    }
    pub fn load_legacy_data(&mut self, data: &Value) {
        self.0.load_legacy_data(data);
    }
    /// Text-protocol convenience that still invokes the actual JSON loader.
    pub fn load_legacy_notes(&mut self, length: i32, mask: u16) -> Result<(), String> {
        if !(-1..=14).contains(&length) {
            return Err("legacy note array length must be-1..=14".into());
        }
        let data = if length < 0 {
            json!({})
        } else {
            json!({"enabledNotes":(0..length).map(|i|mask&(1<<i)!=0).collect::<Vec<_>>()})
        };
        self.0.load_legacy_data(&data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn model(mask: u16) -> StockQuantizer {
        StockQuantizer::build(
            &serde_json::from_value(
                json!({"id":"q","kind":"stock_quantizer","params":{"scale_mask":mask}}),
            )
            .unwrap(),
            48000,
        )
        .unwrap()
    }
    #[test]
    fn all_banks_obey_independent_distance_search_and_lower_table_ties() {
        for mask in 0..=ALL {
            let table = ranges(mask);
            for (bin, actual) in table.into_iter().enumerate() {
                let target = bin.div_ceil(2) as i32;
                let allowed = |note: i32| mask == 0 || mask & (1 << note.rem_euclid(12)) != 0;
                let expected = (0..=12)
                    .find_map(|distance| {
                        let low = target - distance;
                        if allowed(low) {
                            Some(low)
                        } else {
                            let high = target + distance;
                            allowed(high).then_some(high)
                        }
                    })
                    .unwrap();
                assert_eq!(actual, expected, "bank{mask},bin{bin}");
            }
        }
        assert_eq!(&ranges(5)[..5], &[0, 0, 0, 2, 2]);
        assert_eq!(ranges(0), ranges(ALL));
    }
    #[test]
    fn source_half_cell_boundary_is_not_continuous_nearest_enabled_pitch() {
        let mut m = model(5);
        assert_eq!(
            m.process(&PortFrame::mono(0.1), true, false).values[0],
            0.0,
            "C/D bank chooses lower table tie after preliminary half-semitone bin"
        );
        let boundary = 0.125_f32;
        let below = f32::from_bits(boundary.to_bits() - 1);
        assert_eq!(
            m.process(&PortFrame::mono(below), true, false).values[0],
            0.0
        );
        assert_eq!(
            m.process(&PortFrame::mono(boundary), true, false).values[0],
            2.0 / 12.0
        );
        assert_eq!(
            m.process(&PortFrame::mono(-0.875), true, false).values[0],
            -10.0 / 12.0
        );
        let below = f32::from_bits((-0.875_f32).to_bits() + 1);
        assert_eq!(
            m.process(&PortFrame::mono(below), true, false).values[0],
            -1.0
        );
    }
    #[test]
    fn empty_bank_is_chromatic_and_real_display_union_includes_disabled_notes() {
        let mut m = model(0);
        let input = PortFrame {
            channels: 4,
            values: std::array::from_fn(|i| (i % 4) as f32 / 4.0),
        };
        let out = m.process(&input, true, false);
        assert_eq!(out.channels, 4);
        assert_eq!(&out.values[..4], &[0.0, 0.25, 0.5, 0.75]);
        assert_eq!(
            m.state.playing_notes,
            Some((1 << 0) | (1 << 3) | (1 << 6) | (1 << 9))
        );
        let display = m.lights();
        assert_eq!(display.count, 0);
        assert_eq!(display.brightness, [[0.0; 2]; crate::MAX_MODULE_LIGHTS]);
        assert_eq!(display.playing_notes, m.state.playing_notes);
    }
    #[test]
    fn all_widths_and_disconnected_consumer_use_actual_processor_context() {
        for channels in 0..=16 {
            let mut m = model(ALL);
            assert_eq!(m.state.playing_notes, None);
            let input = PortFrame {
                channels,
                values: std::array::from_fn(|i| (i % 4) as f32 / 4.0),
            };
            let disconnected = m.process(&input, false, false);
            assert_eq!(disconnected.channels, 0);
            assert_eq!(disconnected.values, [0.0; 16]);
            assert!(m.state.playing_notes.is_some());
            let out = m.process(&input, true, false);
            assert_eq!(out.channels, channels.max(1));
            assert!(out.values[usize::from(out.channels)..]
                .iter()
                .all(|v| v.to_bits() == 0));
        }
        let mut m = model(ALL);
        let mut empty = PortFrame::default();
        empty.values[0] = 0.25;
        assert_eq!(
            m.process(&empty, true, false).values[0],
            0.25,
            "source reads raw slot0 even when input width0"
        );
    }
    #[test]
    fn bypass_is_raw_pitch_without_offset_or_display_and_keeps_connected_empty_mono() {
        let mut m = model(1);
        m.set_offset(1.0).unwrap();
        let input = PortFrame {
            channels: 3,
            values: std::array::from_fn(|i| match i {
                0 => -0.0,
                1 => 0.123,
                2 => -1.7,
                _ => 99.0,
            }),
        };
        let out = m.process(&input, true, true);
        assert_eq!(out.channels, 3);
        for i in 0..3 {
            assert_eq!(out.values[i].to_bits(), input.values[i].to_bits());
        }
        assert_eq!(m.state.playing_notes, None);
        assert_eq!(
            m.process(&PortFrame::default(), true, true),
            PortFrame::mono(0.0)
        );
        assert_eq!(m.process(&input, false, true), PortFrame::default());
        m.process(&PortFrame::mono(0.0), true, false);
        let playing = m.state.playing_notes;
        m.process(&PortFrame::mono(0.4), true, true);
        assert_eq!(m.state.playing_notes, playing);
    }
    #[test]
    fn reset_load_rotate_and_compatible_adoption_retain_observation_receiving_controls_win() {
        let mut old = model(1);
        old.process(&PortFrame::mono(0.25), true, false);
        let observed = old.state.playing_notes;
        let mut receiving = model(4);
        receiving.set_offset(-0.5).unwrap();
        receiving.adopt(&old);
        assert_eq!(receiving.state.enabled_mask, 4);
        assert_eq!(receiving.state.offset, -0.5);
        assert_eq!(receiving.state.playing_notes, observed);
        receiving.rotate(1);
        assert_eq!(receiving.state.enabled_mask, 8);
        assert_eq!(receiving.state.playing_notes, observed);
        assert!(receiving.set_saved_state(SavedModuleState::StockQuantizerNotes(0)));
        assert_eq!(receiving.state.playing_notes, observed);
        receiving.initialize_state();
        assert_eq!(receiving.state.enabled_mask, ALL);
        assert_eq!(receiving.state.playing_notes, observed);
        assert_eq!(receiving.state.offset, -0.5);
        assert!(!receiving.set_saved_state(SavedModuleState::StockQuantizerNotes(4096)));
        let node: Node =
            serde_json::from_value(json!({"id":"q","kind":"stock_quantizer"})).unwrap();
        let mut other_rate = StockQuantizer::build(&node, 96000).unwrap();
        other_rate.adopt(&old);
        assert_eq!(other_rate.state.playing_notes, None);
    }
    #[test]
    fn actual_legacy_loader_preserves_missing_tail_and_observation_and_uses_json_booleans() {
        let mut m = model(ALL);
        m.process(&PortFrame::mono(0.25), true, false);
        let playing = m.state.playing_notes;
        for data in [
            json!({}),
            json!({"enabledNotes":null}),
            json!({"enabledNotes":false}),
            json!({"enabledNotes":123}),
            json!({"enabledNotes":"true"}),
            json!({"enabledNotes":{}}),
            json!([]),
        ] {
            m.load_legacy_data(&data);
            assert_eq!(m.state.enabled_mask, ALL);
            assert_eq!(m.state.playing_notes, playing);
        }
        m.load_legacy_data(&json!({"enabledNotes":[false,true,1,"true",null,{},[],true]}));
        assert_eq!(m.state.enabled_mask, 0xf82);
        assert_eq!(m.state.playing_notes, playing);
        m.load_legacy_data(&json!({"enabledNotes":[]}));
        assert_eq!(m.state.enabled_mask, 0xf82);
        m.load_legacy_data(&json!({"enabledNotes":[false,false,false,false,false,false,false,false,false,false,false,false,true,true]}));
        assert_eq!(m.state.enabled_mask, 0);
        assert_eq!(m.state.ranges, ranges(ALL));
    }
    #[test]
    fn every_mask_rotation_is_periodic_invertible_and_composes_for_signed_deltas() {
        for mask in 0..=ALL {
            for delta in -13..=13 {
                let shifted = rotate_stock_quantizer_mask(mask, delta);
                assert_eq!(rotate_stock_quantizer_mask(shifted, -delta), mask);
                assert_eq!(
                    rotate_stock_quantizer_mask(shifted, 3),
                    rotate_stock_quantizer_mask(mask, delta + 3)
                );
            }
            assert_eq!(
                rotate_stock_quantizer_mask(mask, i32::MIN),
                rotate_stock_quantizer_mask(mask, i32::MIN.rem_euclid(12))
            );
        }
        assert_eq!(rotate_stock_quantizer_mask(1, 1), 2);
        assert_eq!(rotate_stock_quantizer_mask(1, -1), 1 << 11);
    }
    #[test]
    fn defined_native_extreme_extension_is_finite_and_nonfinite_maps_to_zero() {
        let mut m = model(ALL);
        for value in [
            f32::MAX,
            -f32::MAX,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
            0.0,
            -0.0,
        ] {
            let out = m.process(&PortFrame::mono(value), true, false);
            assert!(out.values[0].is_finite());
            assert!(m
                .state
                .playing_notes
                .is_some_and(|mask| mask > 0 && mask <= ALL));
            if !value.is_finite() {
                assert_eq!(out.values[0], 0.0);
            }
        }
        assert!(m.set_offset(f32::NAN).is_err());
        assert!(m.set_offset(1.01).is_err());
    }
}
