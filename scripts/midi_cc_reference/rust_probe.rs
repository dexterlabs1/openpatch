// SPDX-License-Identifier: MIT
// Minimal host adapter: production MidiCc implements all controller/filter math.
#![allow(dead_code)]
use std::{collections::BTreeMap,io::{self,BufRead,Write}};
#[path = __MIDI_CC_SOURCE__] mod midi_cc;
#[derive(Debug)] struct Node{id:String,kind:String,params:BTreeMap<String,f32>}
fn parameter(node:&Node,name:&str,default:f32,min:f32,max:f32)->Result<f32,String>{let v=node.params.get(name).copied().unwrap_or(default);if v.is_finite()&&(min..=max).contains(&v){Ok(v)}else{Err(name.into())}}
#[derive(Clone,Copy,Debug,PartialEq,Eq)] enum SavedModuleState{MidiCc{assignments:[i16;16],values:[u8;128]},MidiCcMappings([i16;16])}
#[derive(Clone,Copy,Debug,PartialEq,Eq)] enum MidiEvent{ControlChange{channel:u8,controller:u8,value:u8},NoteOn{channel:u8,note:u8,velocity:u8},Panic}
impl MidiEvent{fn is_valid(&self)->bool{match *self{Self::ControlChange{channel,controller,value}=>channel<16&&controller<128&&value<128,Self::NoteOn{channel,note,velocity}=>channel<16&&note<128&&velocity<128,_=>true}}}
#[derive(Clone,Copy,Debug,Default)] struct PortFrame{channels:u8,values:[f32;16]}
fn main(){
 let rate=std::env::args().nth(1).unwrap().parse().unwrap();
 let mut node=Node{id:"oracle".into(),kind:"midi_cc".into(),params:BTreeMap::new()};let mut module=midi_cc::MidiCc::build(&node,rate).unwrap();
 let mut previous=[-999;20];let mut saved=None;let mut saved_options=[0;4];let mut serial=0;let mut out=io::BufWriter::new(io::stdout().lock());
 for line in io::stdin().lock().lines(){let v:Vec<i32>=line.unwrap().split_whitespace().map(|s|s.parse().unwrap()).collect();let config:[i32;20]=v[..4].iter().chain(v[8..24].iter()).copied().collect::<Vec<_>>().try_into().unwrap();
  if config!=previous{node.params.clear();for(i,name)in["smooth","mpe","high_resolution","channel"].iter().enumerate(){node.params.insert(name.to_string(),config[i]as f32);}for i in 0..16{node.params.insert(format!("cc{}",i+1),config[4+i]as f32);}let mut next=midi_cc::MidiCc::build(&node,rate).unwrap();next.adopt(&module);module=next;previous=config;}
  let mut cursor=24;let op=v[5];if op==1{let values=std::array::from_fn(|i|v[cursor+i]as u8);cursor+=128;let SavedModuleState::MidiCc{assignments,..}=module.saved_state()else{panic!()};assert!(module.set_saved_state(SavedModuleState::MidiCc{assignments,values}));}
  if op==2{saved=Some(module.saved_state());saved_options.copy_from_slice(&config[..4]);}
  if op==3{assert_eq!(&config[..4],&saved_options);assert!(module.set_saved_state(saved.unwrap()));}
  if v[6]>0{serial+=1;assert!(module.arm_learn(serial,(v[6]-1)as u8));}
  for _ in 0..v[7]{let(status,channel,controller,value)=(v[cursor],v[cursor+1]as u8,v[cursor+2]as u8,v[cursor+3]as u8);cursor+=4;module.handle_midi(if status==11{MidiEvent::ControlChange{channel,controller,value}}else{MidiEvent::NoteOn{channel,note:controller,velocity:value}});}
  assert_eq!(cursor,v.len());let mut outputs=[PortFrame::default();16];module.process(&mut outputs,v[4]as u16);
  let SavedModuleState::MidiCc{assignments,values}=module.saved_state()else{panic!()};let completed=module.take_learn_completion().map_or(-1,|c|c.cell as i32);let learning=module.learning().map_or(-1,|(_,c)|c as i32);
  for number in assignments.into_iter().map(i32::from).chain(values.into_iter().map(i32::from)).chain([learning,completed]).chain(outputs.iter().map(|o|o.channels as i32)){out.write_all(&number.to_le_bytes()).unwrap();}
  for output in outputs{for value in output.values{out.write_all(&value.to_le_bytes()).unwrap();}}
 }
}
