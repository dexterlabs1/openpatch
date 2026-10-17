use openpatch_core::rackmcp_listen_analysis::{analyze, encode_wav_float_stereo, fft};

#[test]
fn invalid_analysis_boundaries_are_errors_without_partial_or_nonfinite_json_results() {
    let samples = [0.0; 800];
    for rate in [0.0, 7999.0, 192001.0, f32::NAN, f32::INFINITY] {
        assert!(analyze(&samples, &samples, rate, true, true).is_err());
    }
    assert!(analyze(&[], &[], 48000.0, true, true).is_err());
    assert!(analyze(&samples, &samples[..799], 8000.0, true, true).is_err());
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, f32::MAX] {
        assert!(analyze(&[value; 800], &samples, 8000.0, true, true).is_err());
    }
    assert!(analyze(&vec![0.0; 240001], &vec![0.0; 240001], 8000.0, true, true).is_err());
    assert!(fft(&mut [0.0; 3], &mut [0.0; 3]).is_err());
}

#[test]
fn silence_is_a_finite_analysis_result_and_wav_duplication_is_explicit_export_only() {
    let samples = [0.0; 800];
    let result = analyze(&samples, &samples, 8000.0, true, false).unwrap();
    let json = serde_json::to_value(result).unwrap();
    assert_eq!(json["frames"], 800);
    assert_eq!(json["channels"][0]["present"], true);
    assert_eq!(json["channels"][1]["present"], false);
    assert_eq!(json["loudnessValid"], false);
    assert_eq!(json["pitchValid"], false);
    let left = [10.0, -20.0];
    let raw = encode_wav_float_stereo(&left, Some(&[0.0; 2]), 48000).unwrap();
    let mono = encode_wav_float_stereo(&left, None, 48000).unwrap();
    let sample =
        |bytes: &[u8], offset| f32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap());
    assert_eq!(sample(&raw, 58), 1.0);
    assert_eq!(sample(&raw, 62), 0.0);
    assert_eq!(sample(&raw, 66), -2.0);
    assert_eq!(sample(&mono, 62), 1.0);
    assert_eq!(sample(&mono, 70), -2.0);
}
