//! Headless reference adapter API. Production bank preparation, oscillator and
//! state adoption are used directly; this module only maps source controls and
//! exposes bounded runtime observations for the separate OWT1 test executable.
use crate::{
    wavetable_bank::PreparedWaveBank, wavetable_osc::WavetableOscillator, ModuleContent, Node,
    Patch, PortFrame, WaveAsset, WavePreparationCache,
};
use std::sync::Arc;

pub struct Oracle {
    kind: u32,
    rate: u32,
    node: Node,
    oscillator: WavetableOscillator,
    bank: Arc<PreparedWaveBank>,
    cache: WavePreparationCache,
    samples: Vec<f32>,
    wave_length: u32,
    loading: bool,
    valid: bool,
}
pub struct Observation {
    pub phases: [f32; 16],
    pub directions: [f32; 16],
    pub last_sync_values: [f32; 16],
    pub clock_frequency: f32,
    pub timer: f32,
    pub last_position: f32,
    pub reset_masks: [u32; 4],
    pub clock_trigger: u32,
    pub loading: bool,
    pub wave_length: u32,
    pub sample_count: u32,
}
impl Oracle {
    pub fn new(kind: u32, sample_rate: u32) -> Result<Self, String> {
        if kind > 1 {
            return Err("Invalid wavetable kind".into());
        }
        let node:Node=serde_json::from_value(serde_json::json!({"id":"wt","kind":if kind==0 {"wavetable_vco"} else {"wavetable_lfo"}})).map_err(|e|e.to_string())?;
        let cache = WavePreparationCache::default();
        let mut patch = Patch::new_v2();
        patch.nodes.push(node.clone());
        let bank = patch.prepare_wave_banks(&cache)?.get("wt").unwrap();
        let oscillator = WavetableOscillator::build(&node, sample_rate, bank.clone())?;
        let samples = crate::wave_prepare::basic_asset().samples().collect();
        Ok(Self {
            kind,
            rate: sample_rate,
            node,
            oscillator,
            bank,
            cache,
            samples,
            wave_length: 1024,
            loading: false,
            valid: true,
        })
    }
    fn rebuild(&mut self) -> Result<(), String> {
        self.valid = self.wave_length >= 2 && self.samples.len() >= self.wave_length as usize;
        if !self.valid {
            return Ok(());
        }
        let asset = Arc::new(WaveAsset::new(&self.samples)?);
        let mut patch = Patch::new_v2();
        let mut node = self.node.clone();
        node.content = Some(ModuleContent::Wavetable {
            asset_id: asset.id().into(),
            wave_points: self.wave_length,
            name: "Synthetic.f32".into(),
        });
        patch.assets.insert(asset.id().into(), asset);
        patch.nodes.push(node);
        let bank = patch.prepare_wave_banks(&self.cache)?.get("wt").unwrap();
        let mut next = WavetableOscillator::build(&self.node, self.rate, bank.clone())?;
        assert!(next.adopt_from(&self.oscillator));
        self.oscillator = next;
        self.bank = bank;
        Ok(())
    }
    pub fn replace_bank(&mut self, wave_length: u32, samples: Vec<f32>) -> Result<(), String> {
        if samples.len() > crate::wavetable_file::MAX_WAVETABLE_SAMPLES
            || samples.iter().any(|v| !v.is_finite())
        {
            return Err("Invalid source fixture bank".into());
        }
        self.wave_length = wave_length;
        self.samples = samples;
        self.loading = false;
        self.rebuild()
    }
    pub fn reset_bank(&mut self) -> Result<(), String> {
        self.replace_bank(1024, crate::wave_prepare::basic_asset().samples().collect())
    }
    pub fn reset_source(&mut self) -> Result<(), String> {
        self.reset_bank()?;
        self.oscillator.initialize_state();
        Ok(())
    }
    pub fn set_loading(&mut self, value: bool) {
        self.loading = value;
    }
    pub fn set_wave_length(&mut self, value: u32) -> Result<(), String> {
        self.wave_length = value;
        self.rebuild()
    }
    pub fn adopt_current_state(&mut self) -> Result<(), String> {
        let mut next = WavetableOscillator::build(&self.node, self.rate, self.bank.clone())?;
        assert!(next.adopt_from(&self.oscillator));
        self.oscillator = next;
        Ok(())
    }
    pub fn set_source_parameters(&mut self, values: &[f32]) -> Result<(), String> {
        let (names, indices): (&[&str], &[usize]) = if self.kind == 0 {
            (
                &[
                    "tune",
                    "position",
                    "fm_amount",
                    "position_cv_amount",
                    "sync_mode",
                    "fm_mode",
                ],
                &[2, 3, 4, 5, 1, 6],
            )
        } else {
            (
                &[
                    "rate",
                    "position",
                    "fm_amount",
                    "position_cv_amount",
                    "offset",
                    "invert",
                ],
                &[2, 3, 4, 5, 0, 1],
            )
        };
        if values.len() != if self.kind == 0 { 7 } else { 6 } {
            return Err("Invalid source parameter count".into());
        }
        for (name, index) in names.iter().zip(indices) {
            self.node.params.insert((*name).into(), values[*index]);
        }
        self.adopt_current_state()
    }
    pub fn process(&mut self, inputs: &[PortFrame; 4]) -> PortFrame {
        if self.valid && !self.loading {
            self.oscillator.process(inputs)
        } else {
            self.oscillator.process_suspended(inputs)
        }
    }
    pub fn observation(&self) -> Observation {
        let s = self.oscillator.snapshot();
        let mut reset_masks = [0; 4];
        if self.kind == 1 {
            for (group, mask) in reset_masks.iter_mut().enumerate() {
                for lane in 0..4 {
                    if s.reset_high[group * 4 + lane] {
                        *mask |= 1 << lane;
                    }
                }
            }
        }
        Observation {
            phases: s.phases,
            directions: if self.kind == 0 {
                s.directions
            } else {
                [0.; 16]
            },
            last_sync_values: if self.kind == 0 {
                s.last_sync_values
            } else {
                [0.; 16]
            },
            clock_frequency: if self.kind == 1 {
                s.clock_frequency
            } else {
                0.
            },
            timer: if self.kind == 1 { s.timer } else { 0. },
            last_position: s.last_position,
            reset_masks,
            clock_trigger: if self.kind == 1 {
                s.clock_trigger as u32
            } else {
                0
            },
            loading: self.loading,
            wave_length: self.wave_length,
            sample_count: self.samples.len() as u32,
        }
    }
}
