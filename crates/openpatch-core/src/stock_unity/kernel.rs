//! Unity's two mixing sections, meters and lights, following the f32
//! operation order of the pinned Fundamental `src/Unity.cpp` and Rack's
//! `dsp::VuMeter2`/`dsp::ClockDivider` (GPL-3.0-or-later, see
//! LICENSE-THIRD-PARTY). Every add/sub/mul/div applies the Rack Engine's
//! flush-to-zero contract through `native_float`. Self-contained apart from
//! `crate::native_float`, so the reference runner can compile it directly.
use crate::native_float::{rack_add, rack_div, rack_mul, rack_sub};

/// Rack `ClockDivider` division the source sets for its lights.
pub const LIGHT_DIVISION: u32 = 256;
/// `VuMeter2::lambda` (inverse time constant, 1/s).
pub const METER_LAMBDA: f32 = 30.0;

/// One input as the module reads it: connected (at least one channel) and
/// the first channel's voltage. Unity ignores channels above the first.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UnityInput {
    pub connected: bool,
    pub voltage: f32,
}

/// Durable and running state of one Unity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnityKernel {
    /// "Merge channels 1 & 2" (saved as module data `merge`).
    pub merge: bool,
    /// `VuMeter2::v` of each section (peak mode).
    pub meters: [f32; 2],
    /// `ClockDivider::clock`.
    pub clock: u32,
    /// Light brightness: section 1 clip, -6, -12, -18, -24 dB, then section 2.
    pub lights: [f32; 10],
    /// `ProcessArgs::sampleTime`.
    pub sample_time: f32,
}

/// `VuMeter2::getBrightness(dbMin, dbMax)` for a peak meter.
pub fn brightness(v: f32, db_min: f32, db_max: f32) -> f32 {
    let db = rack_mul(v.log10(), 20.0);
    if db >= db_max {
        1.0
    } else if db <= db_min {
        0.0
    } else {
        // math::rescale(db, dbMin, dbMax, 0, 1).
        rack_add(
            0.0,
            rack_mul(
                rack_div(rack_sub(db, db_min), rack_sub(db_max, db_min)),
                rack_sub(1.0, 0.0),
            ),
        )
    }
}

impl UnityKernel {
    pub fn new(sample_rate: u32) -> Self {
        Self {
            merge: false,
            meters: [0.0; 2],
            clock: 0,
            lights: [0.0; 10],
            sample_time: 1.0_f32 / sample_rate as f32,
        }
    }

    /// One frame. `average` holds the two mode switch values (0 = sum,
    /// 1 = average). Returns MIX 1, INV 1, MIX 2, INV 2.
    pub fn process(&mut self, inputs: &[UnityInput; 12], average: [f32; 2]) -> [f32; 4] {
        let mut mix = [0.0_f32; 2];
        let mut count = [0_i32; 2];
        for section in 0..2 {
            for input in &inputs[section * 6..section * 6 + 6] {
                mix[section] = rack_add(mix[section], input.voltage);
                if input.connected {
                    count[section] += 1;
                }
            }
        }
        if self.merge {
            mix[0] = rack_add(mix[0], mix[1]);
            mix[1] = mix[0];
            count[0] += count[1];
            count[1] = count[0];
        }
        let mut outputs = [0.0_f32; 4];
        for section in 0..2 {
            if count[section] > 0 && average[section].round() as i32 == 1 {
                mix[section] = rack_div(mix[section], count[section] as f32);
            }
            outputs[2 * section] = mix[section];
            outputs[2 * section + 1] = -mix[section];
            let value = rack_div(mix[section], 10.0).abs();
            let meter = &mut self.meters[section];
            if value >= *meter {
                *meter = value;
            } else {
                *meter = rack_add(
                    *meter,
                    rack_mul(
                        rack_mul(rack_sub(value, *meter), METER_LAMBDA),
                        self.sample_time,
                    ),
                );
            }
        }
        self.clock += 1;
        if self.clock >= LIGHT_DIVISION {
            self.clock = 0;
            for section in 0..2 {
                let v = self.meters[section];
                self.lights[5 * section] = brightness(v, 0.0, 0.0);
                for led in 1..5 {
                    let db_min = rack_mul(-6.0, (led + 1) as f32);
                    let db_max = rack_mul(-6.0, led as f32);
                    self.lights[5 * section + led] = brightness(v, db_min, db_max);
                }
            }
        }
        outputs
    }
}
