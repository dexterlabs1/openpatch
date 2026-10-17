// SPDX-License-Identifier: MIT
// Public Patch -> Engine integration only; no Player or adapter internals.
extern crate openpatch_core;
extern crate serde_json;
use openpatch_core::{Engine,Patch,PortFrame,ResolvedTap,TapSample,runtime_parameter_descriptor};
use serde_json::{json,Value};
use std::{env,fs};
const PORTS:[&str;18]=["pitch1","pitch2","pitch3","pitch4","gate1","gate2","gate3","gate4","velocity1","velocity2","velocity3","velocity4","accent1","accent2","accent3","accent4","clock","reset"];
const TARGETS:[(&str,&str);2]=[("gain","level"),("filter","frequency")];
fn descriptors()->Value {
    Value::Array([("stock_vca","level"),("rack264_vcf","frequency")].map(|(kind,name)|{
        let d=runtime_parameter_descriptor(kind,name).unwrap();
        json!({"kind":kind,"name":name,"min_bits":d.min.to_bits(),"max_bits":d.max.to_bits(),"default_bits":d.default.to_bits(),"snap":d.snap})
    }).to_vec())
}
fn patch(f:&Value,version:usize,running:u8,button:u8,ext:bool,live:bool)->Patch {
    let mut nodes=vec![json!({"id":"clock_source","kind":"poly_constant"}),json!({"id":"reset_source","kind":"poly_constant"}),json!({"id":"seq","kind":"rackmcp_sequencer","params":{"bpm":300,"swing":63.25,"running":running,"reset":button},"rackmcp_sequence":{"data":f["versions"][version],"targets":[{"source_module_id":"42","source_param_id":1,"module":"filter","parameter":"frequency"},{"source_module_id":"42","source_param_id":0,"module":"gain","parameter":"level"}]}})];
    if live {nodes.extend([json!({"id":"gain","kind":"stock_vca","params":{"level":1}}),json!({"id":"filter","kind":"rack264_vcf","params":{"frequency":0.5}})]);}
    let mut cables=vec![json!({"id":"reset_route","from":"reset_source","output":"out","to":"seq","input":"reset"})];
    if ext {cables.push(json!({"id":"clock_route","from":"clock_source","output":"out","to":"seq","input":"clock"}));}
    serde_json::from_value(json!({"version":2,"sample_rate":f["rate"],"nodes":nodes,"cables":cables,"output":"seq","output_port":"pitch1"})).unwrap()
}
fn taps(engine:&Engine)->[Option<ResolvedTap>;288] {std::array::from_fn(|i|Some(engine.resolve_tap("seq",PORTS[i/16],i%16).unwrap()))}
fn main(){
    let args:Vec<_>=env::args().collect();if args.get(1).map(String::as_str)==Some("--describe"){println!("{}",descriptors());return;}
    let f:Value=serde_json::from_str(&fs::read_to_string(&args[1]).unwrap()).unwrap();assert_eq!(descriptors(),f["descriptors"]);
    let mut version=f["initial_version"].as_u64().unwrap() as usize;
    let mut controls=(1u8,0u8,false,f["initial_live"].as_bool().unwrap());
    let mut engine=patch(&f,version,controls.0,controls.1,controls.2,controls.3).compile().unwrap();let mut observed=taps(&engine);
    for line in fs::read_to_string(&args[2]).unwrap().lines(){
        let c:Vec<_>=line.split_whitespace().collect();assert_eq!(c.len(),10);
        let action=c[0].parse::<u8>().unwrap();let index=c[1].parse::<usize>().unwrap();
        let next=(c[2].parse::<u8>().unwrap(),c[3].parse::<u8>().unwrap(),c[4]=="1",c[7]=="1");
        let data_changed=action==1&&f["versions"][version]!=f["versions"][index];if action==1{version=index;}
        if next!=controls||data_changed {
            let mut replacement=patch(&f,version,next.0,next.1,next.2,next.3).compile().unwrap();
            replacement.adopt_state_from(&engine);replacement.inherit_parameter_values(&engine,&[],&[]);engine=replacement;observed=taps(&engine);controls=next;
        }
        if action==1&&!data_changed{let h=engine.resolve_module_state("seq").unwrap();assert!(engine.reload_rackmcp_sequence_data(h));}
        let manual=c[8].parse::<i32>().unwrap();if manual>=0&&controls.3 {let (module,name)=TARGETS[manual as usize];let h=engine.resolve_parameter(module,name).unwrap();assert!(engine.set_parameter_value(h,c[9].parse().unwrap()));}
        for (module,column) in [("clock_source",5),("reset_source",6)] {let h=engine.resolve_module_state(module).unwrap();assert!(engine.set_host_reference_source(h,PortFrame::mono(c[column].parse().unwrap())));}
        let mut audio=[[0.;2];1];let mut samples=[[TapSample::default();288];1];engine.process_audio_block_with_taps(&mut audio,&observed,&mut samples);
        let mut row=Vec::with_capacity(309);
        for port in 0..18 {let s=&samples[0][port*16..port*16+16];row.push(s.iter().filter(|v|v.active).count().to_string());for v in s{row.push(if v.active{v.value.to_bits()}else{0}.to_string());}}
        for (module,name) in TARGETS{row.push(if controls.3{engine.parameter_value(engine.resolve_parameter(module,name).unwrap()).unwrap().to_bits()}else{0}.to_string());}row.push(u8::from(controls.3).to_string());println!("{}",row.join(" "));
    }
}
