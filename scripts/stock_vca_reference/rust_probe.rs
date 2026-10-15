// SPDX-License-Identifier: MIT
// Original minimal adapters; all DSP and meter acquisition are production code.
#![allow(dead_code)]
use std::{collections::BTreeMap, io::{self, BufRead}};
#[path = __STOCK_VCA_SOURCE__] mod stock_vca;
#[derive(Debug)] struct Node {id:String,kind:String,params:BTreeMap<String,f32>}
const SIGNAL_LIMIT:f32=1_000_000.0;
fn parameter(node:&Node,name:&str,default:f32,min:f32,max:f32)->Result<f32,String>{let v=node.params.get(name).copied().unwrap_or(default);if v.is_finite()&&(min..=max).contains(&v){Ok(v)}else{Err(format!("invalid {name}"))}}
#[derive(Debug,Clone,Copy,Default)] struct PortFrame {channels:u8,values:[f32;16]}
impl PortFrame {fn channel(&self,lane:usize)->f32{if lane<usize::from(self.channels).min(16){self.values[lane]}else{0.0}} fn voice(&self,lane:usize,normal:f32)->f32{match self.channels{0=>normal,1=>self.values[0],_=>self.channel(lane)}}}
#[derive(Debug,Clone,Copy,Default,PartialEq)] struct ModuleLightState{input_channels:Option<u8>,count:u8,brightness:[[f32;2];16],manual:bool}
fn main(){
    let rate=std::env::args().nth(1).unwrap().parse().unwrap();
    let mut node=Node{id:"reference".into(),kind:"stock_vca".into(),params:BTreeMap::new()};
    let mut module=stock_vca::StockVca::build(&node,rate).unwrap();
    let fresh=module.lights();assert_eq!(fresh.input_channels,Some(1));assert_eq!(fresh.brightness,[[0.0;2];16]);
    let mut previous=[f32::NAN;2];
    for line in io::stdin().lock().lines(){
        let n:Vec<f32>=line.unwrap().split_whitespace().map(|s|s.parse().unwrap()).collect();assert_eq!(n.len(),37);
        if previous!=n[1..3]{node.params=[("level".into(),n[1]),("response".into(),n[2])].into_iter().collect();let mut next=stock_vca::StockVca::build(&node,rate).unwrap();next.adopt(&module);module=next;previous.copy_from_slice(&n[1..3]);}
        let mut inputs=[PortFrame::default();2];for(i,p)in inputs.iter_mut().enumerate(){p.channels=n[3+i*17]as u8;p.values.copy_from_slice(&n[4+i*17..20+i*17]);}
        let out=module.process(&inputs);let lights=module.lights();assert_eq!(lights.count,16);assert_eq!(Some(out.channels),lights.input_channels);assert!(!lights.manual);
        assert!(out.values[out.channels as usize..].iter().all(|v|*v==0.0));
        assert!(lights.brightness[out.channels as usize..].iter().all(|v|*v==[0.0;2]));
        print!("{} {}",out.channels,lights.input_channels.unwrap());
        for v in out.values{assert!(v.is_finite());print!(" {:.9e}",v);}
        for gain in lights.brightness{assert!(gain[0].is_finite());assert_eq!(gain[1],0.0);print!(" {:.9e}",gain[0]);}println!();
    }
}
