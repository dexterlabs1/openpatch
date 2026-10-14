use openpatch_core::{Patch, PortFrame};
use serde_json::json;
use std::{hint::black_box, time::Instant};
fn main() {
    let mut cases = Vec::new();
    for count in [1,16,64] {
        let nodes: Vec<_> = (0..count).map(|i| json!({"id":format!("noise{i}"),"kind":"colored_noise","params":{"seed":i+1}})).collect();
        let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":nodes,"output":"noise0","output_port":"gray"})).unwrap();
        let mut timings=Vec::new();
        let mut checksum=0.0_f64;
        for _ in 0..5 {
            let mut engine=patch.compile().unwrap();
            let mut output=[PortFrame::default();128];
            engine.process_poly_block(&mut output);
            let start=Instant::now();
            for _ in 0..750 {
                engine.process_poly_block(black_box(&mut output));
                black_box(&output);
            }
            timings.push(start.elapsed().as_secs_f64());
            for frame in output {
                assert_eq!(frame.channels,1);
                assert!(frame.values[0].is_finite());
                checksum+=f64::from(frame.values[0]);
            }
        }
        timings.sort_by(f64::total_cmp);
        cases.push(json!({"modules":count,"simultaneous_outputs_per_module":7,
            "audio_seconds_per_trial":2,"trials":5,"block_frames":128,
            "min_seconds":timings[0],"median_seconds":timings[2],"max_seconds":timings[4],
            "median_realtime_factor":2.0/timings[2],"checksum":checksum}));
    }
    println!("{}",json!({"sample_rate":48000,"cases":cases}));
}
