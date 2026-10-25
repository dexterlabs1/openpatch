use super::*;

fn cross_session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute(
        "replace_patch",
        json!({"patch":{"version":2,"sample_rate":48000,
        "nodes":[
            {"id":"mod","kind":"stock_vco","params":{"frequency":7.0}},
            {"id":"cross","kind":"cross_matter","params":{"algorithm":0.3}}],
        "cables":[{"from":"mod","output":"saw","to":"cross","input":"modulator"}],
        "output":"cross","output_port":"out"}}),
    )
    .unwrap();
    s
}

fn peak(s: &mut Session, port: &str) -> f64 {
    let trace = s
        .execute(
            "capture_signal",
            json!({"id":"cross","port":port,"seconds":0.1,"points":64}),
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
fn cross_matter_parameters_undo_redo_and_capture_stay_observational() {
    let mut s = cross_session();
    let original = s.snapshot();
    assert!(peak(&mut s, "out") > 0.5);
    assert!(peak(&mut s, "aux") > 0.5);
    assert_eq!(s.snapshot(), original, "capture must remain observational");
    for (name, value) in [
        ("algorithm", 0.85),
        ("carrier", 3.0),
        ("mode", 1.0),
        ("timbre", 0.1),
    ] {
        s.execute(
            "set_parameter",
            json!({"id":"cross","parameter":name,"value":value,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    let changed = s.snapshot();
    assert_eq!(changed["patch"]["nodes"][1]["params"]["mode"], 1.0);
    assert!(peak(&mut s, "out") > 0.1);
    for _ in 0..4 {
        s.execute("undo", json!({})).unwrap();
    }
    assert_eq!(s.snapshot()["patch"], original["patch"]);
    for _ in 0..4 {
        s.execute("redo", json!({})).unwrap();
    }
    assert_eq!(s.snapshot()["patch"], changed["patch"]);
    let before = s.snapshot();
    for (name, value) in [
        ("carrier", 1.5),
        ("carrier", 4.0),
        ("mode", 2.0),
        ("algorithm", 1.2),
    ] {
        assert!(s
            .execute(
                "set_parameter",
                json!({"id":"cross","parameter":name,"value":value})
            )
            .is_err());
        assert_eq!(s.snapshot(), before, "{name}={value}");
    }
}

#[test]
fn cross_matter_initialize_bypass_and_presets_use_shared_history() {
    let mut s = cross_session();
    s.execute(
        "set_parameter",
        json!({"id":"cross","parameter":"level2","value":0.2}),
    )
    .unwrap();
    let edited = s.snapshot()["patch"].clone();
    s.execute("initialize_modules", json!({"ids":["cross"]}))
        .unwrap();
    let params = &s.snapshot()["patch"]["nodes"][1]["params"];
    assert!((params["level2"].as_f64().unwrap() - 0.7).abs() < 1e-6);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.snapshot()["patch"], edited);

    let preset = s
        .execute("capture_module_preset", json!({"id":"cross"}))
        .unwrap()["preset"]
        .clone();
    assert_eq!(preset["params"]["level2"], json!(0.2f32));

    s.execute("set_bypass", json!({"id":"cross","bypassed":true}))
        .unwrap();
    // Bypass routes the modulator to aux unchanged; out has no input.
    assert!(peak(&mut s, "aux") > 4.0);
    assert_eq!(peak(&mut s, "out"), 0.0);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.snapshot()["patch"], edited);
}
