// Offline visual fixture for the two version-specific panels. No audio device,
// native window, saved template or user patch is opened by this preparation.
pub(crate) fn observed_voice() -> (openpatch_core::Patch, openpatch_core::Engine) {
    let patch: openpatch_core::Patch = serde_json::from_value(serde_json::json!({
        "version":2,"sample_rate":48000,
        "nodes":[
            {"id":"voice","kind":"rack264_vco","params":{"frequency":-12,"pulse_width":0.32,"fm_amount":0.2,"pwm_amount":-0.3,"linear_fm":1,"soft_sync":1}},
            {"id":"filter","kind":"rack264_vcf","params":{"frequency":0.62,"resonance":0.3,"drive":0.15,"cutoff_cv_amount":0.5,"resonance_cv_amount":-0.2,"drive_cv_amount":0.25}}
        ],
        "cables":[{"from":"voice","output":"saw","to":"filter","input":"signal"}],
        "output":"filter","output_port":"lowpass"
    })).unwrap();
    let mut engine = patch.compile().expect("Rack 2.6.4 visual voice compiles");
    engine.set_port_lights_enabled(true);
    let mut audio = vec![[0.0; 2]; 4096];
    engine.process_audio_block(&mut audio);
    assert!(audio.iter().flatten().all(|value| value.is_finite()));
    assert!(audio.iter().flatten().any(|value| value.abs() > 0.001));
    let handle = engine.resolve_module_lights("voice").unwrap();
    let lamps = engine.module_lights(handle).unwrap();
    assert_eq!(lamps.count, 5);
    assert_eq!([lamps.brightness[3][0], lamps.brightness[4][0]], [1.0, 1.0]);
    (patch, engine)
}
