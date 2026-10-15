// SPDX-License-Identifier: MIT
// Adapter includes the production detector and observation clock unchanged.
#![allow(dead_code)]
use std::io::{self, BufRead, Write};
pub const MAX_CHANNELS: usize = 16;
pub const MAX_CABLES: usize = 1024;
#[path = __PORT_LIGHT_SOURCE__]
mod port_lights;
use port_lights::{Observer, PortDirection, PortLightState, step_port_light};
fn main() {
    let rate: u32 = std::env::args().nth(1).unwrap().parse().unwrap();
    let mut observer = Observer::default();
    observer.add(0, PortDirection::Input, 0, 0);
    observer.add(0, PortDirection::Output, 0, 0);
    observer.set_enabled(true);
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut words = line.split_whitespace();
        let _bypass: u8 = words.next().unwrap().parse().unwrap();
        let mut widths = [0_u8; 2];
        for endpoint in 0..2 {
            widths[endpoint] = words.next().unwrap().parse().unwrap();
            let mut values = [0_f32; 16];
            for value in &mut values { *value = words.next().unwrap().parse().unwrap(); }
            if observer.due() {
                let old = observer.bindings[endpoint].state.unwrap_or_default();
                let state = step_port_light(old, widths[endpoint] != 0, widths[endpoint], &values, rate);
                assert!(!state.invalid);
                observer.bindings[endpoint].state = Some(state);
            }
        }
        assert!(words.next().is_none());
        write!(out, "{}", observer.processed()).unwrap();
        observer.advance(1);
        for endpoint in 0..2 {
            let state: PortLightState = observer.get(Some(endpoint)).expect("frame-zero acquisition");
            write!(out, " {}", widths[endpoint]).unwrap();
            for value in state.raw_rgb { write!(out, " {:.9e}", value).unwrap(); }
        }
        writeln!(out).unwrap();
    }
}
