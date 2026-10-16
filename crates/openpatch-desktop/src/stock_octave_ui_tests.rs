use super::*;

fn patch(octaves: f32) -> Patch {
    serde_json::from_value(json!({"version":2,"nodes":[{"id":"oct","kind":"stock_octave","params":{"octaves":octaves}}]})).unwrap()
}

fn at(local: Pos2) -> Pos2 {
    local + vec2(24.0, 12.0)
}

fn frame(ctx: &egui::Context, rack: &mut Rack, patch: &Patch, time: f64, events: Vec<egui::Event>, revision: u64) -> (Option<RackAction>, egui::FullOutput) {
    let mut action = None;
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0))),
            time: Some(time),
            focused: true,
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                action = rack.show(ui, patch, &openpatch_core::catalog(), 48000, revision);
            });
        },
    );
    (action, output)
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}

#[test]
fn stock_octave_selector_click_is_revision_fenced_and_preserves_nine_positions() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch(0.0);
    frame(&ctx, &mut rack, &patch, 0.0, vec![], 41);
    for value in -4..=4 {
        let pos = at(stock_octave_selector_rect(value).center());
        frame(&ctx, &mut rack, &patch, 1.0 + f64::from(value + 4), vec![egui::Event::PointerMoved(pos), button(pos, true)], 41);
        let (action, _) = frame(&ctx, &mut rack, &patch, 1.1 + f64::from(value + 4), vec![button(pos, false)], 41);
        let Some(RackAction::Command("set_parameter", args)) = action else { panic!("detent {value}") };
        assert_eq!(args["id"], "oct");
        assert_eq!(args["parameter"], "octaves");
        assert_eq!(args["value"], value);
        assert_eq!(args["expected_revision"], 41);
    }
}

#[test]
fn stock_octave_keyboard_adjustment_clamps_at_endpoints() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch(4.0);
    let pos = at(stock_octave_selector_rect(4).center());
    frame(&ctx, &mut rack, &patch, 0.0, vec![], 8);
    frame(&ctx, &mut rack, &patch, 0.0, vec![egui::Event::PointerMoved(pos), button(pos, true)], 8);
    let (click, _) = frame(&ctx, &mut rack, &patch, 0.1, vec![button(pos, false)], 8);
    assert!(matches!(click, Some(RackAction::Command("set_parameter", _))), "click must focus a real selector before keyboard editing");
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(egui::Id::new(("stock-octave", "oct", 4))));
    let (action, _) = frame(&ctx, &mut rack, &patch, 0.2, vec![key(egui::Key::ArrowRight)], 8);
    assert!(action.is_none(), "the selector cannot move past +4");
}

#[test]
fn stock_octave_focused_arrows_clamp_extreme_saved_values_before_adding() {
    for (raw, button_value, arrow, expected) in [
        (f32::MAX, 4, egui::Key::ArrowLeft, 3),
        (f32::MIN, -4, egui::Key::ArrowRight, -3),
    ] {
        let ctx = egui::Context::default();
        let mut rack = Rack::default();
        let saved = patch(raw);
        let pos = at(stock_octave_selector_rect(button_value).center());
        frame(&ctx, &mut rack, &saved, 0.0, vec![], 9);
        frame(&ctx, &mut rack, &saved, 0.0, vec![egui::Event::PointerMoved(pos), button(pos, true)], 9);
        let (click, _) = frame(&ctx, &mut rack, &saved, 0.1, vec![button(pos, false)], 9);
        assert!(matches!(click, Some(RackAction::Command("set_parameter", _))), "click must focus a real selector before keyboard editing");
        assert_eq!(ctx.memory(|memory| memory.focused()), Some(egui::Id::new(("stock-octave", "oct", button_value))));
        let (action, _) = frame(&ctx, &mut rack, &saved, 0.2, vec![key(arrow)], 9);
        let Some(RackAction::Command("set_parameter", args)) = action else { panic!("focused extreme value is clamped safely") };
        assert_eq!(args["value"], expected);
        assert_eq!(args["expected_revision"], 9);
    }
}

#[test]
fn stock_octave_keyboard_steps_a_focused_detent_and_keeps_the_revision() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let initial = patch(2.0);
    let pos = at(stock_octave_selector_rect(2).center());
    frame(&ctx, &mut rack, &initial, 0.0, vec![], 12);
    frame(&ctx, &mut rack, &initial, 0.0, vec![egui::Event::PointerMoved(pos), button(pos, true)], 12);
    let (click, _) = frame(&ctx, &mut rack, &initial, 0.1, vec![button(pos, false)], 12);
    assert!(matches!(click, Some(RackAction::Command("set_parameter", _))), "click must focus a real selector before keyboard editing");
    assert_eq!(ctx.memory(|memory| memory.focused()), Some(egui::Id::new(("stock-octave", "oct", 2))));
    let (action, _) = frame(&ctx, &mut rack, &initial, 0.2, vec![key(egui::Key::ArrowRight)], 12);
    let Some(RackAction::Command("set_parameter", args)) = action else { panic!("keyboard detent") };
    assert_eq!(args["value"], 3);
    assert_eq!(args["expected_revision"], 12);
    let committed = patch(3.0);
    let (action, _) = frame(&ctx, &mut rack, &committed, 0.3, vec![key(egui::Key::ArrowRight)], 13);
    let Some(RackAction::Command("set_parameter", args)) = action else { panic!("repeated arrow uses committed value") };
    assert_eq!(args["value"], 4);
    assert_eq!(args["expected_revision"], 13);
    rack.set_shortcuts_enabled(false);
    let (action, _) = frame(&ctx, &mut rack, &committed, 0.4, vec![key(egui::Key::ArrowLeft)], 13);
    assert!(action.is_none(), "host-disabled shortcuts do not edit the parameter");
}

