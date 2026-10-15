//! Passive plug-light observation. Native signal units are divided by ten;
//! legacy normalized audio is intentionally not reinterpreted as volts.
//!
//! The detector follows Rack 8ea470f343058ade6f75f16d503a1eb330e1700c,
//! engine/Module.cpp Port_step, engine/Port.hpp and engine/Light.hpp. Stored
//! brightness is signed and unbounded; visible saturation belongs to the UI.

use std::collections::BTreeMap;

/// Physical endpoint direction, independent of cable draw order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PortDirection {
    Input,
    Output,
}

/// Last acquired signal observation, before GUI brightness saturation.
/// `connected` means a physical cable endpoint; a connected zero-channel frame
/// remains distinct from an unplugged endpoint. Invalid measurements reset only
/// this observer, never DSP frames or histories.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PortLightState {
    pub connected: bool,
    pub channels: u8,
    pub raw_rgb: [f32; 3],
    pub invalid: bool,
}

/// An opaque endpoint bound to one compiled engine. Re-resolve after replacement.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedPortLight {
    pub(crate) owner: u64,
    pub(crate) index: Option<usize>,
}

/// One seven-sample acquisition, reusable by standalone numerical reference
/// probes. Only `channels` active values are read. Bad widths, sample rates,
/// nonfinite active voltages, arithmetic overflow or corrupt prior brightness
/// produce finite dark diagnostic state. Inactive lanes are never inspected.
///
/// The poly norm is sqrt(sum(v²)), without a channel-count divisor. Rack's
/// `std::pow(float, int)` promotes to double, with a float sum assignment after
/// each lane. Smoothing retains signed targets and uses source expression order.
pub fn step_port_light(
    previous: PortLightState,
    connected: bool,
    channels: u8,
    values: &[f32],
    sample_rate: u32,
) -> PortLightState {
    if !connected {
        return PortLightState::default();
    }
    let mut state = PortLightState {
        connected,
        channels,
        ..PortLightState::default()
    };
    let count = usize::from(channels);
    if count > crate::MAX_CHANNELS
        || count > values.len()
        || sample_rate == 0
        || values[..count].iter().any(|v| !v.is_finite())
    {
        state.invalid = true;
        return state;
    }
    if count == 0 {
        return state;
    }
    // Invalid input resets history; the next valid acquisition starts at zero.
    let old = if previous.invalid {
        [0.0; 3]
    } else {
        previous.raw_rgb
    };
    if old.iter().any(|v| !v.is_finite()) {
        state.invalid = true;
        return state;
    }
    let dt = 7.0 * (1.0 / sample_rate as f32);
    let smooth = |old: f32, target: f32| {
        if target < old {
            old + (target - old) * 30.0 * dt
        } else {
            target
        }
    };
    if count == 1 {
        let voltage = values[0] / 10.0;
        state.raw_rgb = [smooth(old[0], -voltage), smooth(old[1], voltage), 0.0];
    } else {
        let mut sum = 0.0_f32;
        for &value in &values[..count] {
            sum = (f64::from(sum) + f64::from(value) * f64::from(value)) as f32;
        }
        let norm = sum.sqrt() / 10.0;
        state.raw_rgb = [0.0, 0.0, smooth(old[2], norm)];
    }
    if state.raw_rgb.iter().any(|v| !v.is_finite()) {
        state.raw_rgb = [0.0; 3];
        state.invalid = true;
    }
    state
}

#[derive(Debug)]
pub(crate) struct Binding {
    pub node: usize,
    pub direction: PortDirection,
    pub port: usize,
    // Version 2: input group or flattened output. Version 1: source buffer.
    pub source: usize,
    pub state: Option<PortLightState>,
}

/// All heap storage is compiled on the control thread, even while disabled.
/// There is one binding per physically connected unique endpoint, never more
/// than twice MAX_CABLES. Named disconnected endpoints require no binding.
#[derive(Debug, Default)]
pub(crate) struct Observer {
    pub bindings: Vec<Binding>,
    lookup: BTreeMap<(usize, PortDirection, usize), usize>,
    enabled: bool,
    processed: u64,
    last: Option<u64>,
}
impl Observer {
    pub fn add(&mut self, node: usize, direction: PortDirection, port: usize, source: usize) {
        let key = (node, direction, port);
        if self.lookup.contains_key(&key) {
            return;
        }
        assert!(self.bindings.len() < 2 * crate::MAX_CABLES);
        self.lookup.insert(key, self.bindings.len());
        self.bindings.push(Binding {
            node,
            direction,
            port,
            source,
            state: None,
        });
    }
    pub fn find(&self, node: usize, direction: PortDirection, port: usize) -> Option<usize> {
        self.lookup.get(&(node, direction, port)).copied()
    }
    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled != enabled {
            self.enabled = enabled;
            self.last = None;
            for binding in &mut self.bindings {
                binding.state = None;
            }
        }
    }
    pub fn get(&self, index: Option<usize>) -> Option<PortLightState> {
        if !self.enabled || self.last.is_none() {
            return None;
        }
        match index {
            Some(index) => self.bindings.get(index)?.state,
            None => Some(PortLightState::default()),
        }
    }
    pub fn due(&self) -> bool {
        self.enabled && self.processed.is_multiple_of(7)
    }
    pub fn first_offset(&self) -> Option<usize> {
        self.enabled
            .then_some(((7 - self.processed % 7) % 7) as usize)
    }
    pub fn advance(&mut self, length: usize) {
        if let Some(first) = self.first_offset() {
            if first < length {
                let offset = first + (length - 1 - first) / 7 * 7;
                self.last = Some(self.processed.saturating_add(offset as u64));
            }
        }
        self.processed = self.processed.saturating_add(length as u64);
    }
    pub fn processed(&self) -> u64 {
        self.processed
    }
    pub fn last(&self) -> Option<u64> {
        self.enabled.then_some(self.last).flatten()
    }
    pub fn reset_epoch(&mut self) {
        self.processed = 0;
        self.last = None;
        for binding in &mut self.bindings {
            binding.state = None;
        }
    }
    pub fn adopt_clock(&mut self, old: &Self) {
        self.processed = old.processed;
        self.last = if self.enabled && old.enabled {
            old.last
        } else {
            None
        };
        for binding in &mut self.bindings {
            binding.state = None;
        }
    }
    pub fn can_adopt(&self, old: &Self) -> bool {
        self.enabled && old.enabled
    }
    pub fn clear_node(&mut self, node: usize) {
        for binding in &mut self.bindings {
            if binding.node == node {
                binding.state = None;
            }
        }
    }
}
