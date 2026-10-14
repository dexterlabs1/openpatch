//! Original input-only polyphonic polarity visualizer.
use crate::{ModuleLightState, PortFrame, MAX_MODULE_LIGHTS};

#[derive(Debug)]
pub(crate) struct Viz {
    sample_time: f32,
    divider: u8,
    channels: u8,
    brightness: [[f32; 2]; MAX_MODULE_LIGHTS],
}
impl Viz {
    pub(crate) fn new(rate: u32) -> Self {
        Self {
            sample_time: (1.0 / rate as f32) * 16.0,
            divider: 0,
            channels: 0,
            brightness: [[0.0; 2]; MAX_MODULE_LIGHTS],
        }
    }
    pub(crate) fn process(&mut self, input: PortFrame) {
        self.divider += 1;
        if self.divider != 16 {
            return;
        }
        self.divider = 0;
        self.channels = input.channels.min(16);
        for (i, pair) in self.brightness.iter_mut().enumerate() {
            let voltage = input.channel(i) / 10.0;
            for (value, target) in pair.iter_mut().zip([voltage, -voltage]) {
                if target < *value {
                    *value += (target - *value) * 30.0 * self.sample_time;
                } else {
                    *value = target;
                }
            }
        }
    }
    pub(crate) fn adopt(&mut self, previous: &Self) {
        self.divider = previous.divider;
        self.channels = previous.channels;
        self.brightness = previous.brightness;
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        ModuleLightState {
            count: 16,
            input_channels: Some(self.channels),
            brightness: self.brightness,
            manual: false,
        }
    }
}
