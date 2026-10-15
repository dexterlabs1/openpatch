// SPDX-License-Identifier: MIT
// Minimal host adapters. All DSP is included from the production source.
#![allow(dead_code)]
use std::{collections::BTreeMap,io::{self,Read,Write}};
#[path=__STOCK_DELAY_SOURCE__] mod stock_delay;
#[derive(Debug)] struct Node{id:String,kind:String,params:BTreeMap<String,f32>}
const SIGNAL_LIMIT:f32=1_000_000.0;
fn parameter(node:&Node,name:&str,default:f32,min:f32,max:f32)->Result<f32,String>{let v=node.params.get(name).copied().unwrap_or(default);if v.is_finite()&&(min..=max).contains(&v){Ok(v)}else{Err(format!("invalid {name}"))}}
#[derive(Debug,Clone,Copy,Default)] struct PortFrame{channels:u8,values:[f32;16]}
impl PortFrame {
    fn channel(&self,lane:usize)->f32{if lane<usize::from(self.channels).min(16){self.values[lane]}else{0.0}}
    fn voice(&self,lane:usize,normal:f32)->f32{match self.channels{0=>normal,1=>self.values[0],_=>self.channel(lane)}}
}
fn read_u32(input:&mut impl Read)->Option<u32>{let mut bytes=[0;4];match input.read(&mut bytes[..1]){Ok(0)=>None,Ok(1)=>{input.read_exact(&mut bytes[1..]).unwrap();Some(u32::from_le_bytes(bytes))},_=>panic!("input read failure")}}
fn read_f32(input:&mut impl Read)->f32{f32::from_bits(read_u32(input).expect("truncated f32"))}
fn main(){
    let rate:u32=std::env::args().nth(1).unwrap().parse().unwrap();
    let names=["time","feedback","tone","mix","time_cv_amount","feedback_cv_amount","tone_cv_amount","mix_cv_amount"];
    let mut node=Node{id:"reference".into(),kind:"stock_delay".into(),params:BTreeMap::new()};
    let mut module=stock_delay::StockDelay::build(&node,rate).unwrap();
    let mut previous=[f32::NAN;8];let mut input=io::BufReader::new(io::stdin().lock());let mut output=io::BufWriter::new(io::stdout().lock());
    while let Some(control)=read_u32(&mut input){
        assert!(control<=1);let mut params=[0.;8];for value in &mut params{*value=read_f32(&mut input);}
        if previous!=params||control==1{
            node.params=names.iter().zip(params).map(|(name,value)|(name.to_string(),value)).collect();
            if control==1{let mut next=stock_delay::StockDelay::build(&node,rate).unwrap();next.adopt(&mut module);module=next;}else{module.set_parameters(&node).unwrap();}
            previous=params;
        }
        let mut ports=[PortFrame::default();6];
        for port in &mut ports{port.channels=read_u32(&mut input).expect("truncated width")as u8;assert!(port.channels<=16);for lane in 0..usize::from(port.channels){port.values[lane]=read_f32(&mut input);}}
        let result=module.process(&ports);
        for port in &result{assert_eq!(port.channels,1);assert!(port.values.iter().all(|value|value.is_finite()));assert!(port.values[1..].iter().all(|value|*value==0.));output.write_all(&u32::from(port.channels).to_le_bytes()).unwrap();}
        for port in &result{output.write_all(&port.values[0].to_le_bytes()).unwrap();}
    }
}
