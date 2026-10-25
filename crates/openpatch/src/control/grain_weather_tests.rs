use super::*;

fn grain_session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute(
        "replace_patch",
        json!({"patch":{"version":2,"sample_rate":48000,
        "nodes":[
            {"id":"voice","kind":"stock_vco","params":{"frequency":-5.0}},
            {"id":"grain","kind":"grain_weather","params":{"density":0.8,"blend":1.0,"position":0.05}}],
        "cables":[{"from":"voice","output":"saw","to":"grain","input":"left"}],
        "output":"grain","output_port":"left"}}),
    )
    .unwrap();
    s
}

fn peak(s: &mut Session, port: &str) -> f64 {
    let trace = s
        .execute(
            "capture_signal",
            json!({"id":"grain","port":port,"seconds":0.5,"points":64}),
        )
        .unwrap();
    trace["bins"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            b["max"]
                .as_f64()
                .unwrap()
                .abs()
                .max(b["min"].as_f64().unwrap().abs())
        })
        .fold(0.0, f64::max)
}

#[test]
fn grain_weather_parameters_undo_redo_and_capture_stay_observational() {
    let mut s = grain_session();
    let original = s.snapshot();
    assert!(peak(&mut s, "left") > 0.3);
    assert!(
        peak(&mut s, "right") > 0.3,
        "right follows the normalled left input"
    );
    assert_eq!(s.snapshot(), original, "capture must remain observational");
    for (name, value) in [
        ("mode", 3.0),
        ("quality", 3.0),
        ("pitch", -7.0),
        ("reverb", 0.6),
        ("freeze", 1.0),
    ] {
        s.execute(
            "set_parameter",
            json!({"id":"grain","parameter":name,"value":value,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    let changed = s.snapshot();
    assert_eq!(changed["patch"]["nodes"][1]["params"]["mode"], 3.0);
    for _ in 0..5 {
        s.execute("undo", json!({})).unwrap();
    }
    assert_eq!(s.snapshot()["patch"], original["patch"]);
    for _ in 0..5 {
        s.execute("redo", json!({})).unwrap();
    }
    assert_eq!(s.snapshot()["patch"], changed["patch"]);
    let before = s.snapshot();
    for (name, value) in [
        ("mode", 1.5),
        ("mode", 4.0),
        ("quality", -1.0),
        ("freeze", 0.5),
        ("pitch", 30.0),
        ("blend", 1.2),
    ] {
        assert!(s
            .execute(
                "set_parameter",
                json!({"id":"grain","parameter":name,"value":value})
            )
            .is_err());
        assert_eq!(s.snapshot(), before, "{name}={value}");
    }
}

#[test]
fn grain_weather_initialize_bypass_and_presets_hold_controls_not_audio() {
    let mut s = grain_session();
    s.execute(
        "set_parameter",
        json!({"id":"grain","parameter":"texture","value":0.2}),
    )
    .unwrap();
    let edited = s.snapshot()["patch"].clone();
    s.execute("initialize_modules", json!({"ids":["grain"]}))
        .unwrap();
    let params = &s.snapshot()["patch"]["nodes"][1]["params"];
    assert!((params["texture"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert!((params["density"].as_f64().unwrap() - 0.7).abs() < 1e-6);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.snapshot()["patch"], edited);

    let preset = s
        .execute("capture_module_preset", json!({"id":"grain"}))
        .unwrap()["preset"]
        .clone();
    assert_eq!(preset["params"]["texture"], json!(0.2f32));
    assert!(
        serde_json::to_string(&preset).unwrap().len() < 2048,
        "presets hold authored controls, never buffer audio"
    );

    s.execute("set_bypass", json!({"id":"grain","bypassed":true}))
        .unwrap();
    // Bypass routes left to left unchanged; the unpatched right is silent.
    assert!(peak(&mut s, "left") > 4.0);
    assert_eq!(peak(&mut s, "right"), 0.0);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.snapshot()["patch"], edited);
}
