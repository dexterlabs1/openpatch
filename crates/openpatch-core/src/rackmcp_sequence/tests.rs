use super::*;
use serde_json::{json, Value};

fn parse(value: Value) -> SequenceData {
    serde_json::from_value(value).unwrap()
}
fn base() -> Value {
    json!({"patterns":[{"name":"A","length":4,"tracks":[{"steps":[{"step":0,"notes":[0,7],"accent":true}]}]}]})
}

#[test]
fn strict_data_retains_patterns_polyphony_and_defaults_without_score_translation() {
    let mut source = base();
    source["patterns"][0]["tracks"][0]["voices"] = json!(4);
    let data = parse(source);
    let compiled = data.compile().unwrap();
    assert_eq!(compiled.pattern_names(), &["A"]);
    let mut player = Player::default();
    player.adopt_data(&compiled);
    let first = player.process(&compiled, 48000.0, 120.0, 50.0, true, false, false);
    assert_eq!(first.voices, [4, 1, 1, 1]);
    assert_eq!(first.pitch[0], [0.0, 7.0 / 12.0, 0.0, 7.0 / 12.0]);
    assert!(first.clock);
    assert_eq!(first.gate[0], 0.0); // genuine one-millisecond retrigger gap
    let mut frame = first;
    for _ in 0..50 {
        frame = player.process(&compiled, 48000.0, 120.0, 50.0, true, false, false);
    }
    assert_eq!(frame.gate[0], 10.0);
    assert_eq!(frame.accent[0], 10.0);
    assert_eq!(frame.velocity[0], 8.0);
    let stopped = player.process(&compiled, 48000.0, 120.0, 50.0, false, false, false);
    assert_eq!(stopped.gate, [0.0; 4]);
    assert_eq!(stopped.velocity[0], 8.0);
}

#[test]
fn invalid_structured_edits_never_produce_a_compiled_store() {
    let mut values = vec![];
    let mut v = base();
    v["patterns"][0]["tracks"][0]["steps"][0]["step"] = json!(4);
    values.push(v);
    let mut v = base();
    v["patterns"][0]["tracks"][0]["steps"]
        .as_array_mut()
        .unwrap()
        .push(json!({"step":0}));
    values.push(v);
    let mut v = base();
    let copy = v["patterns"][0].clone();
    v["patterns"].as_array_mut().unwrap().push(copy);
    values.push(v);
    let mut v = base();
    v["selectedPattern"] = json!("missing");
    values.push(v);
    let mut v = base();
    v["patterns"][0]["tracks"][0]["voices"] = json!(0);
    values.push(v);
    let mut v = base();
    v["patterns"][0]["tracks"][0]["steps"][0]["prob"] = json!(1.1);
    values.push(v);
    for v in values {
        assert!(parse(v).compile().is_err());
    }
    let mut v = base();
    v["patterns"][0]["randomDecoration"] = json!(true);
    assert!(serde_json::from_value::<SequenceData>(v).is_err());
    for field in ["selectedPattern", "patterns", "lanes", "version", "seed"] {
        let mut v = base();
        v[field] = Value::Null;
        assert!(
            serde_json::from_value::<SequenceData>(v).is_err(),
            "accepted null {field}"
        );
    }
}

#[test]
fn target_lanes_keep_source_ids_easing_duplicate_bar_order_and_loop_endpoint() {
    let mut v = base();
    v["lanes"] = json!([{"moduleId":"9223372036854775807","paramId":65535,"lengthBars":4,"points":[
        {"bar":0,"value":0},{"bar":1,"value":1,"curve":"easeIn"},
        {"bar":1,"value":0.25},{"bar":3,"value":0.75,"curve":"hold"}
    ]}]);
    let compiled = parse(v).compile().unwrap();
    let lane = compiled.lanes()[0].as_ref().unwrap();
    assert_eq!((lane.module_id, lane.param_id), (i64::MAX, u16::MAX));
    assert_eq!(lane.value_at(-1.0), 0.0);
    assert_eq!(lane.value_at(0.5), 0.125);
    assert_eq!(lane.value_at(1.0), 0.25);
    assert_eq!(lane.value_at(2.0), 0.25);
    assert_eq!(lane.value_at(3.0), 0.75);
    assert_eq!(lane.value_at(4.0), 0.0);
}

#[test]
fn lane_targets_are_unique_and_canonical_and_native_float_safety_is_explicit() {
    let lane = json!({"moduleId":"12","paramId":0,"mode":"raw","points":[{"bar":0,"value":-5}]});
    for id in ["012", "-1", "+1", "9223372036854775808"] {
        let mut v = base();
        let mut l = lane.clone();
        l["moduleId"] = json!(id);
        v["lanes"] = json!([l]);
        assert!(parse(v).compile().is_err());
    }
    let mut v = base();
    v["lanes"] = json!([lane.clone(), lane.clone()]);
    assert!(parse(v).compile().is_err());
    let mut v = base();
    let mut overflow = lane;
    overflow["points"][0]["value"] = json!(1e300);
    v["lanes"] = json!([overflow]);
    assert!(matches!(parse(v).compile(), Err(e) if e.contains("f32")));
}

#[test]
fn data_adoption_preserves_playhead_and_nonzero_first_selection_waits_for_advance() {
    let mut v = base();
    v["patterns"].as_array_mut().unwrap().push(json!({"name":"B","length":2,"tracks":[{"steps":[{"step":0,"notes":[12]},{"step":1,"notes":[24]}]}]}));
    v["selectedPattern"] = json!("B");
    let a = parse(v.clone()).compile().unwrap();
    let mut player = Player::default();
    player.adopt_data(&a);
    assert_eq!(player.position().1, Some(1));
    player.process(&a, 48000.0, 120.0, 50.0, true, true, true);
    assert_eq!(player.position().0, 0);
    player.process(&a, 48000.0, 120.0, 50.0, true, true, true);
    assert_eq!((player.position().0, player.position().2), (1, 1));
    for _ in 0..128 {
        player.process(&a, 48000.0, 120.0, 50.0, true, true, false);
    }
    let position = player.position();
    v["patterns"][1]["tracks"][0]["steps"][1]["notes"] = json!([-12]);
    let b = parse(v).compile().unwrap();
    player.adopt_data(&b);
    assert_eq!(player.position(), position);
    assert!(!player.request_pattern(&b, 16, true));
}
