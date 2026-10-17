extern crate openpatch_core;
extern crate serde_json;
use openpatch_core::rackmcp_sequence::{Player,SequenceData};
use std::{env,fs};
fn main() {
    let args:Vec<_>=env::args().collect();
    let data:Vec<SequenceData>=serde_json::from_str(&fs::read_to_string(&args[1]).unwrap()).unwrap();
    let versions:Vec<_>=data.iter().map(|v|v.compile().unwrap()).collect();
    let mut player=Player::default();
    let mut current=0;
    player.adopt_data(&versions[0]);
    for line in fs::read_to_string(&args[2]).unwrap().lines() {
        let cols:Vec<_>=line.split_whitespace().collect();
        let action=cols[0].parse::<u8>().unwrap();
        let index=cols[1].parse::<usize>().unwrap();
        let value=cols[2].parse::<f64>().unwrap();
        match action {
            1=>player.reset(),
            2=>player.seek(value),
            3=>{assert!(player.request_pattern(&versions[current],index,value!=0.0));},
            4=>{current=index;player.adopt_data(&versions[current]);},
            _=>{}
        }
        let frame=player.process(&versions[current],cols[3].parse().unwrap(),cols[4].parse().unwrap(),cols[5].parse().unwrap(),cols[6]=="1",cols[7]=="1",cols[8]=="1");
        let (active,pending,step,loops,beats)=player.position();
        let mut row=vec![active.to_string(),pending.map(|p|p as i32).unwrap_or(-1).to_string(),step.to_string(),loops.to_string(),beats.to_bits().to_string()];
        for t in 0..4 {
            row.push(frame.voices[t].to_string());
            for v in frame.pitch[t] {row.push(v.to_bits().to_string());}
            for v in [frame.gate[t],frame.velocity[t],frame.accent[t]] {row.push(v.to_bits().to_string());}
        }
        row.extend([u8::from(frame.clock).to_string(),u8::from(frame.reset).to_string()]);
        for lane in versions[current].lanes() {
            row.push(lane.as_ref().map(|l|l.value_at(beats/4.0)).unwrap_or(0.0).to_bits().to_string());
        }
        println!("{}",row.join(" "));
    }
}
