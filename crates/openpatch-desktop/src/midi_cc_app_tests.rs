use super::*;
use openpatch::control::LiveModuleEvents;
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

fn cc_host(path: &Path) -> OpenPatch {
    let mut app = host(path);
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,
        "nodes":[{"id":"cc","kind":"midi_cc"}]}})
    ));
    app
}
fn labels(output: &egui::FullOutput) -> Vec<String> {
    fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => out.push(text.galley.text().into()),
            egui::Shape::Vec(children) => {
                for child in children {
                    collect(child, out);
                }
            }
            _ => {}
        }
    }
    let mut result = vec![];
    for shape in &output.shapes {
        collect(&shape.shape, &mut result);
    }
    result
}
fn panel_labels(app: &mut OpenPatch) -> Vec<String> {
    let ctx = egui::Context::default();
    labels(&ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 700.0),
            )),
            focused: true,
            ..Default::default()
        },
        |ctx| app.rack_workspace(ctx, None),
    ))
}
#[test]
fn midi_cc_app_backend_attachment_retries_after_processing_without_losing_old_backend() {
    struct ProcessingBackend {
        processing: AtomicBool,
        preparations: AtomicUsize,
    }
    impl LiveModuleEvents for ProcessingBackend {
        fn trigger(&self, _: &str, _: openpatch_core::ModuleAction) -> Result<Value, String> {
            Ok(json!({}))
        }
        fn snapshot(&self) -> Value {
            json!({"old_backend":true})
        }
        fn prepare_midi_cc_edit(&self, ids: &[String]) -> Result<(), String> {
            assert_eq!(ids, ["cc"]);
            self.preparations.fetch_add(1, Ordering::SeqCst);
            if self.processing.load(Ordering::SeqCst) {
                Err("MIDI CC Learn is Processing; retry".into())
            } else {
                Ok(())
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut app = cc_host(dir.path());
    let backend = Arc::new(ProcessingBackend {
        processing: AtomicBool::new(true),
        preparations: AtomicUsize::new(0),
    });
    app.shared
        .lock()
        .unwrap()
        .set_live_module_events(Some(backend.clone()))
        .unwrap();
    let before = app.shared.lock().unwrap().snapshot();
    app.attach_live_events();
    assert!(app.live_events_attach_pending);
    assert!(app.settings_status.contains("Processing"));
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert_eq!(
        execute_shared(&app.shared, "inspect_performance", json!({})).unwrap()["performance"]
            ["old_backend"],
        true
    );
    backend.processing.store(false, Ordering::SeqCst);
    app.sync();
    assert!(!app.live_events_attach_pending);
    assert!(!app.settings_status.contains("reconnecting"));
    assert_eq!(backend.preparations.load(Ordering::SeqCst), 2);
    assert_eq!(
        execute_shared(&app.shared, "inspect_performance", json!({})).unwrap()["supported"],
        false
    );
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
}
#[test]
fn midi_cc_app_backend_attachment_defers_a_busy_session_lock_and_retries_at_sync() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = cc_host(dir.path());
    let shared = app.shared.clone();
    let guard = shared.lock().unwrap();
    app.attach_live_events();
    assert!(app.live_events_attach_pending);
    drop(guard);
    app.sync();
    assert!(!app.live_events_attach_pending);
}
#[test]
fn midi_cc_app_offline_snapshots_survive_smoke_and_clear_after_live_audio_disappears() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = cc_host(dir.path());
    let mut values = vec![0; 128];
    values[1] = 77;
    app.rack.set_midi_cc_states(BTreeMap::from([(
        "cc".into(),
        json!({
        "assignments":(1..=16).collect::<Vec<_>>(),"values":values,
        "learning":{"cell":16,"serial":42}}),
    )]));
    app.smoke_test = Some((dir.path().join("unused.png"), 0, 0));
    app.sync_midi_cc_states();
    let before = panel_labels(&mut app);
    assert!(before.iter().any(|s| s == "VALUE 77"));
    assert!(before.iter().any(|s| s == "LRN"));
    app.smoke_test = None;
    app.sync_midi_cc_states();
    let after = panel_labels(&mut app);
    assert!(!after.iter().any(|s| s.starts_with("VALUE ") || s == "LRN"));
}
#[test]
fn midi_cc_app_learn_requests_report_requests_and_do_not_guess_learn_state_or_edit_history() {
    struct LearnBackend {
        cell: AtomicUsize,
        cancellations: AtomicUsize,
    }
    impl LiveModuleEvents for LearnBackend {
        fn trigger(&self, _: &str, _: openpatch_core::ModuleAction) -> Result<Value, String> {
            Ok(json!({}))
        }
        fn snapshot(&self) -> Value {
            json!({})
        }
        fn learn_midi_cc(&self, id: &str, cell: u8) -> Result<Value, String> {
            assert_eq!(id, "cc");
            self.cell.store(usize::from(cell), Ordering::SeqCst);
            Ok(json!({"requested":true}))
        }
        fn cancel_midi_cc_learn(&self, id: &str) -> Result<Value, String> {
            assert_eq!(id, "cc");
            self.cancellations.fetch_add(1, Ordering::SeqCst);
            Ok(json!({"requested":true}))
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut app = cc_host(dir.path());
    let backend = Arc::new(LearnBackend {
        cell: AtomicUsize::new(99),
        cancellations: AtomicUsize::new(0),
    });
    app.shared
        .lock()
        .unwrap()
        .set_live_module_events(Some(backend.clone()))
        .unwrap();
    let before = app.shared.lock().unwrap().snapshot();
    assert!(app.command("learn_midi_cc", json!({"id":"cc","cell":16})));
    assert_eq!(backend.cell.load(Ordering::SeqCst), 15);
    assert!(app.status.contains("requested"));
    assert!(!panel_labels(&mut app).iter().any(|s| s == "LRN"));
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert!(app.command("cancel_midi_cc_learn", json!({"id":"cc"})));
    assert_eq!(backend.cancellations.load(Ordering::SeqCst), 1);
    assert!(app.status.contains("cancellation requested"));
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
}
#[test]
fn midi_cc_app_preset_inspector_omits_all_hidden_controller_values() {
    let catalog = catalog();
    let mut params = serde_json::Map::new();
    params.insert("cc1".into(), json!(127));
    for i in 0..128 {
        params.insert(format!("cc_value{i}"), json!(i));
    }
    let preset = json!({"kind":"midi_cc","params":params});
    let ctx = egui::Context::default();
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| preset_controls(ui, &preset, &catalog));
    });
    let rendered = labels(&output);
    assert!(rendered.iter().any(|s| s == "cc1"));
    assert!(!rendered.iter().any(|s| s.starts_with("cc_value")));
}
