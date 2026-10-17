use super::*;

fn visit(shape: &egui::Shape, texts: &mut Vec<(String, Rect, Color32)>) {
    match shape {
        egui::Shape::Text(text) => texts.push((text.galley.text().into(), Rect::from_min_size(text.pos, text.galley.size()), text.fallback_color)),
        egui::Shape::Vec(shapes) => for shape in shapes { visit(shape, texts); },
        _ => {}
    }
}

#[test]
fn viz_channels_geometry_and_unavailable_are_truthful() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{"id":"viz","kind":"poly_viz"}]})).unwrap();
    for channels in [None, Some(0), Some(1), Some(8), Some(16)] {
        rack.module_lights.clear();
        if let Some(channels) = channels {
            rack.module_lights.insert("viz".into(), ModuleLightState { count:16, input_channels:Some(channels), ..Default::default() });
        }
        let painted = frame_revision_state(&ctx, &mut rack, &patch, 1.0, vec![], true, true, 7).1;
        let mut texts = vec![];
        for shape in &painted.shapes { visit(&shape.shape, &mut texts); }
        let status = channels.map_or_else(|| "TELEMETRY UNAVAILABLE".into(), |c| format!("{c} / 16 CHANNELS"));
        assert!(texts.iter().any(|(s,_,_)| s == &status));
        let jack = fixture_jack(&rack, &patch, "viz", "signal", true);
        assert_eq!(jack, pos2(114.0,379.0));
        let hit = Rect::from_center_size(jack, vec2(36.0,36.0));
        for channel in 0..16 {
            let label = format!("{:02}",channel+1);
            let (_, bounds, color) = texts.iter().find(|(s,_,_)| s == &label).unwrap();
            assert!(!bounds.intersects(hit));
            assert_eq!(*color, if channels.is_some_and(|c| channel < c as usize) { INK } else { Color32::from_gray(155) });
        }
        for (label,bounds,_) in &texts {
            if label == &status || label == "+ GREEN   − RED" { assert!(!bounds.intersects(hit)); }
        }
        assert!(!Rect::from_min_max(pos2(44.0,118.0),pos2(182.0,314.0)).intersects(hit));
    }
    let module = openpatch_core::catalog()["modules"].as_array().unwrap().iter().find(|m|m["kind"]=="poly_viz").unwrap().clone();
    assert!(module["outputs"].as_array().unwrap().is_empty());
    assert!(module["params"].as_object().unwrap().is_empty());
    assert_eq!(panel_width("poly_viz"),180.0);
}

#[test]
fn viz_input_drag_is_independent_and_has_no_output_target() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{"id":"viz","kind":"poly_viz"},{"id":"source","kind":"constant"}]})).unwrap();
    frame(&ctx,&mut rack,&patch,0.0,vec![]);
    let source = fixture_jack(&rack,&patch,"source","out",false);
    let input = fixture_jack(&rack,&patch,"viz","signal",true);
    let action = drag(&ctx,&mut rack,&patch,1.0,source,input);
    assert!(matches!(action,Some(RackAction::Command("connect",ref args)) if args["from"]=="source" && args["to"]=="viz" && args["input"]=="signal"));
    assert!(rack.editing.is_none());
    assert!(!port_exists(&patch,&openpatch_core::catalog(),"viz","out",true));
}

#[test]
fn retired_viz_voice_keeps_its_release_tail_but_dims_number() {
    let ctx = egui::Context::default();
    let mut rack = Rack::default();
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[{"id":"viz","kind":"poly_viz"}]})).unwrap();
    let mut state = ModuleLightState { count:16, input_channels:Some(0), ..Default::default() };
    state.brightness[15] = [0.75, -0.5];
    rack.module_lights.insert("viz".into(),state);
    let painted = frame_revision_state(&ctx,&mut rack,&patch,1.0,vec![],true,true,7).1;
    let mut texts = vec![];
    for shape in &painted.shapes {
        visit(&shape.shape,&mut texts);
    }
    let lens = |output: &egui::FullOutput, center: Pos2| {
        output.shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle)
                if circle.center == center
                    && circle.radius == crate::lighting::lamp_radius(3.0) => Some(circle.fill),
            _ => None,
        }).expect("voice lens")
    };
    let tail = lens(&painted, pos2(157.0,300.0));
    assert!(tail.g() > tail.r() && tail.g() > tail.b(), "positive release tail stays green");
    assert_eq!(texts.iter().find(|(label,_,_)|label=="16").unwrap().2,Color32::from_gray(155));
    state.brightness[15] = [0.0; 2];
    rack.module_lights.insert("viz".into(),state);
    let zero = frame_revision_state(&ctx,&mut rack,&patch,1.1,vec![],true,true,7).1;
    assert!(tail.g() > lens(&zero, pos2(157.0,300.0)).g(), "the tail requires measured voltage");
    assert_eq!(lens(&painted, pos2(171.0,300.0)), lens(&zero, pos2(171.0,300.0)), "negative brightness must not invent a red tail");
}
