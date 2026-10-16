use openpatch_core::{ModuleParameterActions, Patch, PortFrame};
use serde_json::json;

fn patch() -> Patch {
    serde_json::from_value(json!({
        "version": 2,
        "sample_rate": 48000,
        "nodes": [
            {"id":"source","kind":"poly_constant","params":{"channels":1,"v1":1.25}},
            {"id":"mix","kind":"stock_vca_mix","params":{
                "mix_level":1.0,"level1":1.0,"level2":1.0,"level3":1.0,"level4":1.0,
                "channel_exp":0.0,"mix_exp":0.0}}
        ],
        "cables": [{"id":"signal","from":"source","output":"out","to":"mix","input":"in1"}],
        "output":"mix",
        "output_port":"out1"
    }))
    .unwrap()
}

#[test]
fn initialize_after_adoption_keeps_receiving_controls_and_meter_history() {
    let original = patch();
    let mut old = original.compile().unwrap();
    let mut buffer = [PortFrame::default(); 513];
    old.process_poly_block(&mut buffer);
    let old_handle = old.resolve_module_state("mix").unwrap();
    let old_state = old.stock_vca_mix_snapshot(old_handle).unwrap();
    assert_eq!(old_state.divider_clock, 1);
    assert_eq!(old_state.light_brightness[0], 1.0);

    let mut edited = original.clone();
    let node = edited
        .nodes
        .iter_mut()
        .find(|node| node.id == "mix")
        .unwrap();
    node.params.insert("mix_level".into(), -3.25);
    node.params.insert("level1".into(), 10.5);
    node.params.insert("channel_exp".into(), 1.0);
    node.params.insert("mix_exp".into(), 1.0);
    let mut receiving = edited.compile().unwrap();
    receiving.adopt_state_from(&old);
    let handle = receiving.resolve_module_state("mix").unwrap();
    let before = receiving.stock_vca_mix_snapshot(handle).unwrap();
    let lights = receiving.module_lights(handle).unwrap();
    let ports = receiving.host_module_snapshot(handle).unwrap();
    assert_eq!(&before.parameters[..2], &[-3.25, 10.5]);
    assert!(before.ch_exp && before.mix_exp);
    assert_eq!(before.meters, old_state.meters);
    assert_eq!(before.divider_clock, old_state.divider_clock);
    assert_eq!(before.light_brightness, old_state.light_brightness);

    assert!(receiving.initialize_module_state(handle));
    assert_eq!(receiving.stock_vca_mix_snapshot(handle).unwrap(), before);
    assert_eq!(receiving.module_lights(handle).unwrap(), lights);
    assert_eq!(receiving.host_module_snapshot(handle).unwrap(), ports);

    // Source Reset first changes the receiving Patch's controls. Its runtime
    // Initialize hook then preserves the adopted meter and port histories.
    let mut reset_patch = edited;
    let reset_node = reset_patch
        .nodes
        .iter_mut()
        .find(|node| node.id == "mix")
        .unwrap();
    ModuleParameterActions::new(reset_patch.sample_rate)
        .initialize(reset_node)
        .unwrap();
    let mut reset = reset_patch.compile().unwrap();
    reset.adopt_state_from(&receiving);
    let reset_handle = reset.resolve_module_state("mix").unwrap();
    assert!(reset.initialize_module_state(reset_handle));
    let reset_state = reset.stock_vca_mix_snapshot(reset_handle).unwrap();
    assert_eq!(reset_state.parameters, [1.0; 5]);
    assert!(!reset_state.ch_exp && !reset_state.mix_exp);
    assert_eq!(reset_state.meters, before.meters);
    assert_eq!(reset_state.divider_clock, before.divider_clock);
    assert_eq!(reset_state.light_brightness, before.light_brightness);
}