#[test]
fn stock_octave_numeric_editor_is_finite_and_bounded_and_out_of_range_stays_visible() {
    let patch = patch(6.25);
    let edit = stock_octave_edit(&patch.nodes[0], 7);
    assert_eq!(edit.min, -4.0);
    assert_eq!(edit.max, 4.0);
    assert_eq!(edit.text, "6.25");
    assert!(stock_octave_authored_label(f32::MAX).len() < 20);
    assert!(stock_octave_authored_label(i32::MIN as f32).len() < 20);
    for (text, accepted) in [("-4", true), ("4", true), ("2.5", true), ("4.001", false), ("-4.001", false), ("nan", false), ("inf", false)] {
        let mut draft = edit.clone();
        draft.text = text.into();
        assert_eq!(parsed_parameter_result(&draft).is_ok(), accepted, "{text}");
    }
}

#[test]
fn stock_octave_panel_separates_saved_setting_from_actual_cv_and_unavailable_state() {
    fn texts(output: &egui::FullOutput) -> String {
        fn visit(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Vec(shapes) => for shape in shapes { visit(shape, out); },
                egui::Shape::Text(text) => { out.push_str(text.galley.text()); out.push('\n'); },
                _ => {}
            }
        }
        let mut out = String::new();
        for shape in &output.shapes { visit(&shape.shape, &mut out); }
        out
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch(2.0);
    let (_, unavailable) = frame(&ctx, &mut rack, &patch, 0.0, vec![], 3);
    let unavailable = texts(&unavailable);
    assert!(unavailable.contains("SAVED AUTHORED VALUE"));
    assert!(unavailable.contains("ACTUAL CH 0 EFFECTIVE OFFSET"));
    assert!(unavailable.contains("TELEMETRY UNAVAILABLE"));
    assert!(!unavailable.contains("+0 oct"), "unavailable channel telemetry is never shown as a fabricated zero");
    rack.module_lights.insert("oct".into(), ModuleLightState { octave_shift: Some(-3), ..Default::default() });
    let (_, observed) = frame(&ctx, &mut rack, &patch, 0.1, vec![], 3);
    let observed = texts(&observed);
    assert!(observed.contains("+2.000 oct"));
    assert!(observed.contains("-3 oct"));
    assert!(observed.contains("FROZEN IN BYPASS"));
}

#[test]
fn stock_octave_authored_selector_and_actual_readout_remain_emissive_in_darkness() {
    fn selected_button(output: &egui::FullOutput) -> (Rect, Color32) {
        let target = stock_octave_selector_rect(-2).translate(vec2(24.0, 12.0));
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Rect(rect) if rect.rect == target => Some((rect.rect, rect.fill)),
            _ => None,
        }).expect("selected authored button geometry")
    }
    fn text_color(output: &egui::FullOutput, label: &str) -> Color32 {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.override_text_color.unwrap_or(text.fallback_color))
            }
            _ => None,
        }).unwrap_or_else(|| panic!("missing text {label:?}"))
    }
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch = patch(-2.0);
    rack.module_lights.insert("oct".into(), ModuleLightState { octave_shift: Some(3), ..Default::default() });
    rack.set_room_brightness(1.0);
    let (_, lit) = frame(&ctx, &mut rack, &patch, 0.0, vec![], 4);
    rack.set_room_brightness(0.0);
    let (_, dark) = frame(&ctx, &mut rack, &patch, 0.1, vec![], 4);
    let (lit_geometry, lit_fill) = selected_button(&lit);
    let (dark_geometry, dark_fill) = selected_button(&dark);
    assert_eq!(lit_geometry, dark_geometry, "authored selector geometry remains fixed");
    assert_eq!(lit_fill, dark_fill, "the selected display background is stable across lighting modes");
    assert_eq!(dark_fill, Color32::from_rgb(40, 82, 63), "selected button keeps its dark display background");
    assert_eq!(text_color(&lit, "-2"), Color32::from_rgb(177, 239, 203));
    assert_eq!(text_color(&dark, "-2"), Color32::from_rgb(177, 239, 203), "selected text remains readable without changing galley layout");
    assert!(dark.shapes.iter().any(|shape| match &shape.shape {
        egui::Shape::Rect(rect) => rect.fill == Color32::from_rgb(114, 213, 157) && dark_geometry.contains(rect.rect.center()),
        _ => false,
    }), "the restrained authored-state indicator remains emissive");
    assert_eq!(text_color(&lit, "+3 oct"), text_color(&dark, "+3 oct"), "actual offset remains emissive");
    assert_ne!(text_color(&lit, "SAVED AUTHORED VALUE"), text_color(&dark, "SAVED AUTHORED VALUE"), "ordinary panel text still dims");
    let mut dark_text = String::new();
    for shape in &dark.shapes {
        if let egui::Shape::Text(text) = &shape.shape {
            dark_text.push_str(text.galley.text());
            dark_text.push('\n');
        }
    }
    assert!(dark_text.contains("-2.000 oct"));
    assert!(dark_text.contains("+3 oct"));
}

