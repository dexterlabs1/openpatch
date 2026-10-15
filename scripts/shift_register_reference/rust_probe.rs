// SPDX-License-Identifier: MIT
// Compile production DSP itself; these adapters supply host types, not DSP.
#![allow(dead_code)]
use std::{collections::BTreeMap, io::{self, BufRead}};
#[path = __SHIFT_REGISTER_SOURCE__]
mod shift_register;
#[derive(Debug)]
struct Node { id: String, kind: String, params: BTreeMap<String, f32> }
fn parameter(node: &Node, name: &str, default: f32, min: f32, max: f32) -> Result<f32, String> {
    let v = node.params.get(name).copied().unwrap_or(default);
    if v.is_finite() && (min..=max).contains(&v) { Ok(v) } else { Err(format!("invalid {name}")) }
}
#[derive(Debug, Clone, Copy)]
enum ModuleAction { PushPress, PushRelease, ResetPress, ResetRelease, Clock }
const MAX_MODULE_ACTIONS_PER_SAMPLE: usize = 64;
#[derive(Debug, Clone, Copy, Default)]
struct PortFrame { channels: u8, values: [f32;16] }
impl PortFrame {
    fn mono(v: f32) -> Self { let mut f=Self::default(); f.channels=1; f.values[0]=v; f }
    fn channel(&self, channel: usize) -> f32 { if channel < self.channels as usize {self.values[channel]} else {0.0} }
}
#[derive(Debug, Default)]
struct ModuleControlState { pressed: bool, pressed_mask: u16 }
#[derive(Debug, Default)]
struct ModuleLightState { manual: bool }
#[derive(Debug)]
enum SavedModuleState { ShiftRegister {values:[u32;8],rng:u32}, Other }
fn main() {
    let node=Node{id:"reference".into(),kind:"sh_asr".into(),params:BTreeMap::new()};
    let mut module=shift_register::ShiftRegister::build(&node).unwrap();
    let (mut prior_push,mut prior_clear)=(false,false);
    // Match C++'s explicit low button arming frame. It changes no held values.
    let mut output=[PortFrame::default();8];
    module.process(&[PortFrame::default();16],&mut output);
    for line in io::stdin().lock().lines() {
        let line=line.unwrap();
        let numbers:Vec<f32>=line.split_whitespace().map(|s|s.parse().unwrap()).collect();
        assert_eq!(numbers.len(),274);
        let (push,clear)=(numbers[0]!=0.0,numbers[1]!=0.0);
        if push!=prior_push {assert!(module.enqueue(if push {ModuleAction::PushPress}else{ModuleAction::PushRelease}));}
        if clear!=prior_clear {assert!(module.enqueue(if clear {ModuleAction::ResetPress}else{ModuleAction::ResetRelease}));}
        (prior_push,prior_clear)=(push,clear);
        let mut inputs=[PortFrame::default();16];
        for (i,input) in inputs.iter_mut().enumerate() {
            input.channels=numbers[2+i*17] as u8;
            input.values.copy_from_slice(&numbers[3+i*17..19+i*17]);
        }
        module.process(&inputs,&mut output);
        for (i,value) in output.iter().enumerate() {
            assert_eq!(value.channels,1);
            assert!(value.values[1..].iter().all(|v|*v==0.0));
            if i>0 {print!(" ");}
            print!("{:.9e}",value.values[0]);
        }
        println!();
    }
}
