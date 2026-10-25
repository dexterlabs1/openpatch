use super::*;

fn strings_session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute(
        "replace_patch",
        json!({"patch":{"version":2,"sample_rate":48000,
        "nodes":[
            {"id":"clock","kind":"stock_lfo","params":{"rate":3.0,"offset":0.0}},
            {"id":"strings","kind":"string_field","params":{"model":2.0,"brightness":0.7}}],
        "cables":[{"from":"clock","output":"square","to":"strings","input":"strum"}],
        "output":"strings","output_port":"odd"}}),
    )
    .unwrap();
    s
}

fn peak(s: &mut Session, port: &str) -> f64 {
    let trace = s
        .execute(
            "capture_signal",
            json!({"id":"strings","port":port,"seconds":0.5,"points":64}),
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
fn string_field_parameters_undo_redo_and_capture_stay_observational() {
    let mut s = strings_session();
    let original = s.snapshot();
    assert!(peak(&mut s, "odd") > 0.1, "the internal exciter plays");
    assert_eq!(s.snapshot(), original, "capture must remain observational");
    for (name, value) in [
        ("model", 4.0),
        ("polyphony", 3.0),
        ("frequency", 41.0),
        ("damping", 0.9),
        ("structure_amount", -0.5),
    ] {
        s.execute(
            "set_parameter",
            json!({"id":"strings","parameter":name,"value":value,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    let changed = s.snapshot();
    assert_eq!(changed["patch"]["nodes"][1]["params"]["model"], 4.0);
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
        ("model", 2.5),
        ("model", 6.0),
        ("polyphony", 0.0),
        ("polyphony", 5.0),
        ("frequency", 70.0),
        ("damping_amount", 1.5),
    ] {
        assert!(s
            .execute(
                "set_parameter",
                json!({"id":"strings","parameter":name,"value":value})
            )
            .is_err());
        assert_eq!(s.snapshot(), before, "{name}={value}");
    }
}

#[test]
fn string_field_initialize_bypass_and_presets_hold_controls_not_memory() {
    let mut s = strings_session();
    s.execute(
        "set_parameter",
        json!({"id":"strings","parameter":"position","value":0.2}),
    )
    .unwrap();
    let edited = s.snapshot()["patch"].clone();
    s.execute("initialize_modules", json!({"ids":["strings"]}))
        .unwrap();
    let params = &s.snapshot()["patch"]["nodes"][1]["params"];
    assert!((params["position"].as_f64().unwrap() - 0.5).abs() < 1e-6);
    assert!((params["frequency"].as_f64().unwrap() - 30.0).abs() < 1e-6);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.snapshot()["patch"], edited);

    let preset = s
        .execute("capture_module_preset", json!({"id":"strings"}))
        .unwrap()["preset"]
        .clone();
    assert_eq!(preset["params"]["position"], json!(0.2f32));
    assert!(
        serde_json::to_string(&preset).unwrap().len() < 2048,
        "presets hold authored controls, never string memory"
    );

    s.execute("set_bypass", json!({"id":"strings","bypassed":true}))
        .unwrap();
    // Bypass routes the unpatched exciter input: silence on both outputs.
    assert_eq!(peak(&mut s, "odd"), 0.0);
    assert_eq!(peak(&mut s, "even"), 0.0);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.snapshot()["patch"], edited);
    assert!(peak(&mut s, "even") > 0.1);
}
