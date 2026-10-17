// Copyright 2013 Emilie Gillet.
// Native translation of the pinned Frames keyframer; MIT license and complete
// permission notice: scripts/frames_reference/NOTICE.md.
//! Fixed-capacity integer keyframe interpolation. No allocation or floating point.
//! `new` declares a finite construction: zero storage, Clear, linear settings,
//! response zero and zero immediate values. Upstream TEST Init itself is a no-op.
#[path = "tables.rs"]
mod tables;
use tables::*;

pub const CHANNELS: usize = 4;
pub const CAPACITY: usize = 64;
const PALETTE: [[u8; 3]; 8] = [
    [255, 0, 0],
    [255, 64, 0],
    [255, 255, 0],
    [64, 255, 0],
    [0, 255, 64],
    [0, 0, 255],
    [255, 0, 255],
    [255, 0, 64],
];
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum EasingCurve {
    Step = 0,
    #[default]
    Linear = 1,
    InQuartic = 2,
    OutQuartic = 3,
    Sine = 4,
    Bounce = 5,
}
impl EasingCurve {
    pub fn from_index(index: u8) -> Self {
        match index {
            0 => Self::Step,
            2 => Self::InQuartic,
            3 => Self::OutQuartic,
            4 => Self::Sine,
            5 => Self::Bounce,
            _ => Self::Linear,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChannelSettings {
    pub easing_curve: EasingCurve,
    pub response: u8,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Keyframe {
    pub timestamp: u16,
    pub id: u16,
    pub values: [u16; CHANNELS],
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Evaluation {
    pub levels: [u16; CHANNELS],
    pub dac_code: [u16; CHANNELS],
    pub color: [u8; 3],
    pub position: i16,
    pub nearest_keyframe: i16,
}
#[derive(Clone, Debug)]
pub struct FrameKernel {
    keyframes: [Keyframe; CAPACITY],
    settings: [ChannelSettings; CHANNELS],
    num_keyframes: usize,
    id_counter: u16,
    immediate: [u16; CHANNELS],
    output: Evaluation,
}
impl Default for FrameKernel {
    fn default() -> Self {
        Self::new()
    }
}
impl FrameKernel {
    pub fn new() -> Self {
        Self {
            keyframes: [Keyframe::default(); CAPACITY],
            settings: [ChannelSettings::default(); CHANNELS],
            num_keyframes: 0,
            id_counter: 0,
            immediate: [0; CHANNELS],
            output: Evaluation::default(),
        }
    }
    /// Clear preserves settings, immediate values and last outputs, as upstream.
    pub fn clear(&mut self) {
        self.keyframes.fill(Keyframe::default());
        self.num_keyframes = 0;
        self.id_counter = 0;
    }
    /// Control-thread restore preserving source palette IDs. Invalid input is atomic.
    pub fn load_keyframes(&mut self, keys: &[Keyframe]) -> bool {
        if keys.len() > CAPACITY
            || keys
                .windows(2)
                .any(|pair| pair[0].timestamp >= pair[1].timestamp)
        {
            return false;
        }
        self.clear();
        self.keyframes[..keys.len()].copy_from_slice(keys);
        self.num_keyframes = keys.len();
        self.id_counter = keys
            .iter()
            .map(|key| key.id)
            .max()
            .map_or(0, |id| id.wrapping_add(1));
        true
    }
    pub fn num_keyframes(&self) -> usize {
        self.num_keyframes
    }
    pub fn keyframe(&self, index: usize) -> Option<&Keyframe> {
        self.keyframes
            .get(index)
            .filter(|_| index < self.num_keyframes)
    }
    pub fn settings(&self, channel: usize) -> &ChannelSettings {
        &self.settings[channel]
    }
    pub fn set_settings(&mut self, channel: usize, settings: ChannelSettings) {
        self.settings[channel] = settings;
    }
    pub fn set_immediate(&mut self, channel: usize, value: u16) {
        self.immediate[channel] = value;
    }
    pub fn output(&self) -> Evaluation {
        self.output
    }
    fn find_keyframe(&self, timestamp: u16) -> usize {
        let mut first = 0;
        let mut count = self.num_keyframes;
        while count != 0 {
            let step = count / 2;
            let middle = first + step;
            if self.keyframes[middle].timestamp < timestamp {
                first = middle + 1;
                count -= step + 1;
            } else {
                count = step;
            }
        }
        first
    }
    pub fn add_keyframe(&mut self, timestamp: u16, values: [u16; CHANNELS]) -> bool {
        // At capacity even replacement is rejected by the original.
        if self.num_keyframes == CAPACITY {
            return false;
        }
        let insertion = self.find_keyframe(timestamp);
        if insertion >= self.num_keyframes || self.keyframes[insertion].timestamp != timestamp {
            self.keyframes
                .copy_within(insertion..self.num_keyframes, insertion + 1);
            self.keyframes[insertion] = Keyframe {
                timestamp,
                id: self.id_counter,
                values,
            };
            self.id_counter = self.id_counter.wrapping_add(1);
            self.num_keyframes += 1;
        }
        self.keyframes[insertion].values = values;
        true
    }
    pub fn remove_keyframe(&mut self, timestamp: u16) -> bool {
        if self.num_keyframes == 0 {
            return false;
        }
        let splice = self.find_keyframe(timestamp);
        // Preserve upstream stale-slot matches when storage exists; only reject
        // the actual capacity/afterlast out-of-bounds access.
        if self
            .keyframes
            .get(splice)
            .is_none_or(|key| key.timestamp != timestamp)
        {
            return false;
        }
        if splice < self.num_keyframes {
            self.keyframes
                .copy_within(splice + 1..self.num_keyframes, splice);
        }
        self.num_keyframes -= 1;
        true
    }
    /// Original returns the first qualifying index, with strict tolerance bounds.
    pub fn find_nearest_keyframe(&self, timestamp: u16, tolerance: u16) -> i16 {
        if self.num_keyframes == 0 {
            return -1;
        }
        let index = self.find_keyframe(timestamp);
        let start = index.saturating_sub(1);
        let end = if index < self.num_keyframes - 1 {
            index + 2
        } else {
            self.num_keyframes
        };
        for i in start..end {
            let distance = self.keyframes[i].timestamp as i32 - timestamp as i32;
            if distance < tolerance as i32 && -distance < tolerance as i32 {
                return i as i16;
            }
        }
        -1
    }
    pub fn evaluate(&mut self, timestamp: u16) -> Evaluation {
        let mut output = self.output;
        if self.num_keyframes == 0 {
            output.levels = self.immediate;
            output.color = [255; 3];
            output.position = -1;
            output.nearest_keyframe = -1;
        } else {
            let position = self.find_keyframe(timestamp);
            output.position = position as i16;
            if position == 0 || position == self.num_keyframes {
                let source = self.keyframes[if position == 0 {
                    0
                } else {
                    self.num_keyframes - 1
                }];
                output.levels = source.values;
                output.color = PALETTE[source.id as usize & 7];
            } else {
                let a = self.keyframes[position - 1];
                let b = self.keyframes[position];
                let scale =
                    (((timestamp - a.timestamp) as u32) << 16) / (b.timestamp - a.timestamp) as u32;
                for (channel, level) in output.levels.iter_mut().enumerate() {
                    *level = Self::easing(
                        a.values[channel],
                        b.values[channel],
                        scale,
                        self.settings[channel].easing_curve,
                    );
                }
                for (component, color) in output.color.iter_mut().enumerate() {
                    let from = PALETTE[a.id as usize & 7][component] as i64;
                    let to = PALETTE[b.id as usize & 7][component] as i64;
                    *color = (from + (((to - from) * scale as i64) >> 16)) as u8;
                }
            }
            let previous = if position == 0 {
                0
            } else {
                self.keyframes[position - 1].timestamp
            };
            let t_this = timestamp.wrapping_sub(previous);
            // Preserve unused-slot behavior for <64 keys. At capacity use a zero
            // sentinel instead of upstream's out-of-bounds read after the last key.
            let next = self.keyframes.get(position).map_or(0, |key| key.timestamp);
            let t_next = next.wrapping_sub(timestamp);
            output.nearest_keyframe = if t_next < t_this {
                (position + 1) as i16
            } else {
                position as i16
            };
        }
        for (channel, code) in output.dac_code.iter_mut().enumerate() {
            *code =
                Self::convert_to_dac_code(output.levels[channel], self.settings[channel].response);
        }
        self.output = output;
        output
    }
    pub fn convert_to_dac_code(gain: u16, response: u8) -> u16 {
        let exponential = 65535 - gain as i32;
        let a = LUT_VCA_LINEAR[(gain >> 6) as usize] as i32;
        let b = LUT_VCA_LINEAR[(gain >> 6) as usize + 1] as i32;
        let linear = a + (((b - a) * (((gain as u32) << 10) & 0xffff) as i32) >> 16);
        let balance = LUT_RESPONSE_BALANCE[response as usize] as i32;
        ((linear + (((exponential - linear) * balance) >> 15)) >> 4) as u16
    }
    pub fn easing(from: u16, to: u16, scale: u32, curve: EasingCurve) -> u16 {
        let scale = scale.min(65536);
        let shaped = match curve {
            EasingCurve::Step => {
                if scale < 32768 {
                    0
                } else {
                    65535
                }
            }
            EasingCurve::Linear => scale as i32,
            _ => {
                let table = match curve {
                    EasingCurve::InQuartic => &LUT_EASING_IN_QUARTIC,
                    EasingCurve::OutQuartic => &LUT_EASING_OUT_QUARTIC,
                    EasingCurve::Sine => &LUT_EASING_IN_OUT_SINE,
                    _ => &LUT_EASING_IN_OUT_BOUNCE,
                };
                let a = table[(scale >> 6) as usize] as i32;
                // At scale=65536 the source reads [1025], out of bounds, even
                // though its coefficient is zero. Native explicitly supplies zero.
                let b = table.get((scale >> 6) as usize + 1).copied().unwrap_or(0) as i32;
                // C++ promotes the signed slope to uint32 before this multiply.
                // Descending bounce segments therefore use a logical shift.
                a + ((((b - a) >> 1) as u32).wrapping_mul((scale << 10) & 0xffff) >> 15) as i32
            }
        };
        // Defined native wrapping extension when the original signed bounce
        // multiplication would overflow. Reference positives exclude that domain.
        (from as i32 + ((to as i32 - from as i32).wrapping_mul(shaped >> 1) >> 15)) as u16
    }
    pub fn sample_animation(&self, channel: usize, tick: u16, easing: bool) -> u16 {
        let settings = self.settings[channel];
        let mut sample = Self::easing(
            if tick > 32768 { 65535 } else { 0 },
            if tick > 32768 { 0 } else { 65535 },
            ((tick as u32) << 1) & 0xffff,
            if easing {
                settings.easing_curve
            } else {
                EasingCurve::Linear
            },
        );
        if !easing {
            let linear = sample as i32;
            let exponential = LUT_EXPONENTIAL[(sample >> 8) as usize] as i32;
            sample = (linear
                + (((exponential - linear)
                    * LUT_RESPONSE_BALANCE[settings.response as usize] as i32)
                    >> 15)) as u16;
        }
        sample
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restore_is_atomic_and_preserves_palette_identity() {
        let mut k = FrameKernel::new();
        let keys = [
            Keyframe {
                timestamp: 100,
                id: 6,
                values: [12345; 4],
            },
            Keyframe {
                timestamp: 60000,
                id: 11,
                values: [54321; 4],
            },
        ];
        assert!(k.load_keyframes(&keys));
        assert_eq!(k.evaluate(0).color, PALETTE[6]);
        assert_eq!(k.evaluate(65535).color, PALETTE[3]);
        assert!(!k.load_keyframes(&[keys[1], keys[0]]));
        assert_eq!(k.keyframe(0), Some(&keys[0]));
        assert!(k.add_keyframe(32000, [22222; 4]));
        assert_eq!(k.keyframe(1).unwrap().id, 12);
    }
    #[test]
    fn bounce_preserves_defined_unsigned_table_promotion() {
        // A small enough level delta keeps the source signed product defined,
        // while exposing its unsigned slope interpolation overshoot.
        assert!(FrameKernel::easing(0, 10000, 24001, EasingCurve::Bounce) > 10000);
    }
    #[test]
    fn native_guards_source_undefined_boundaries() {
        let mut k = FrameKernel::new();
        for i in 0..CAPACITY {
            assert!(k.add_keyframe(i as u16 * 1000, [22222; 4]));
        }
        assert_eq!(k.evaluate(65535).levels, [22222; 4]);
        assert!(!k.remove_keyframe(65535));
        k.clear();
        assert!(k.add_keyframe(0, [12345; 4]));
        assert!(k.add_keyframe(65535, [54321; 4]));
        for curve in [
            EasingCurve::InQuartic,
            EasingCurve::OutQuartic,
            EasingCurve::Sine,
            EasingCurve::Bounce,
        ] {
            for channel in 0..CHANNELS {
                k.set_settings(
                    channel,
                    ChannelSettings {
                        easing_curve: curve,
                        response: 0,
                    },
                );
            }
            let output = k.evaluate(65535);
            assert_eq!(output.levels, [54319; 4]);
        }
    }
}
