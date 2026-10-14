use openpatch_core::{Patch, TapSample};
use serde_json::json;
use std::{
    fs::OpenOptions,
    io::{BufWriter, Write},
    path::Path,
};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dest = Path::new(&args[1]);
    std::fs::create_dir_all(dest).unwrap();
    const N: usize = 8192;
    let mut cases = vec![];
    for rate in [44100, 48000, 96000] {
        for bin in [43, 171, 683, 1501] {
            for amplitude in [0.1, 1.0, 5.0] {
                for drive in [0.0, 0.1, 0.5, 1.0] {
                    let frequency = f64::from(rate) * f64::from(bin) / N as f64;
                    assert_eq!(frequency, f64::from(frequency as f32));
                    let patch: Patch = serde_json::from_value(json!({"version":2,"sample_rate":rate,
                        "nodes":[{"id":"sine","kind":"oscillator","params":{"frequency":frequency,"amplitude":1}},
                        {"id":"input","kind":"gain","params":{"gain":amplitude}},
                        {"id":"filter","kind":"svf_filter","params":{"cutoff":12000,"resonance":0.2,"drive":drive}}],
                        "cables":[{"id":"a","from":"sine","to":"input","input":"signal"},
                        {"id":"b","from":"input","to":"filter","input":"signal"}],"output":"filter","output_port":"band"})).unwrap();
                    let mut engine = patch.compile().unwrap();
                    let taps = [
                        Some(engine.resolve_tap("filter", "low", 0).unwrap()),
                        Some(engine.resolve_tap("filter", "high", 0).unwrap()),
                    ];
                    let mut audio = [[0.0; 2]; 128];
                    let mut captures = [[TapSample::default(); 2]; 128];
                    let filename = format!("case-{}.f32", cases.len());
                    let mut output = BufWriter::new(
                        OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(dest.join(&filename))
                            .unwrap(),
                    );
                    for block in 0..8 * N / 128 {
                        engine.process_audio_block_with_taps(&mut audio, &taps, &mut captures);
                        if block >= 6 * N / 128 {
                            for (sample, tap) in audio.iter().zip(&captures) {
                                assert!(tap.iter().all(|t| t.active && t.value.is_finite()));
                                for value in [tap[0].value, sample[0], tap[1].value] {
                                    assert!(value.is_finite());
                                    output.write_all(&value.to_le_bytes()).unwrap();
                                }
                            }
                        }
                    }
                    output.flush().unwrap();
                    cases.push(json!({"rate":rate,"bin":bin,"frequency_hz":frequency,"amplitude_volts_peak":amplitude,"drive":drive,"cutoff_hz":12000,"resonance":0.2,"trace":filename}));
                }
            }
        }
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dest.join("cases.json"))
        .unwrap();
    file.write_all(serde_json::to_string_pretty(&cases).unwrap().as_bytes())
        .unwrap();
    println!("Exported {} actual Engine cases", cases.len());
}
