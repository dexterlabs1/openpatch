use super::*;
use openpatch_core::PortDirection;

fn fixture() -> (Patch, Arc<crate::audio::PortLightFrame>) {
    let patch: Patch = serde_json::from_value(json!({"version":2,"nodes":[
        {"id":"p","kind":"constant","params":{"value":5}},
        {"id":"n","kind":"constant","params":{"value":-5}},
        {"id":"sum","kind":"gain"},{"id":"other","kind":"gain"}],
        "cables":[{"id":"a","from":"p","to":"sum","input":"signal","color":[100,90,200]},
            {"id":"b","from":"n","to":"sum","input":"signal","color":[240,120,30]},
            {"id":"c","from":"p","to":"other","input":"signal","color":[80,180,180]}],"output":"sum"})).unwrap();
    let mut engine = patch.compile().unwrap();
    engine.set_port_lights_enabled(true);
    engine.process_block(&mut [0.0; 64]);
    let frame = crate::audio::offline_frame(&patch, &engine, 0).unwrap();
    (patch, frame)
}
fn rack() -> Rack {
    let mut rack = Rack::default();
    rack.restore_layout(&json!({"version":1,"zoom":1,"pan":[24,12],"positions":{
        "p":[0,0],"n":[150,0],"sum":[300,0],"other":[480,0]}}))
        .unwrap();
    rack
}
fn emissions(output: &egui::FullOutput) -> Vec<(Pos2, Color32)> {
    output
        .shapes
        .iter()
        .filter_map(|shape| match shape.shape {
            egui::Shape::Circle(ref circle)
                if circle.fill.a() == 0
                    && circle.fill != Color32::TRANSPARENT
                    && (circle.radius - 2.0).abs() < 0.001 =>
            {
                Some((circle.center, circle.fill))
            }
            _ => None,
        })
        .collect()
}
fn without_emissions(output: &egui::FullOutput) -> Vec<String> {
    output.shapes.iter().filter(|shape| !matches!(shape.shape,
        egui::Shape::Circle(ref circle) if circle.fill.a() == 0 && circle.fill != Color32::TRANSPARENT
            && (circle.radius - 2.0).abs() < 0.001)).map(|shape| format!("{shape:?}")).collect()
}

#[test]
fn port_lights_paint_only_top_plugs_with_own_actual_endpoint_signal_and_preserve_cable_art() {
    let (patch, snapshot) = fixture();
    let before = serde_json::to_value(&patch).unwrap();
    let ctx = egui::Context::default();
    let mut rack = rack();
    frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
    let passive = frame_output(&ctx, &mut rack, &patch, 0.1, vec![]).1;
    assert!(emissions(&passive).is_empty());
    rack.set_port_lights(Some(snapshot));
    let (action, active) = frame_output(&ctx, &mut rack, &patch, 0.2, vec![]);
    assert!(action.is_none());
    let lenses = emissions(&active);
    assert_eq!(
        lenses.len(),
        3,
        "fanout source has one top light; stacked cancellation stays dark"
    );
    assert_eq!(lenses.iter().filter(|(_, c)| c.g() > c.r()).count(), 2);
    assert_eq!(lenses.iter().filter(|(_, c)| c.r() > c.g()).count(), 1);
    assert_eq!(
        without_emissions(&passive),
        without_emissions(&active),
        "all cable colors, physical caps, clips and order are unchanged"
    );
    assert_eq!(serde_json::to_value(&patch).unwrap(), before);
    rack.set_room_brightness(0.0);
    let dark = frame_output(&ctx, &mut rack, &patch, 0.3, vec![]).1;
    assert_eq!(
        emissions(&dark),
        lenses,
        "emission survives zero ambient brightness"
    );
    let stale = frame_revision_state(&ctx, &mut rack, &patch, 0.4, vec![], true, true, 1).1;
    assert!(emissions(&stale).is_empty());
}

#[test]
fn port_lights_bundle_move_preserves_attached_anchors_but_never_free_ends_or_ghosts() {
    let (patch, snapshot) = fixture();
    let ctx = egui::Context::default();
    let mut rack = rack();
    rack.set_port_lights(Some(snapshot));
    frame_output(&ctx, &mut rack, &patch, 0.0, vec![]);
    let mut pending = PendingCable::new(
        CableEnd::Input {
            node: "sum".into(),
            port: "signal".into(),
        },
        &patch,
    );
    pending.original = Some(patch.cables[0].clone());
    pending.companions = vec![patch.cables[1].clone()];
    rack.pending = Some(pending);
    let pointer = pos2(850.0, 510.0);
    let output = frame_output(
        &ctx,
        &mut rack,
        &patch,
        0.1,
        vec![egui::Event::PointerMoved(pointer)],
    )
    .1;
    let lenses = emissions(&output);
    assert_eq!(
        lenses.len(),
        3,
        "two attached outputs and untouched sink remain lit"
    );
    assert!(lenses
        .iter()
        .all(|(point, _)| point.distance(pointer) > 10.0));
    assert_eq!(
        lenses.iter().filter(|(_, c)| c.r() > c.g()).count(),
        1,
        "companion anchor retains negative source"
    );
    assert!(rack
        .port_signal_state("sum", PortDirection::Input, "signal")
        .unwrap()
        .raw_rgb
        .iter()
        .all(|v| *v == 0.0));
    rack.set_port_lights(None);
    let offline = frame_output(&ctx, &mut rack, &patch, 0.2, vec![]).1;
    assert!(emissions(&offline).is_empty());
}

#[test]
fn port_light_emission_keeps_original_paint_order_under_later_cap_occlusion() {
    let (_, snapshot) = fixture();
    let ctx = egui::Context::default();
    let mut rack = rack();
    rack.set_port_lights(Some(snapshot));
    rack.set_room_brightness(0.0);
    let center = pos2(100.0, 100.0);
    let output = ctx.run(egui::RawInput::default(), |ctx| {
        let painter = ctx.layer_painter(egui::LayerId::background());
        let begin = rack.lighting.begin(&painter);
        paint_cable_plug(&painter, center, CABLES[0], 1.0);
        rack.paint_port_signal(&painter, center, "p", PortDirection::Output, "out");
        paint_cable_plug(&painter, center, CABLES[1], 1.0);
        rack.lighting.finish(&painter, begin);
    });
    let light = output
        .shapes
        .iter()
        .position(|s| {
            matches!(s.shape, egui::Shape::Circle(ref c)
        if c.fill.a() == 0 && c.fill.g() > 50 && c.radius == 2.0)
        })
        .unwrap();
    let top_lens = output
        .shapes
        .iter()
        .rposition(|s| {
            matches!(s.shape, egui::Shape::Circle(ref c)
        if c.radius == 2.0 && c.fill == Color32::BLACK)
        })
        .unwrap();
    assert!(
        light < top_lens,
        "later passive cap covers earlier protected emission"
    );
}
