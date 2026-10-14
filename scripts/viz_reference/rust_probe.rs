// Compile production DSP directly with minimal host adapters.
use std::io::{self, BufRead};
#[path = __VIZ_SOURCE__]
mod viz;
const MAX_MODULE_LIGHTS: usize = 16;
#[derive(Clone, Copy, Default)]
struct PortFrame { channels: u8, values: [f32; 16] }
impl PortFrame {
    fn channel(&self, index: usize) -> f32 {
        if index < self.channels as usize { self.values[index] } else { 0.0 }
    }
}
struct ModuleLightState {
    input_channels: Option<u8>, count: u8, brightness: [[f32; 2]; MAX_MODULE_LIGHTS], manual: bool,
}
fn main() {
    let rate = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut model = viz::Viz::new(rate);
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let values: Vec<f32> = line.split_whitespace().map(|v| v.parse().unwrap()).collect();
        if values[0] != 0.0 {
            let mut next = viz::Viz::new(rate);
            next.adopt(&model);
            model = next;
        }
        let mut frame = PortFrame { channels: values[1] as u8, ..PortFrame::default() };
        frame.values.copy_from_slice(&values[2..18]);
        let _: () = model.process(frame); // Input-only DSP has no output frame.
        let lights = model.lights();
        assert_eq!(lights.count, 16);
        assert!(!lights.manual);
        print!("{}", lights.input_channels.unwrap());
        for pair in lights.brightness { for value in pair { print!(" {:.9e}", value); } }
        println!();
    }
}