#[test]
fn stock_octave_shared_rack_and_browser_face_match_at_supported_scales() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"].as_array().unwrap().iter().find(|m| m["kind"] == "stock_octave").unwrap();
    fn rendered(browser: bool, scale: f32, module: &Value, catalog: &Value) -> Vec<String> {
        let size = vec2(panel_width("stock_octave"), PANEL_HEIGHT);
        let bounds = Rect::from_min_size(pos2(24.0, 12.0), size * scale);
        let zoom = (bounds.width() / size.x).min(bounds.height() / size.y);
        let fitted = Rect::from_center_size(bounds.center(), size * zoom);
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let patch = patch(1.5);
        let mut rack = Rack { zoom, pan: Vec2::ZERO, ..Default::default() };
        rack.positions.insert("oct".into(), Pos2::ZERO);
        let mut preview = ModulePreview::new(module);
        let mut output = egui::FullOutput::default();
        for _ in 0..2 {
            output = ctx.run(egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1400.0, 900.0))),
                ..Default::default()
            }, |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    if browser {
                        preview.show(ui, bounds, catalog);
                    } else {
                        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(fitted));
                        child.set_clip_rect(ui.clip_rect().intersect(bounds));
                        let result = rack.render_modules(&mut child, &patch, catalog, fitted, 48000, 0, RenderMode::Interactive { drag_interactive: false, allow_manual_press: false });
                        assert!(result.action.is_none());
                    }
                });
            });
        }
        fn visit(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(shapes) => for shape in shapes { visit(shape, out); },
                egui::Shape::Text(text) if !text.galley.text().is_empty() => out.push(text.galley.text().to_string()),
                _ => {}
            }
        }
        let mut out = Vec::new();
        for shape in &output.shapes { visit(&shape.shape, &mut out); }
        out
    }
    for scale in [0.25, 0.42, 0.7, 1.0, 1.75] {
        let rack = rendered(false, scale, module, &catalog);
        let browser = rendered(true, scale, module, &catalog);
        for label in ["STOCK OCTAVE", "AUTHORED SHIFT", "SAVED AUTHORED VALUE", "ACTUAL CH 0 EFFECTIVE OFFSET", "TELEMETRY UNAVAILABLE", "+0"] {
            assert!(rack.iter().any(|text| text == label), "rack missing {label} at scale {scale}");
            assert!(browser.iter().any(|text| text == label), "browser missing {label} at scale {scale}");
        }
        for value in -4..=4 {
            assert!(stock_octave_selector_rect(value).intersect(Rect::from_min_size(Pos2::ZERO, vec2(panel_width("stock_octave"), PANEL_HEIGHT))).is_positive());
        }
    }
}

#[test]
fn stock_octave_mcp_parameter_change_cue_anchors_the_authored_selector() {
    use std::time::{Duration, Instant};
    use crate::construction_motion::MotionMode;

    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let before = patch(0.0);
    let after = patch(3.0);
    frame_revision_state(&ctx, &mut rack, &before, 0.0, vec![], true, true, 9);
    let receipt = json!({"plan_id":13,"completed_steps":1,"last_revision":10,"current_revision":10,
        "last_commit":{"age_ms":0,"invalidate_existing_cables":false,"plan_id":13,"step_index":0,
        "before_revision":9,"revision":10,"label":"Set octave transposition","new_modules":[],"new_cables":[],
        "parameter_changes":[{"id":"oct","kind":"stock_octave","parameter":"octaves","old_effective":0.0,"new_effective":3.0}]}});
    let now = Instant::now();
    rack.observe_construction_transition_at(&before, &after, 9, 10, &receipt, MotionMode::Full, now);
    rack.set_construction_clock_override(Some(now + Duration::from_millis(160)));
    let (action, _) = frame_revision_state(&ctx, &mut rack, &after, 0.1, vec![], true, true, 10);
    assert!(action.is_none());
    let anchor = rack.construction_anchors.get(&("oct".into(), "octaves".into())).unwrap();
    for value in -4..=4 {
        assert!(anchor.rect.contains(at(stock_octave_selector_rect(value).center())));
    }
    assert_eq!((anchor.old_text.as_str(), anchor.new_text.as_str()), ("+0.000 oct", "+3.000 oct"));
}
