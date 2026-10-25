use super::*;

fn session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute(
        "replace_patch",
        json!({"patch":{"version":2,"sample_rate":48000,
        "nodes":[
            {"id":"voices","kind":"poly_constant","params":{"channels":2,"v1":4.0,"v2":-2.0}},
            {"id":"vca","kind":"stock_vca2","params":{"level1":0.5}},
            {"id":"unity","kind":"stock_unity","params":{}}],
        "cables":[
            {"from":"voices","output":"out","to":"vca","input":"in1"},
            {"from":"vca","output":"out1","to":"unity","input":"ch1_1"},
            {"from":"voices","output":"out","to":"unity","input":"ch2_1"}],
        "output":"unity","output_port":"mix1"}}),
    )
    .unwrap();
    s
}

fn level(s: &mut Session, id: &str, port: &str) -> f64 {
    let trace = s
        .execute(
            "capture_signal",
            json!({"id":id,"port":port,"seconds":0.05,"points":64}),
        )
        .unwrap();
    trace["bins"].as_array().unwrap().last().unwrap()["max"]
        .as_f64()
        .unwrap()
}

#[test]
fn vca2_and_unity_controls_undo_redo_and_capture_stay_observational() {
    let mut s = session();
    let original = s.snapshot();
    assert_eq!(level(&mut s, "unity", "mix1"), 2.0);
    assert_eq!(level(&mut s, "unity", "inv2"), -4.0);
    assert_eq!(s.snapshot(), original, "capture must remain observational");
    for (id, name, value) in [
        ("vca", "level1", 1.0),
        ("unity", "merge", 1.0),
        ("unity", "average1", 1.0),
    ] {
        s.execute(
            "set_parameter",
            json!({"id":id,"parameter":name,"value":value,"expected_revision":s.revision()}),
        )
        .unwrap();
    }
    // Merged: 4 V from VCA-2 plus 4 V from the voices, averaged over two jacks.
    assert_eq!(level(&mut s, "unity", "mix1"), 4.0);
    assert_eq!(level(&mut s, "unity", "mix2"), 8.0);
    let changed = s.snapshot();
    for _ in 0..3 {
        s.execute("undo", json!({})).unwrap();
    }
    assert_eq!(s.snapshot()["patch"], original["patch"]);
    for _ in 0..3 {
        s.execute("redo", json!({})).unwrap();
    }
    assert_eq!(s.snapshot()["patch"], changed["patch"]);
    let before = s.snapshot();
    for (id, name, value) in [
        ("vca", "level2", 1.5),
        ("vca", "level1", -0.1),
        ("unity", "merge", 0.5),
        ("unity", "average2", 2.0),
    ] {
        assert!(s
            .execute(
                "set_parameter",
                json!({"id":id,"parameter":name,"value":value})
            )
            .is_err());
        assert_eq!(s.snapshot(), before, "{name}={value}");
    }
}

#[test]
fn unity_initialize_turns_merge_off_and_bypass_silences_it() {
    let mut s = session();
    s.execute(
        "set_parameter",
        json!({"id":"unity","parameter":"merge","value":1.0}),
    )
    .unwrap();
    let edited = s.snapshot()["patch"].clone();
    s.execute("initialize_modules", json!({"ids":["unity","vca"]}))
        .unwrap();
    let nodes = &s.snapshot()["patch"]["nodes"];
    assert_eq!(nodes[2]["params"]["merge"].as_f64().unwrap_or(0.0), 0.0);
    assert_eq!(nodes[1]["params"]["level1"].as_f64().unwrap_or(1.0), 1.0);
    s.execute("undo", json!({})).unwrap();
    assert_eq!(s.snapshot()["patch"], edited);
    s.execute("set_bypass", json!({"id":"unity","bypassed":true}))
        .unwrap();
    assert_eq!(level(&mut s, "unity", "mix1"), 0.0);
    s.execute("set_bypass", json!({"id":"vca","bypassed":true}))
        .unwrap();
    // VCA-2 bypass passes its input unchanged: the full 4 V voice.
    assert_eq!(level(&mut s, "vca", "out1"), 4.0);
}
