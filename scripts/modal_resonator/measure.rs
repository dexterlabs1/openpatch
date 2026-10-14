use openpatch_core::{Patch, PortFrame};
use serde_json::json;
use std::{hint::black_box, time::Instant};
fn main() {
    let mut cases = Vec::new();
    for voices in [1,16] {
        for modulation in [false,true] {
            let mut params = serde_json::Map::new();
            params.insert("channels".into(),json!(voices));
            for i in 1..=voices { params.insert(format!("v{i}"),json!(10.0)); }
            let mut patch = json!({"version":2,"sample_rate":48000,"nodes":[
                {"id":"strikes","kind":"poly_constant","params":params},
                {"id":"body","kind":"modal_resonator","params":{"frequency":220,"stiffness":0.025,"decay":30,"damping":0.1}}
            ],"cables":[{"from":"strikes","to":"body","input":"strike"}],"output":"body"});
            if modulation {
                patch["nodes"].as_array_mut().unwrap().extend([
                    json!({"id":"mod","kind":"multi_lfo","params":{"frequency":80}}),
                    json!({"id":"depth","kind":"attenuverter","params":{"gain":0.02}})
                ]);
                patch["cables"].as_array_mut().unwrap().extend([
                    json!({"from":"mod","output":"sine","to":"depth","input":"signal"}),
                    json!({"from":"depth","to":"body","input":"pitch"})
                ]);
            }
            let patch: Patch = serde_json::from_value(patch).unwrap();
            let mut timings=Vec::new();
            let mut checksum=0.0_f64;
            let mut peak=0.0_f32;
            let mut channels=0;
            for _ in 0..5 {
                let mut engine=patch.compile().unwrap();
                let mut output=[PortFrame::default();128];
                engine.process_poly_block(&mut output);
                let start=Instant::now();
                // Exactly one second at 48 kHz, 128-frame blocks.
                for _ in 0..375 {
                    engine.process_poly_block(black_box(&mut output));
                    black_box(&output);
                }
                timings.push(start.elapsed().as_secs_f64());
                for frame in output {
                    channels=channels.max(frame.channels);
                    for sample in &frame.values[..frame.channels as usize] {
                        assert!(sample.is_finite());
                        peak=peak.max(sample.abs());checksum+=f64::from(*sample);
                    }
                }
            }
            timings.sort_by(f64::total_cmp);
            assert_eq!(channels,voices);
            cases.push(json!({"voices":voices,"pitch":"1 V/oct",
                "modulation":if modulation {"80 Hz sine, +/-0.1 V, mono broadcasts"} else {"fixed 0 V"},
                "audio_seconds_per_trial":1,"trials":5,"block_frames":128,
                "min_seconds":timings[0],"median_seconds":timings[2],"max_seconds":timings[4],
                "median_realtime_factor":1.0/timings[2],"final_block_peak_v":peak,"checksum":checksum}));
        }
    }
    println!("{}",json!({"sample_rate":48000,"cases":cases}));
}
