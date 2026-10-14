use openpatch_core::{Cable, Patch};
use serde_json::json;

#[test]
fn optional_color_roundtrips_and_legacy_cables_omit_it() {
    let legacy = json!({"from":"source","to":"sink","input":"signal"});
    let cable: Cable = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(cable.color, None);
    assert_eq!(serde_json::to_value(&cable).unwrap(), legacy);

    let colored =
        json!({"id":"route","from":"source","to":"sink","input":"signal","color":[0,128,255]});
    let cable: Cable = serde_json::from_value(colored.clone()).unwrap();
    assert_eq!(cable.color, Some([0, 128, 255]));
    assert_eq!(serde_json::to_value(&cable).unwrap(), colored);

    for color in [
        json!([]),
        json!([0, 1]),
        json!([0, 1, 2, 3]),
        json!([-1, 0, 0]),
        json!([0, 256, 0]),
        json!([0, 0.5, 0]),
        json!([0, "1", 0]),
        json!([0, true, 0]),
        json!("#0080ff"),
        json!({}),
    ] {
        let mut invalid = legacy.clone();
        invalid["color"] = color;
        assert!(
            serde_json::from_value::<Cable>(invalid.clone()).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn changing_only_colors_preserves_oscillator_filter_and_delayed_cable_state() {
    for version in [1, 2] {
        for explicit_ids in [false, true] {
            let mut patch: Patch = serde_json::from_value(json!({
                "version":version,
                "nodes":[
                    {"id":"source","kind":"oscillator","params":{"frequency":137}},
                    {"id":"filter","kind":"lowpass","params":{"cutoff":300}},
                    {"id":"sink","kind":"gain"}
                ],
                "cables":[
                    {"from":"source","to":"filter","input":"signal"},
                    {"from":"filter","to":"sink","input":"signal"}
                ],
                "output":"sink"
            }))
            .unwrap();
            if explicit_ids {
                patch.assign_missing_cable_ids().unwrap();
            }
            let mut original = patch.compile().unwrap();
            original.process_block(&mut [0.0; 379]);
            patch.cables[0].color = Some([255, 0, 128]);
            patch.cables[1].color = Some([0, 255, 64]);
            let mut updated = patch.compile().unwrap();
            updated.adopt_state_from(&original);
            let mut expected = [0.0; 257];
            let mut actual = [0.0; 257];
            original.process_block(&mut expected);
            updated.process_block(&mut actual);
            assert_eq!(
                actual, expected,
                "version={version}, explicit_ids={explicit_ids}"
            );
        }
    }
}
