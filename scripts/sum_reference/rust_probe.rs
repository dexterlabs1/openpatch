// Compile the production module itself, not a restated implementation.
// The small host adapter deliberately excludes graph/API validation coverage.
use std::io::{self, BufRead};
#[path = __SUM_SOURCE__]
mod sum;

struct Node { level: f32 }
fn parameter(n: &Node, _: &str, _: f32, _: f32, _: f32) -> Result<f32, String> { Ok(n.level) }
fn bounded(v: f32) -> f32 { v.clamp(-1_000_000.0, 1_000_000.0) }
#[derive(Clone, Copy, Default)]
struct PortFrame { channels: u8, values: [f32; 16] }
impl PortFrame {
    fn mono(v: f32) -> Self { let mut f = Self::default(); f.channels = 1; f.values[0] = v; f }
}
#[derive(Default)]
struct ModuleLightState {
    input_channels: Option<u8>, count: u8, brightness: [[f32; 2]; 10], manual: bool,
}
fn main() {
    let rate = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut level = 1.0;
    let mut model = sum::Sum::build(&Node { level }, rate).unwrap();
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let v: Vec<f32> = line.split_whitespace().map(|x| x.parse().unwrap()).collect();
        if level != v[0] {
            level = v[0];
            let mut next = sum::Sum::build(&Node { level }, rate).unwrap();
            next.adopt(&model);
            model = next;
        }
        let mut frame = PortFrame { channels: v[1] as u8, ..PortFrame::default() };
        frame.values.copy_from_slice(&v[2..18]);
        let output = model.process(frame);
        let lights = model.lights();
        assert_eq!(output.channels, 1);
        assert_eq!(lights.count, 6);
        assert!(!lights.manual);
        print!("{:.9e} {}", output.values[0], lights.input_channels.unwrap());
        for b in lights.brightness.iter().take(6) { print!(" {:.9e}", b[0]); }
        println!();
    }
}
