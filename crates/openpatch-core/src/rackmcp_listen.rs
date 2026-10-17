//! RackMCP 2.0.0 Listen audio processor, adapted from Zachary Scheffler's
//! ListenModule.{hpp,cpp}, GPL-3.0-or-later, revision
//! 285e84c5ff2d3ecef25890da07f578af6a0c5ca6. See LICENSE-THIRD-PARTY.
//! The host owns capture transport. Processing only borrows its selected sink.
use crate::host_ports::HostPortSnapshot;
use crate::native_float::rack_add;
use crate::{ModuleLightState, Node};
use serde_json::{json, Value};

pub(crate) const INPUTS: [&str; 2] = ["left", "right"];
pub(crate) const OUTPUTS: [&str; 2] = ["left", "right"];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ListenFrame {
    pub left: f32,
    pub right: f32,
}

/// Borrowed realtime producer. Implementations must never allocate, block,
/// perform I/O or free storage in either method. A full sink rejects the new frame.
pub trait ListenSink {
    fn armed(&self) -> bool;
    fn push(&mut self, frame: ListenFrame) -> bool;
    /// Publish exact source-cadence hints after the current sample push.
    fn observe_status(&mut self, _status: ListenStatus) {}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct ListenStatus {
    pub frames_seen: u64,
    pub dropped: u32,
    pub left_connected: bool,
    pub right_connected: bool,
    /// Actual published lamp, updated every 256 normal processed samples.
    pub capturing: bool,
}

pub(crate) fn catalog() -> Value {
    json!({"kind":"rackmcp_listen","title":"RACKMCP LISTEN","model":"source-version",
        "description":"RackMCP 2.0.0 master-bus listen tap. Independent raw polyphonic left/right pass-through; selected live capture receives their voltage sums.",
        "inputs":INPUTS,"outputs":OUTPUTS,"params":{},"hp":6,"tags":["Utility","Visual"],
        "source":{"plugin":"RackMCP","version":"2.0.0","revision":"285e84c5ff2d3ecef25890da07f578af6a0c5ca6","license":"GPL-3.0-or-later"}})
}

pub(crate) fn annotate(module: &mut Value) {
    module["polyphony"] = json!({"version_1":"unsupported","version_2":"independent raw L/R channel counts and active voltages; no gain, normaling or downmix on outputs","maximum_channels":16,"primary_input":null,"inactive_storage":"SDK setChannels before active writes; disconnected outputs retain raw writes with zero channels"});
    module["input_names"] = json!(["Left (bus in)", "Right (bus in)"]);
    module["output_names"] = json!(["Left (to Audio)", "Right (to Audio)"]);
    module["light_names"] = json!(["Capturing"]);
    module["light_policy"] = json!({"colors":["red"],"publication_frames":256,"source":"actual armed capture sink; connection hints and lamp latch together","bypass":"counter, divider, hints, lamp and capture freeze"});
    module["state_adoption"] = json!("same identity/kind preserves counters, divider phase, connection hints and lamp across graph and sample-rate changes; excluded/new incarnations remain fresh; transport belongs to the host");
    module["capture_policy"] = json!("unbypassed framesSeen increments while unarmed and silent; armed capture sums active lanes in ascending source float32 order, absent R is zero; rejecting a frame increments wrapping u32 dropped; host owns preallocated bounded transport");
}

#[derive(Debug, Default)]
pub(crate) struct RackMcpListen {
    status: ListenStatus,
    light_divider: u16,
}
impl RackMcpListen {
    pub(crate) fn build(node: &Node) -> Result<Self, String> {
        if let Some(key) = node.params.keys().next() {
            return Err(format!("node '{}': unknown parameter '{key}'", node.id));
        }
        Ok(Self::default())
    }
    pub(crate) fn status(&self) -> ListenStatus {
        self.status
    }
    pub(crate) fn lights(&self) -> ModuleLightState {
        let mut lights = ModuleLightState {
            count: 1,
            ..ModuleLightState::default()
        };
        lights.brightness[0][0] = if self.status.capturing { 1.0 } else { 0.0 };
        lights
    }
    pub(crate) fn adopt(&mut self, previous: &Self) {
        self.status = previous.status;
        self.light_divider = previous.light_divider;
    }
    pub(crate) fn process(
        &mut self,
        inputs: &[HostPortSnapshot; 16],
        outputs: &mut [HostPortSnapshot],
        mut sink: Option<&mut dyn ListenSink>,
    ) {
        // Listen sets channels before writing, unlike Module::processBypass.
        for port in 0..2 {
            let count = inputs[port].channels;
            outputs[port].set_channels(count);
            for lane in 0..usize::from(count) {
                outputs[port].values[lane] = inputs[port].values[lane];
            }
        }
        self.status.frames_seen = self.status.frames_seen.wrapping_add(1);
        let armed = sink.as_ref().is_some_and(|sink| sink.armed());
        if armed {
            let sum = |port: &HostPortSnapshot| {
                port.values[..usize::from(port.channels)]
                    .iter()
                    .copied()
                    .fold(0.0, rack_add)
            };
            let frame = ListenFrame {
                left: sum(&inputs[0]),
                right: if inputs[1].channels > 0 {
                    sum(&inputs[1])
                } else {
                    0.0
                },
            };
            if !sink.as_mut().expect("armed sink exists").push(frame) {
                self.status.dropped = self.status.dropped.wrapping_add(1);
            }
        }
        self.light_divider += 1;
        if self.light_divider == 256 {
            self.light_divider = 0;
            self.status.left_connected = inputs[0].channels > 0;
            self.status.right_connected = inputs[1].channels > 0;
            self.status.capturing = armed;
            if let Some(sink) = sink {
                sink.observe_status(self.status);
            }
        }
    }
    pub(crate) fn process_bypass(
        &self,
        inputs: &[HostPortSnapshot; 16],
        outputs: &mut [HostPortSnapshot],
    ) {
        for port in 0..2 {
            let count = inputs[port].channels;
            for lane in 0..usize::from(count) {
                outputs[port].values[lane] = inputs[port].values[lane];
            }
            outputs[port].set_channels(count);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Reject;
    impl ListenSink for Reject {
        fn armed(&self) -> bool {
            true
        }
        fn push(&mut self, _: ListenFrame) -> bool {
            false
        }
    }
    #[test]
    fn source_unsigned_counters_wrap_and_unarmed_processing_still_advances_frames() {
        let mut model = RackMcpListen::default();
        model.status.frames_seen = u64::MAX;
        model.status.dropped = u32::MAX;
        let inputs = [HostPortSnapshot::default(); 16];
        let mut outputs = [HostPortSnapshot::default(); 2];
        model.process(&inputs, &mut outputs, Some(&mut Reject));
        assert_eq!(model.status.frames_seen, 0);
        assert_eq!(model.status.dropped, 0);
        model.process(&inputs, &mut outputs, None);
        assert_eq!(model.status.frames_seen, 1);
        assert_eq!(model.status.dropped, 0);
    }
    #[test]
    fn raw_output_storage_follows_set_channels_before_write_and_bypass_after_write() {
        let mut model = RackMcpListen::default();
        let mut inputs = [HostPortSnapshot::default(); 16];
        inputs[0].channels = 2;
        inputs[0].values[0] = -0.0;
        inputs[0].values[1] = 3.0;
        let mut outputs = [HostPortSnapshot {
            channels: 4,
            values: [9.0; 16],
        }; 2];
        outputs[1].channels = 0;
        inputs[1].channels = 1;
        inputs[1].values[0] = -7.0;
        model.process(&inputs, &mut outputs, None);
        assert_eq!(outputs[0].channels, 2);
        assert_eq!(outputs[0].values[0].to_bits(), (-0.0f32).to_bits());
        assert_eq!(&outputs[0].values[1..5], &[3.0, 0.0, 0.0, 9.0]);
        assert_eq!(outputs[1].channels, 0);
        assert_eq!(&outputs[1].values[..2], &[-7.0, 9.0]);
        let frozen = model.status;
        inputs[0].channels = 0;
        model.process_bypass(&inputs, &mut outputs);
        assert_eq!(outputs[0].channels, 1);
        assert_eq!(&outputs[0].values[..3], &[0.0, 0.0, 0.0]);
        assert_eq!(model.status, frozen);
    }
}
