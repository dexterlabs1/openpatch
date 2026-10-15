use openpatch_core::{ModuleAction, Node, Patch, PortFrame};
use std::collections::BTreeMap;

fn patch(bypassed: bool) -> Patch {
    let mut patch = Patch::new_v2();
    patch.sample_rate = 8000;
    patch.nodes = vec![
        Node {
            content: None,
            score: None,
            id: "seq".into(),
            kind: "seq3".into(),
            bypassed,
            params: BTreeMap::new(),
        },
        Node {
            content: None,
            score: None,
            id: "other".into(),
            kind: "constant".into(),
            bypassed: false,
            params: BTreeMap::new(),
        },
    ];
    patch.output = Some("seq".into());
    patch.output_port = "run".into();
    patch
}

#[test]
fn state_load_rejects_foreign_and_other_models_but_accepts_bypassed_seq3() {
    let mut live = patch(false).compile().unwrap();
    let mut other_engine = patch(false).compile().unwrap();
    let seq = live.resolve_module_state("seq").unwrap();
    let other = live.resolve_module_state("other").unwrap();
    assert!(!live.set_module_running(other, false));
    assert!(!other_engine.set_module_running(seq, false));
    assert!(live.set_module_running(seq, false));
    assert!(!live.module_telemetry(seq).unwrap().running);

    let mut bypassed = patch(true).compile().unwrap();
    let bypassed_seq = bypassed.resolve_module_state("seq").unwrap();
    assert!(bypassed.set_module_running(bypassed_seq, false));
    assert!(!bypassed.module_telemetry(bypassed_seq).unwrap().running);
    bypassed.process_poly_block(&mut [PortFrame::default(); 16]);
    assert!(!bypassed.module_telemetry(bypassed_seq).unwrap().running);

    let mut resumed = patch(false).compile().unwrap();
    resumed.adopt_state_from(&bypassed);
    let resumed_seq = resumed.resolve_module_state("seq").unwrap();
    assert!(!resumed.module_telemetry(resumed_seq).unwrap().running);
    assert!(resumed.set_module_running(resumed_seq, true));
    assert!(resumed.module_telemetry(resumed_seq).unwrap().running);
}

#[test]
fn state_load_changes_running_without_step_reset_or_run_pulse() {
    let mut engine = patch(false).compile().unwrap();
    let seq = engine.resolve_module_state("seq").unwrap();
    assert!(engine.handle_module_event(seq, ModuleAction::Clock));
    engine.process_poly_block(&mut [PortFrame::default(); 1]);
    assert_eq!(engine.module_telemetry(seq).unwrap().step, 2);

    assert!(engine.handle_module_event(seq, ModuleAction::Run));
    engine.process_poly_block(&mut [PortFrame::default(); 16]);
    assert!(!engine.module_telemetry(seq).unwrap().running);
    let step = engine.module_telemetry(seq).unwrap().step;
    assert!(engine.set_module_running(seq, true));
    assert_eq!(engine.module_telemetry(seq).unwrap().step, step);
    assert!(engine.module_telemetry(seq).unwrap().running);
    let mut frame = [PortFrame::default(); 1];
    engine.process_poly_block(&mut frame);
    assert_eq!(frame[0].values[0], 0.0);
    assert_eq!(engine.module_telemetry(seq).unwrap().step, step);
}
