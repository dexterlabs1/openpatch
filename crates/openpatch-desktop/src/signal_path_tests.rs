use super::*;

fn routing_fixture() -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"source","kind":"constant"},{"id":"cv","kind":"constant"},
        {"id":"amp","kind":"stock_vca"},{"id":"filter","kind":"gain"},
        {"id":"out","kind":"gain"},{"id":"branch","kind":"gain"},
        {"id":"other","kind":"gain"}],"cables":[
        {"id":"audio","from":"source","to":"amp","input":"signal"},
        {"id":"cv","from":"cv","to":"amp","input":"cv"},
        {"id":"selected","from":"amp","to":"filter","input":"signal"},
        {"id":"tail","from":"filter","to":"out","input":"signal"},
        {"id":"branch","from":"filter","to":"branch","input":"signal"},
        {"id":"unrelated","from":"source","to":"other","input":"signal"}
    ]}))
    .unwrap()
}
fn selected_path(patch: &Patch) -> SignalPath {
    let mut path = SignalPath {
        enabled: true,
        ..Default::default()
    };
    path.refresh(patch, &[]);
    path.selected = Some(patch.cables[2].clone());
    path.trace();
    path
}

#[test]
fn signal_path_follows_directed_predecessors_successors_and_cv_without_unrelated_side_chains() {
    let patch = routing_fixture();
    let path = selected_path(&patch);
    assert_eq!(path.highlighted, vec![true, true, true, true, true, false]);
    assert_eq!(
        path.modules,
        BTreeSet::from_iter(["source", "cv", "amp", "filter", "out", "branch"].map(String::from))
    );
    assert_eq!(path.dim(&patch.cables[5]), 0.14);
}

#[test]
fn signal_path_feedback_terminates_and_selection_reconciles_moves_deletion_and_source_routes() {
    let mut patch = routing_fixture();
    patch.cables.push(
        serde_json::from_value(json!({"id":"feedback","from":"out","to":"amp","input":"signal"}))
            .unwrap(),
    );
    let mut path = selected_path(&patch);
    assert!(path.highlighted[6]);
    patch.cables.pop();
    patch.cables[2].to = "branch".into();
    path.refresh(&patch, &[]);
    assert_eq!(path.selected.as_ref().unwrap().to, "branch");
    assert!(!path.highlighted[3]);
    let mut retained = patch.cables[3].clone();
    retained.from = "branch".into();
    path.refresh(&patch, &[retained.clone()]);
    assert!(path.contains(&retained));
    patch.cables.remove(2);
    path.refresh(&patch, &[retained]);
    assert!(path.selected.is_none());
    assert!(path.highlighted.iter().all(|v| !v));
}

fn inspect_rack() -> Rack {
    let mut rack = Rack::default();
    rack.restore_layout(&json!({"version":1,"zoom":0.7,"pan":[24,12],"positions":{
        "source":[0,0],"cv":[150,0],"amp":[300,0],"filter":[500,0],
        "out":[700,0],"branch":[850,0],"other":[1000,0]}}))
        .unwrap();
    rack
}

#[test]
fn signal_path_real_cable_click_highlights_and_esc_exits_without_graph_actions() {
    let patch = routing_fixture();
    let before = json!(patch);
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = inspect_rack();
    rack.set_signal_path_mode(true);
    let (_, output) = frame_output(&ctx, &mut rack, &patch, 0., vec![]);
    let mut capture = crate::headless_review::CpuCapture::default();
    capture.observe(&output);
    let curve = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::CubicBezier(curve) => Some(curve.points),
            _ => None,
        })
        .nth(2 * 3)
        .expect("third cable's first body layer");
    let at = cable_curve_prefix(curve, 0.55)[3];
    for (time, events) in [
        (0.1, vec![egui::Event::PointerMoved(at)]),
        (
            0.2,
            vec![egui::Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        ),
        (
            0.3,
            vec![egui::Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        ),
    ] {
        let (action, output) = frame_output(&ctx, &mut rack, &patch, time, events);
        capture.observe(&output);
        assert!(action.is_none());
    }
    assert_eq!(
        rack.signal_path
            .selected
            .as_ref()
            .and_then(|c| c.id.as_deref()),
        Some("selected")
    );
    assert_eq!(
        rack.signal_path.highlighted,
        vec![true, true, true, true, true, false]
    );
    let (_, traced) = frame_output(&ctx, &mut rack, &patch, 0.4, vec![]);
    assert!(traced.shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.job.text.contains("connections"))));
    if let Some(destination) = std::env::var_os("OPENPATCH_SIGNAL_PATH_REVIEW") {
        let destination = std::path::PathBuf::from(destination);
        std::fs::create_dir_all(&destination).unwrap();
        capture.save(
            &ctx,
            traced,
            vec2(1100., 700.),
            destination.join("routing-trace.png"),
        );
        rack.set_room_brightness(0.);
        let (_, dark) = frame_output(&ctx, &mut rack, &patch, 0.45, vec![]);
        capture.save(
            &ctx,
            dark,
            vec2(1100., 700.),
            destination.join("routing-trace-dark.png"),
        );
    }
    let (action, _) = frame_output(
        &ctx,
        &mut rack,
        &patch,
        0.5,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(action.is_none());
    assert!(!rack.signal_path_mode());
    assert_eq!(json!(patch), before);
}

#[test]
fn cable_gravity_never_pushes_sideways_outside_its_plug_span_at_any_tension() {
    for zoom in [0.25, 1., 4.] {
        for (a, b) in [
            (pos2(20., 0.), pos2(20., 240.)),
            (pos2(0., 0.), pos2(700., 40.)),
            (pos2(700., 40.), pos2(0., 0.)),
        ] {
            let a = (a.to_vec2() * zoom).to_pos2();
            let b = (b.to_vec2() * zoom).to_pos2();
            for fan in [-20., 0., 20.] {
                for tension in [0., 0.25, 0.5, 0.75, 1.] {
                    let curve = cable_curve_points(a, b, zoom, fan, tension);
                    for i in 0..=100 {
                        let t = i as f32 / 100.;
                        let point = cable_curve_prefix(curve, t)[3];
                        assert!(point.x >= a.x.min(b.x) - 0.001 && point.x <= a.x.max(b.x) + 0.001);
                        assert!(point.y >= (a.y + (b.y - a.y) * t) - 0.001);
                    }
                }
            }
        }
    }
}
