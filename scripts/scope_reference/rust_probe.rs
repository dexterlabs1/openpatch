#![allow(dead_code)]
use std::{collections::BTreeMap, io::{self, BufRead}};
#[path = __SCOPE_SOURCE__]
mod scope;
struct Node { id: String, params: BTreeMap<String, f32> }
fn parameter(node: &Node, name: &str, default: f32, min: f32, max: f32) -> Result<f32, String> {
    let v = node.params.get(name).copied().unwrap_or(default);
    if v.is_finite() && v >= min && v <= max { Ok(v) } else { Err(name.into()) }
}
#[derive(Clone, Copy, Default)]
struct PortFrame { channels: u8, values: [f32; 16] }
fn hash_float(hash: &mut u64, value: f32) { *hash = (*hash ^ u64::from(value.to_bits())).wrapping_mul(1099511628211); }
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let width: u32 = args[1].parse().unwrap();
    let mut node = Node { id: "probe".into(), params: BTreeMap::from([
        ("time".into(), width as f32 / 256.0), ("trigger_enabled".into(), args[2].parse().unwrap()),
        ("xy".into(), args[3].parse().unwrap()), ("threshold".into(), 2.0)]) };
    let mut model = scope::Scope::build(&node, 65536).unwrap();
    assert_eq!(model.snapshot.samples_per_bin, width);
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let values: Vec<f32> = line.split_whitespace().map(|s| s.parse().unwrap()).collect();
        if values[0] != 0.0 {
            node.params.insert("x_scale".into(), 7.0);
            node.params.insert("x_offset".into(), -9.0);
            let mut next = scope::Scope::build(&node, 65536).unwrap();
            next.adopt(&model, true); model = next;
        }
        let mut inputs = [PortFrame::default(); 3];
        for (axis, input) in inputs.iter_mut().enumerate() {
            input.channels = values[1 + axis * 17] as u8;
            for lane in 0..usize::from(input.channels) { input.values[lane] = values[2 + axis * 17 + lane]; }
        }
        let mut outputs = [PortFrame::default(); 2];
        model.process(&inputs, &mut outputs);
        let s = &model.snapshot;
        let mut hash = 14695981039346656037u64;
        for axis in [&s.x, &s.y] { for lane in axis { for point in lane { hash_float(&mut hash, point.min); hash_float(&mut hash, point.max); } } }
        for output in outputs { for value in output.values { hash_float(&mut hash, value); } }
        println!("{} {} {} {} {}", s.buffer_index, s.sample_index, outputs[0].channels, outputs[1].channels, hash);
    }
}
