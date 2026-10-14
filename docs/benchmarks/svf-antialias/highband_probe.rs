
use crate::{Node, PortFrame};
use crate::voice_tools::VoiceTools;
use serde_json::json;
use std::{fs::OpenOptions,io::{BufWriter,Write},path::Path};
pub fn export(dest:&Path) {
 std::fs::create_dir_all(dest).unwrap();
 const N:usize=8192;
 let mut cases=vec![];
 for rate in [44100,48000,96000] {
  let mut bins=vec![4,43,171,683,1501,3441,3687,3933,4014];
  bins.push((18000.0/rate as f64*N as f64).round() as usize);
  bins.push((20000.0/rate as f64*N as f64).round() as usize);
  bins.sort(); bins.dedup();
  for bin in bins {
   for amplitude in [0.1_f64,5.0] {
    for drive in [0.0_f64,0.1,1.0] {
     let node:Node=serde_json::from_value(json!({"id":"filter","kind":"svf_filter","params":{"cutoff":12000,"resonance":0.2,"drive":drive}})).unwrap();
     let mut filter=VoiceTools::build(&node,rate).unwrap();
     let filename=format!("case-{}.f32",cases.len());
     let mut file=BufWriter::new(OpenOptions::new().create_new(true).write(true).open(dest.join(&filename)).unwrap());
     for n in 0..8*N {
      let x=if n<2 {0.0} else {amplitude*(std::f64::consts::TAU*(((n-2)*bin)%N) as f64/N as f64).sin()};
      let mut outputs=[PortFrame::default();3];
      filter.process(&[PortFrame::mono(x as f32)],&mut outputs);
      if n>=6*N {for port in outputs {assert!(port.values[0].is_finite());file.write_all(&port.values[0].to_le_bytes()).unwrap();}}
     }
     file.flush().unwrap();
     cases.push(json!({"rate":rate,"bin":bin,"frequency_hz":rate as f64*bin as f64/N as f64,"amplitude_volts_peak":amplitude,"drive":drive,"cutoff_hz":12000,"resonance":0.2,"trace":filename}));
    }
   }
  }
 }
 let mut file=OpenOptions::new().create_new(true).write(true).open(dest.join("cases.json")).unwrap();
 file.write_all(serde_json::to_string_pretty(&cases).unwrap().as_bytes()).unwrap();
 println!("Exported {} direct component cases; analytic input delayed two samples",cases.len());
}
