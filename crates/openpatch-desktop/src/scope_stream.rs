//! Fixed storage for two raw signal taps. No decimation, normalization, gain or
//! crossfade is applied. Missing voices are distinct from active zero volts.
use openpatch_core::TapSample;
use rtrb::{Consumer, Producer, RingBuffer};

pub const SCOPE_PACKET_SIZE: usize = 64;
pub const SCOPE_QUEUE_SIZE: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeEndpoint {
    pub id: String,
    pub port: String,
    /// Zero-based direct voice index; mono does not broadcast to higher lanes.
    pub channel: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScopeSelection {
    pub x: Option<ScopeEndpoint>,
    pub y: Option<ScopeEndpoint>,
}
#[derive(Clone, Copy, Debug)]
pub struct ScopePacket {
    pub start_frame: u64,
    pub generation: u64,
    pub sample_rate: u32,
    pub x: [f32; SCOPE_PACKET_SIZE],
    pub y: [f32; SCOPE_PACKET_SIZE],
    pub x_active: [bool; SCOPE_PACKET_SIZE],
    pub y_active: [bool; SCOPE_PACKET_SIZE],
}
impl ScopePacket {
    fn new(sample_rate: u32) -> Self {
        Self {
            start_frame: 0,
            generation: 0,
            sample_rate,
            x: [0.0; SCOPE_PACKET_SIZE],
            y: [0.0; SCOPE_PACKET_SIZE],
            x_active: [false; SCOPE_PACKET_SIZE],
            y_active: [false; SCOPE_PACKET_SIZE],
        }
    }
}

pub(super) struct ScopeCapture {
    output: Producer<ScopePacket>,
    packet: ScopePacket,
    count: usize,
}
impl ScopeCapture {
    pub(super) fn new(rate: u32) -> (Self, Consumer<ScopePacket>) {
        let (output, input) = RingBuffer::new(SCOPE_QUEUE_SIZE);
        (
            Self {
                output,
                packet: ScopePacket::new(rate),
                count: 0,
            },
            input,
        )
    }
    /// Discard an incomplete old-generation packet rather than mislabel samples.
    pub(super) fn reset(&mut self, generation: u64) {
        self.packet.generation = generation;
        self.count = 0;
    }
    pub(super) fn sample(&mut self, frame: u64, values: [TapSample; 2]) {
        if self.count == 0 {
            self.packet.start_frame = frame;
        }
        self.packet.x[self.count] = values[0].value;
        self.packet.x_active[self.count] = values[0].active;
        self.packet.y[self.count] = values[1].value;
        self.packet.y_active[self.count] = values[1].active;
        self.count += 1;
        if self.count == SCOPE_PACKET_SIZE {
            // Full packets contain only Copy data. A full queue drops this
            // packet without heap teardown; subsequent timestamps retain gaps.
            let _ = self.output.push(self.packet);
            self.count = 0;
        }
    }
}
