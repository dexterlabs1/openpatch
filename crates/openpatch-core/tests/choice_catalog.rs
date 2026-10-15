//! Choice descriptors are a public UI contract: every advertised raw selection
//! must compile, and omitting a choice must retain the advertised default DSP.
use openpatch_core::{catalog, Patch, PortFrame};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const CONTROLS: [(&str, &str, &str); 5] = [
    ("analog_vco", "waveform", "Saw"),
    ("crossfade", "law", "Linear"),
    ("audio_stereo", "dc_block", "Off"),
    ("score_sequencer", "running", "On"),
    ("drum_voice", "mode", "Kick"),
];

fn spec(kind: &str, parameter: &str) -> Value {
    catalog()["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == kind)
        .unwrap()["params"][parameter]
        .clone()
}

fn fixture(kind: &str) -> Patch {
    // Stimulate each choice so a wrong advertised default cannot hide behind a
    // silent/unconsumed output. SCORE's clock shows its actual transport state.
    let (input, output) = match kind {
        "crossfade" => (Some("a"), "out"),
        "audio_stereo" => (Some("left"), "out"),
        "drum_voice" => (Some("trigger"), "out"),
        "score_sequencer" => (None, "clock"),
        _ => (None, "out"),
    };
    serde_json::from_value(json!({
        "version":2,"sample_rate":48000,
        "nodes":[{"id":"target","kind":kind},
                 {"id":"source","kind":"constant","params":{"value":10.0}}],
        "cables": input.map_or_else(Vec::new, |input| vec![json!({
            "from":"source","to":"target","input":input,"output":"out"
        })]),
        "output":"target","output_port":output
    }))
    .unwrap()
}

fn trace(patch: &Patch) -> Vec<PortFrame> {
    let mut engine = patch.compile().unwrap();
    let mut frames = vec![PortFrame::default(); 512];
    engine.process_poly_block(&mut frames);
    assert!(frames
        .iter()
        .all(|frame| frame.values.iter().all(|v| v.is_finite())));
    frames
}

#[test]
fn advertised_choices_cover_the_real_accepted_raw_domain_and_label_the_default() {
    for (kind, parameter, default_label) in CONTROLS {
        let descriptor = spec(kind, parameter);
        let values = descriptor["values"]
            .as_array()
            .expect("explicit choice values");
        let labels = descriptor["labels"]
            .as_array()
            .expect("explicit choice labels");
        assert_eq!(values.len(), labels.len(), "{kind}.{parameter}");
        let raw: Vec<_> = values.iter().map(|v| v.as_f64().unwrap() as f32).collect();
        let label_set: BTreeSet<_> = labels.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(
            label_set.len(),
            values.len(),
            "each choice needs a distinct label"
        );
        assert!(label_set.iter().all(|label| !label.trim().is_empty()));
        assert!(raw.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(raw[0], descriptor["min"].as_f64().unwrap() as f32);
        assert_eq!(
            *raw.last().unwrap(),
            descriptor["max"].as_f64().unwrap() as f32
        );
        let default = descriptor["default"].as_f64().unwrap() as f32;
        let default_index = raw
            .iter()
            .position(|v| *v == default)
            .expect("default is a choice");
        assert_eq!(labels[default_index], default_label, "{kind}.{parameter}");
        for value in [
            -1.0,
            -0.5,
            0.0,
            0.5,
            1.0,
            1.5,
            2.0,
            2.5,
            3.0,
            f32::NAN,
            f32::INFINITY,
        ] {
            let mut patch = fixture(kind);
            patch.nodes[0].params.insert(parameter.into(), value);
            assert_eq!(
                patch.compile().is_ok(),
                raw.contains(&value),
                "{kind}.{parameter} advertised domain versus builder at {value}"
            );
        }
    }
}

#[test]
fn advertised_defaults_preserve_implicit_behavior_and_all_choices_reach_distinct_branches() {
    for (kind, parameter, _) in CONTROLS {
        let descriptor = spec(kind, parameter);
        let patch = fixture(kind);
        let implicit = trace(&patch);
        let default = descriptor["default"].as_f64().unwrap() as f32;
        let mut choices: Vec<(String, Vec<PortFrame>)> = Vec::new();
        for (value, label) in descriptor["values"]
            .as_array()
            .unwrap()
            .iter()
            .zip(descriptor["labels"].as_array().unwrap())
        {
            let raw = value.as_f64().unwrap() as f32;
            let mut explicit = patch.clone();
            explicit.nodes[0].params.insert(parameter.into(), raw);
            let actual = trace(&explicit);
            if raw == default {
                assert_eq!(
                    actual, implicit,
                    "{kind}.{parameter} implicit default differs from catalog"
                );
            }
            for (other_label, other) in &choices {
                assert_ne!(
                    &actual,
                    other,
                    "{kind}.{parameter}: {} and {other_label} do not reach distinct behavior",
                    label.as_str().unwrap()
                );
            }
            choices.push((label.as_str().unwrap().into(), actual));
        }
    }
}
