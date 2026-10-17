// SPDX-License-Identifier: MIT
// Original probe: calls only the actual production module's public API.
use openpatch_core::rackmcp_listen_analysis::{analyze, encode_wav_float_stereo, note_name, volts_to_dbfs};
use std::{env, fs};
fn main() {
    let args: Vec<String> = env::args().collect();
    assert_eq!(args.len(), 6);
    let bytes = fs::read(&args[1]).unwrap();
    assert_eq!(bytes.len() % 8, 0);
    let mut left = Vec::new(); let mut right = Vec::new();
    for frame in bytes.chunks_exact(8) {
        left.push(f32::from_le_bytes(frame[..4].try_into().unwrap()));
        right.push(f32::from_le_bytes(frame[4..].try_into().unwrap()));
    }
    let rate: f32 = args[2].parse().unwrap();
    let r = analyze(&left, &right, rate, args[3] == "1", args[4] == "1").unwrap();
    let mut entries: Vec<String> = Vec::new();
    let mut emit = |key: &str, value: u64| entries.push(format!("\"{key}\":{value}"));
    emit("sampleRate", u64::from(r.sample_rate.to_bits())); emit("frames",r.frames as u64);
    for (i,c) in r.channels.iter().enumerate() {
        let p=format!("channels.{i}.");
        emit(&(p.clone()+"present"), u64::from(c.present));
        emit(&(p.clone()+"peakVolts"),u64::from(c.peak_volts.to_bits()));
        emit(&(p.clone()+"rmsVolts"),u64::from(c.rms_volts.to_bits()));
        emit(&(p.clone()+"dcVolts"),u64::from(c.dc_volts.to_bits()));
        emit(&(p.clone()+"clippedSamples"),u64::from(c.clipped_samples));
        emit(&(p.clone()+"loudnessValid"),u64::from(c.loudness_valid));
        emit(&(p.clone()+"loudnessLufs"),c.loudness_lufs.to_bits());
        emit(&(p.clone()+"peakDbfs"),volts_to_dbfs(f64::from(c.peak_volts)).to_bits());
        emit(&(p+"rmsDbfs"),volts_to_dbfs(f64::from(c.rms_volts)).to_bits());
    }
    macro_rules! f32field { ($key:literal,$field:ident) => { emit($key,u64::from(r.$field.to_bits())); } }
    macro_rules! f64field { ($key:literal,$field:ident) => { emit($key,r.$field.to_bits()); } }
    macro_rules! flag { ($key:literal,$field:ident) => { emit($key,u64::from(r.$field)); } }
    flag!("correlationValid",correlation_valid); f32field!("correlation",correlation);
    flag!("loudnessValid",loudness_valid); f64field!("integratedLufs",integrated_lufs);
    flag!("momentaryMaxValid",momentary_max_valid); f64field!("momentaryMaxLufs",momentary_max_lufs);
    emit("bands.len",r.bands.len() as u64);
    for (i,b) in r.bands.iter().enumerate() {
        emit(&format!("bands.{i}.centerHz"),u64::from(b.center_hz.to_bits()));
        emit(&format!("bands.{i}.levelDb"),u64::from(b.level_db.to_bits()));
    }
    flag!("centroidValid",centroid_valid); f32field!("centroidHz",centroid_hz);
    flag!("onsetCount",onset_count); flag!("tempoValid",tempo_valid); f32field!("tempoBpm",tempo_bpm);
    f32field!("tempoConfidence",tempo_confidence); flag!("pitchValid",pitch_valid);
    f32field!("pitchHz",pitch_hz); f32field!("pitchConfidence",pitch_confidence);
    let (note,cents)=note_name(r.pitch_hz); emit("noteCents",u64::from(cents.to_bits()));
    entries.push(format!("\"noteName\":\"{note}\""));
    println!("{{{}}}",entries.join(","));
    fs::write(&args[5],encode_wav_float_stereo(&left,Some(&right),rate as u32).unwrap()).unwrap();
    fs::write(format!("{}.mono",args[5]),encode_wav_float_stereo(&left,None,rate as u32).unwrap()).unwrap();
}
