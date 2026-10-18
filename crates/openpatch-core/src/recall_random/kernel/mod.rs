//! Source-derived Marbles kernels. Each port retains its own upstream notice.
pub mod math;
pub mod resources;
pub mod random;
pub mod ramp;
pub mod t;
pub mod quantizer;
pub mod lag;
pub mod output;
pub mod xy;
pub const BLOCK: usize = 5;
#[derive(Clone, Copy, Debug)]
pub struct Ratio { pub p: i32, pub q: i32 }
#[derive(Clone, Debug, Default)]
pub struct Ramps { pub external: [f32; BLOCK], pub master: [f32; BLOCK], pub slave: [[f32; BLOCK]; 2] }
pub mod record;
