// SPDX-License-Identifier: MIT
// These minimal adapters supply host data types, never envelope mathematics.
#![allow(dead_code)]
use std::{collections::BTreeMap, io::{self, BufRead}};
#[path = __ADSR_EG_SOURCE__]
mod adsr_eg;
#[derive(Debug)]
struct Node { id: String, kind: String, params: BTreeMap<String, f32> }
fn parameter(node: &Node, name: &str, default: f32, min: f32, max: f32) -> Result<f32, String> {
    let v = node.params.get(name).copied().unwrap_or(default);
    if v.is_finite() && (min..=max).contains(&v) { Ok(v) } else { Err(format!("invalid {name}")) }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleAction { PushPress, PushRelease, Clock }
const MAX_MODULE_ACTIONS_PER_SAMPLE: usize = 64;
const MAX_MODULE_LIGHTS: usize = 16;
const MAX_CHANNELS: usize = 16;
const SIGNAL_LIMIT: f32 = 1_000_000.0;
#[derive(Debug, Clone, Copy, Default)]
struct PortFrame { channels: u8, values: [f32;16] }
impl PortFrame {
    fn mono(v: f32) -> Self { let mut f=Self::default(); f.channels=1; f.values[0]=v; f }
    fn channel(&self, lane: usize) -> f32 { if lane < usize::from(self.channels).min(16) {self.values[lane]} else {0.0} }
    fn voice(&self, lane: usize, normal: f32) -> f32 { match self.channels {0=>normal,1=>self.values[0],_=>self.channel(lane)} }
}
#[derive(Debug, Default)]
struct ModuleControlState { pressed: bool, pressed_mask: u16 }
#[derive(Debug, Default, Clone, Copy)]
struct ModuleLightState { input_channels: Option<u8>, count: u8, brightness: [[f32;2];16], manual: bool }
fn main() {
    let rate: u32=std::env::args().nth(1).unwrap().parse().unwrap();
    let names=["attack","decay","sustain","release","attack_amount","decay_amount","sustain_amount","release_amount"];
    let mut node=Node{id:"reference".into(),kind:"adsr_eg".into(),params:BTreeMap::new()};
    let mut module=adsr_eg::AdsrEg::build(&node,rate).unwrap();
    let mut previous=[f32::NAN;8];
    let mut prior_push=false;
    let mut output=[PortFrame::default();1];
    // No arming frame, no warm-up: first coefficient tick belongs to the trace.
    for line in io::stdin().lock().lines() {
        let line=line.unwrap();
        let numbers:Vec<f32>=line.split_whitespace().map(|s|s.parse().unwrap()).collect();
        assert_eq!(numbers.len(),111);
        if previous != numbers[1..9] {
            node.params=names.iter().zip(&numbers[1..9]).map(|(n,v)|(n.to_string(),*v)).collect();
            let mut next=adsr_eg::AdsrEg::build(&node,rate).unwrap();
            next.adopt(&module);
            module=next;
            previous.copy_from_slice(&numbers[1..9]);
        }
        let push=numbers[0]!=0.0;
        if push!=prior_push {assert!(module.enqueue(if push {ModuleAction::PushPress}else{ModuleAction::PushRelease}));}
        prior_push=push;
        let mut inputs=[PortFrame::default();6];
        for (i,input) in inputs.iter_mut().enumerate() {
            input.channels=numbers[9+i*17] as u8;
            input.values.copy_from_slice(&numbers[10+i*17..26+i*17]);
        }
        module.process(&inputs,&mut output);
        assert!((1..=16).contains(&output[0].channels));
        assert!(output[0].values[usize::from(output[0].channels)..].iter().all(|v|*v==0.0));
        print!("{}",output[0].channels);
        for value in output[0].values { print!(" {:.9e}",value); }
        let lights=module.lights();
        assert_eq!(lights.count,5);
        for pair in &lights.brightness[..5] { assert_eq!(pair[1],0.0); print!(" {:.9e}",pair[0]); }
        println!();
    }
}
