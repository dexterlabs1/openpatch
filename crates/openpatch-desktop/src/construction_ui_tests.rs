use super::*;

struct Host {
    context: egui::Context,
    panel: Panel,
    time: f64,
}

impl Host {
    fn new(snapshot: Value) -> Self {
        let mut panel = Panel::default();
        panel.observe(snapshot);
        Self {
            context: egui::Context::default(),
            panel,
            time: 0.0,
        }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> (Option<Action>, egui::FullOutput) {
        self.time += 0.05;
        let mut action = None;
        let output = self.context.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100.0, 220.0),
                )),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |context| {
                if let Some(emitted) = self.panel.show(context) {
                    action = Some(emitted);
                }
            },
        );
        (action, output)
    }

    fn click(&mut self, label: &str) -> Option<Action> {
        // Settle layout before locating the actual painted control. Coordinates
        // come from its visible label rather than duplicating panel layout.
        assert!(self.frame(vec![]).0.is_none());
        let (action, output) = self.frame(vec![]);
        assert!(action.is_none());
        let point = text_rect(&output, label)
            .unwrap_or_else(|| panic!("missing visible construction control {label}"))
            .center();
        assert!(self
            .frame(vec![egui::Event::PointerMoved(point)])
            .0
            .is_none());
        assert!(self
            .frame(vec![egui::Event::PointerButton {
                pos: point,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }])
            .0
            .is_none());
        self.frame(vec![egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }])
        .0
    }
}

fn text_rect(output: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
    fn find(shape: &egui::Shape, label: &str, clip: egui::Rect) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
                clip.contains(rect.center()).then_some(rect)
            }
            egui::Shape::Vec(children) => {
                children.iter().find_map(|shape| find(shape, label, clip))
            }
            _ => None,
        }
    }
    output
        .shapes
        .iter()
        .find_map(|clipped| find(&clipped.shape, label, clipped.clip_rect))
}

fn snapshot(status: &str, plan_id: u64) -> Value {
    json!({
        "plan_id": plan_id,
        "title": "Build a chord",
        "status": status,
        "completed_steps": 1,
        "total_steps": 3,
        "current_revision": 41,
        "expected_revision": 41,
        "last_revision": 41,
        "next_label": "Add the second oscillator",
        "last_label": "Add the first oscillator"
    })
}

fn assert_action(action: Option<Action>, name: &str, arguments: Value) {
    let action = action.unwrap_or_else(|| panic!("missing construction action {name}"));
    assert_eq!(action.name, name);
    assert_eq!(action.arguments, arguments);
}

#[test]
fn running_pointer_controls_emit_pause_and_cancel_for_the_observed_plan() {
    let mut host = Host::new(snapshot("running", 73));
    assert_action(
        host.click("Pause"),
        "pause_construction",
        json!({"plan_id": 73}),
    );
    // Requests do not optimistically change the observed status. The next
    // session observation owns that transition, so cancellation stays usable.
    assert_action(
        host.click("Cancel"),
        "cancel_construction",
        json!({"plan_id": 73}),
    );
}

#[test]
fn paused_resume_carries_the_observed_revision_and_stale_resume_is_disabled() {
    let mut host = Host::new(snapshot("paused", 91));
    assert_action(
        host.click("Resume"),
        "resume_construction",
        json!({"plan_id": 91, "expected_revision": 41}),
    );

    for current_revision in [json!(42), Value::Null] {
        let mut stale = snapshot("paused", 92);
        stale["current_revision"] = current_revision;
        host.panel.observe(stale);
        assert!(host.click("Resume").is_none());
        // An enabled neighbor proves the pointer sequence still operates; a
        // stale rack prevents resume, while cancellation remains available.
        assert_action(
            host.click("Cancel"),
            "cancel_construction",
            json!({"plan_id": 92}),
        );
    }

    let mut reviewed = snapshot("paused", 92);
    reviewed["current_revision"] = json!(42);
    reviewed["expected_revision"] = json!(42);
    host.panel.observe(reviewed);
    assert_action(
        host.click("Resume"),
        "resume_construction",
        json!({"plan_id": 92, "expected_revision": 42}),
    );
}

#[test]
fn terminal_dismissal_survives_observation_but_does_not_hide_a_new_plan() {
    for status in ["completed", "cancelled", "conflicted", "failed"] {
        let mut host = Host::new(snapshot(status, 100));
        assert!(host.click("Dismiss").is_none());
        let (action, hidden) = host.frame(vec![]);
        assert!(action.is_none());
        assert!(text_rect(&hidden, "Dismiss").is_none());
        assert!(text_rect(&hidden, "Build a chord").is_none());

        let mut same_plan = snapshot(status, 100);
        same_plan["current_revision"] = json!(43);
        host.panel.observe(same_plan);
        assert!(host.frame(vec![]).1.shapes.is_empty());

        host.panel.observe(snapshot("running", 101));
        assert_action(
            host.click("Pause"),
            "pause_construction",
            json!({"plan_id": 101}),
        );
    }
}

#[test]
fn idle_session_snapshot_draws_no_panel_and_emits_no_patch_work() {
    let workspace = tempfile::tempdir().unwrap();
    let session = openpatch::control::Session::new(workspace.path()).unwrap();
    let before = session.snapshot();
    let mut host = Host::new(session.construction_snapshot());
    for events in [
        vec![],
        vec![egui::Event::PointerMoved(egui::pos2(60.0, 15.0))],
        vec![egui::Event::PointerButton {
            pos: egui::pos2(60.0, 15.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        }],
        vec![egui::Event::PointerButton {
            pos: egui::pos2(60.0, 15.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        }],
    ] {
        let (action, output) = host.frame(events);
        assert!(
            action.is_none(),
            "idle UI must give the host no action to dispatch"
        );
        assert!(output.shapes.is_empty(), "idle UI must reserve no panel");
    }
    assert_eq!(session.snapshot(), before);
}

#[test]
fn motion_mode_controls_are_local_and_survive_plan_observation_and_dismissal() {
    use crate::construction_motion::MotionMode;
    let mut host = Host::new(snapshot("running", 211));
    assert_eq!(host.panel.motion_mode(), MotionMode::Full);
    for (label, mode) in [("Reduced", MotionMode::Reduced), ("Off", MotionMode::Off)] {
        assert!(host.click(label).is_none());
        assert_eq!(host.panel.motion_mode(), mode);
    }
    host.panel.observe(snapshot("paused", 211));
    assert_eq!(host.panel.motion_mode(), MotionMode::Off);
    host.panel.observe(snapshot("completed", 211));
    assert!(host.click("Dismiss").is_none());
    assert!(host.frame(vec![]).1.shapes.is_empty());
    host.panel.observe(snapshot("running", 212));
    assert_eq!(host.panel.motion_mode(), MotionMode::Off);
    assert!(host.click("Full").is_none());
    assert_eq!(host.panel.motion_mode(), MotionMode::Full);
}
