#![allow(dead_code)]
#[path = "../../crates/openpatch-core/src/rhythm_terrain/resources.rs"]
mod resources;
#[path = "../../crates/openpatch-core/src/rhythm_terrain/kernel.rs"]
mod kernel;
use kernel::{Clock, Options, PatternGenerator, Settings};
use std::io::{self, BufRead};
fn main() {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let seed: u32 = lines.next().unwrap().unwrap().parse().unwrap();
    let mut p = PatternGenerator::new(seed);
    let mut c = Clock::new();
    let mut index = 0usize;
    let mut previous_inputs = 0u8;
    let mut tap_duration = 0u32;
    let mut swing_amount = 0i8;
    for line in lines {
        let line = line.unwrap();
        if line.is_empty() { continue; }
        let values: Vec<u32> = line.split_whitespace().map(|v| v.parse().unwrap()).collect();
        let op = values[0]; let repeat = values[1];
        let mut a = [0u32; 7];
        for (i,v) in values[2..].iter().enumerate() { a[i] = *v; }
        for _ in 0..repeat {
            match op {
                0 => { p.options = Options::unpack(a[0] as u8); p.settings = Settings {
                    options: [a[1] as u8,a[2] as u8,a[3] as u8],
                    density: [a[4] as u8,a[5] as u8,a[6] as u8] }; }
                1 => p.tick_clock(a[0] as u8), 2 => p.retrigger(),
                3 => p.increment_pulse_counter(), 4 => p.clock_falling_edge(), 5 => p.reset(),
                10 => c.update(a[0] as u16,a[1] as u8), 11 => c.tick(),
                12 => c.wrap(a[0] as u8 as i8), 13 => c.reset(), 14 => c.lock(), 15 => c.unlock(),
                20 => { for part in 0..3 { for step in 0..32 {
                    println!("[\"m\",{step},{part},{},{},{}]",a[0],a[1],PatternGenerator::read_drum_map(step,part,a[0] as u8,a[1] as u8));
                } } }
                40 => {
                    tap_duration = tap_duration.wrapping_add(1);
                    let inputs = (if a[0] != 0 { 2 } else { 0 }) | (if a[1] != 0 { 4 } else { 0 });
                    let increment = [6,3,1][usize::from(p.clock_resolution())];
                    let mut num_ticks = 0;
                    if c.bpm() < 40 && !c.locked() {
                        if inputs & 2 != 0 && previous_inputs & 2 == 0 { num_ticks = increment; }
                        if inputs & 2 == 0 && previous_inputs & 2 != 0 { p.clock_falling_edge(); }
                    } else {
                        c.tick(); c.wrap(swing_amount);
                        if c.raising_edge() { num_ticks = increment; }
                        if c.past_falling_edge() { p.clock_falling_edge(); }
                    }
                    if inputs & 4 != 0 && previous_inputs & 4 == 0 {
                        p.reset();
                        if c.bpm() >= 40 || c.locked() { p.retrigger(); c.reset(); }
                    }
                    previous_inputs = inputs;
                    if num_ticks != 0 { swing_amount = p.swing_amount(); p.tick_clock(num_ticks); }
                    p.increment_pulse_counter();
                }
                30 => {
                    for (i,v) in resources::EUCLIDEAN.iter().enumerate() { println!("[\"e\",{i},{v}]"); }
                    for (i,v) in resources::TEMPO_PHASE_INCREMENT.iter().enumerate() { println!("[\"t\",{i},{v}]"); }
                    for (i,node) in resources::NODES.iter().enumerate() { for (j,v) in node.iter().enumerate() { println!("[\"n\",{i},{j},{v}]"); } }
                }
                _ => panic!("unknown operation"),
            }
            let s = p.snapshot(); let k = c.snapshot();
            println!("[{index},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}]",
                s.state,s.pulse,s.step,s.euclidean_step[0],s.euclidean_step[1],s.euclidean_step[2],
                s.part_perturbation[0],s.part_perturbation[1],s.part_perturbation[2],s.pulse_duration_counter,
                u8::from(s.first_beat),u8::from(s.beat),s.rng_state,s.swing_amount,p.options.pack(),
                u8::from(k.locked),k.bpm,k.phase,k.phase_increment,k.falling_edge,u8::from(k.raising_edge),u8::from(k.past_falling_edge));
            if op == 40 { println!("[\"s\",{index},{tap_duration},{swing_amount},{}]", previous_inputs); }
            index += 1;
        }
    }
}
