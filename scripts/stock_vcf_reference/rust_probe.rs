// SPDX-License-Identifier: MIT
// Minimal host adapters; DSP is the production source, not a second model.
#![allow(dead_code)]
use std::{collections::BTreeMap,io::{self,BufRead}};
#[path=__STOCK_VCF_SOURCE__] mod stock_vcf;
#[derive(Debug)] struct Node{id:String,kind:String,params:BTreeMap<String,f32>}
const SIGNAL_LIMIT:f32=1_000_000.0;
fn parameter(node:&Node,name:&str,default:f32,min:f32,max:f32)->Result<f32,String>{let v=node.params.get(name).copied().unwrap_or(default);if v.is_finite()&&(min..=max).contains(&v){Ok(v)}else{Err(format!("invalid {name}"))}}
#[derive(Debug,Clone,Copy,Default)] struct PortFrame{channels:u8,values:[f32;16]}
impl PortFrame {fn channel(&self,lane:usize)->f32{if lane<usize::from(self.channels).min(16){self.values[lane]}else{0.0}} fn voice(&self,lane:usize,normal:f32)->f32{match self.channels{0=>normal,1=>self.values[0],_=>self.channel(lane)}}}
fn main(){
    let args:Vec<String>=std::env::args().collect();let rate:u32=args[1].parse().unwrap();
    let names=["frequency","resonance","cutoff_cv_amount","resonance_cv_amount","drive","drive_cv_amount"];
    let mut node=Node{id:"reference".into(),kind:"stock_vcf".into(),params:BTreeMap::new()};
    let mut module=stock_vcf::StockVcf::build(&node,rate).unwrap();
    stock_vcf::reference_seed(&mut module,u64::from_str_radix(args[2].trim_start_matches("0x"),16).unwrap(),u64::from_str_radix(args[3].trim_start_matches("0x"),16).unwrap());
    let initial=stock_vcf::reference_state(&module);
    let mut previous=[f32::NAN;6];let mut draws=0_u64;
    for line in io::stdin().lock().lines(){let line=line.unwrap();let n:Vec<f32>=line.split_whitespace().map(|s|s.parse().unwrap()).collect();assert_eq!(n.len(),75);
        if previous!=n[1..7]{node.params=names.iter().zip(&n[1..7]).map(|(n,v)|(n.to_string(),*v)).collect();let mut next=stock_vcf::StockVcf::build(&node,rate).unwrap();next.adopt(&module);module=next;previous.copy_from_slice(&n[1..7]);}
        let mut input=[PortFrame::default();4];for(i,p)in input.iter_mut().enumerate(){p.channels=n[7+i*17]as u8;p.values.copy_from_slice(&n[8+i*17..24+i*17]);}
        let mut output=[PortFrame::default();2];let mask=n[0]as u16;module.process(&input,&mut output,mask);
        let width=usize::from(input[3].channels).max(1);if mask!=0{draws+=width.div_ceil(4)as u64;}
        assert_eq!(output[0].channels,output[1].channels);print!("{}",output[0].channels);
        for p in output{assert!(p.values[usize::from(p.channels)..].iter().all(|v|*v==0.0));for v in p.values{assert!(v.is_finite());print!(" {:.9e}",v);}}println!();
    }
    let state=stock_vcf::reference_state(&module);eprintln!("{} {} {} {} {}",initial[0],initial[1],state[0],state[1],draws);
}
