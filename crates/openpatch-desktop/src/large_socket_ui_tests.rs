use super::*;

const KINDS: [&str; 22] = [
    "analog_vco", "ladder_vcf", "fade", "compare", "process", "rescale", "gate",
    "attenuverter", "white_noise", "oscillator", "multi_vco", "multi_lfo",
    "wavetable_vco", "wavetable_lfo", "stock_vcf", "stock_vca",
    "lfo", "lowpass", "gain", "analog_vca", "adsr", "svf_filter",
];

#[derive(Clone, Copy)]
struct SocketSpec {
    name: &'static str,
    label: &'static str,
    output: bool,
    x: f32,
    y: f32,
}

impl SocketSpec {
    const fn input(name: &'static str, label: &'static str, x: f32, y: f32) -> Self {
        Self { name, label, output: false, x, y }
    }

    const fn output(name: &'static str, label: &'static str, x: f32, y: f32) -> Self {
        Self { name, label, output: true, x, y }
    }

    fn end(self) -> CableEnd {
        if self.output {
            CableEnd::Output { node: "subject".into(), port: self.name.into() }
        } else {
            CableEnd::Input { node: "subject".into(), port: self.name.into() }
        }
    }
}

// Independent saved-panel port contracts. The new presentation may move a
// caption, but must never move a cable endpoint, rename a port, or widen a face.
fn contract(kind: &str) -> (f32, Vec<SocketSpec>) {
    use SocketSpec as S;
    match kind {
        "lfo" => (180.0, vec![S::input("fm", "FM", 90.0, 332.0), S::output("out", "OUT", 90.0, 374.0)]),
        "lowpass" => (180.0, vec![S::input("signal", "IN", 90.0, 332.0), S::output("out", "OUT", 90.0, 374.0)]),
        "gain" | "analog_vca" => (150.0, vec![S::input("signal", "IN", 50.0, 332.0), S::input("cv", "CV", 100.0, 332.0), S::output("out", "OUT", 75.0, 374.0)]),
        "adsr" => (240.0, vec![S::input("gate", "GATE", 80.0, 332.0), S::input("retrigger", "RETRIG", 160.0, 332.0), S::output("out", "OUT", 120.0, 374.0)]),
        "svf_filter" => (270.0, vec![S::input("signal", "IN", 90.0, 332.0), S::input("cutoff_cv", "V/OCT", 180.0, 332.0), S::output("low", "LOW", 67.5, 374.0), S::output("band", "BAND", 135.0, 374.0), S::output("high", "HIGH", 202.5, 374.0)]),
        "analog_vco" => (210.0, vec![
            S::input("pitch", "V/OCT", 70.0, 332.0),
            S::input("fm", "FM", 140.0, 332.0),
            S::output("out", "OUT", 105.0, 374.0),
        ]),
        "ladder_vcf" => (210.0, vec![
            S::input("signal", "IN", 70.0, 332.0),
            S::input("cutoff_cv", "V/OCT", 140.0, 332.0),
            S::output("out", "OUT", 105.0, 374.0),
        ]),
        "fade" => (240.0, vec![
            S::input("a", "A", 45.0, 330.0),
            S::input("b", "B", 120.0, 330.0),
            S::input("cv", "CV", 195.0, 330.0),
            S::output("out", "OUT", 75.0, 374.0),
            S::output("complement", "COMP", 165.0, 374.0),
        ]),
        "compare" => (240.0, vec![
            S::input("a", "A", 60.0, 214.0),
            S::input("b", "B", 180.0, 214.0),
            S::output("max", "MAX", 30.0, 285.0),
            S::output("min", "MIN", 90.0, 285.0),
            S::output("clip", "CLIP", 150.0, 285.0),
            S::output("lim", "LIM", 210.0, 285.0),
            S::output("clip_gate", "CLIP G", 30.0, 361.0),
            S::output("lim_gate", "LIM G", 90.0, 361.0),
            S::output("greater", "A > B", 150.0, 361.0),
            S::output("less", "A < B", 210.0, 361.0),
        ]),
        "process" => (240.0, vec![
            S::input("slew_cv", "SLEW CV", 60.0, 226.0),
            S::input("signal", "IN", 120.0, 226.0),
            S::input("gate", "GATE", 180.0, 226.0),
            S::output("sh1", "S&H 1", 60.0, 286.0),
            S::output("sh2", "S&H 2", 120.0, 286.0),
            S::output("track_hold", "T&H", 180.0, 286.0),
            S::output("hold_track", "H&T", 60.0, 360.0),
            S::output("slew", "SLEW", 120.0, 360.0),
            S::output("glide", "GLIDE", 180.0, 360.0),
        ]),
        "rescale" => (240.0, vec![
            S::input("signal", "IN", 60.0, 360.0),
            S::output("out", "OUT", 180.0, 360.0),
        ]),
        "gate" => (150.0, vec![S::output("out", "OUT", 75.0, 374.0)]),
        "attenuverter" => (180.0, vec![
            S::input("signal", "IN", 90.0, 332.0),
            S::output("out", "OUT", 90.0, 374.0),
        ]),
        "white_noise" => (180.0, vec![S::output("out", "OUT", 90.0, 374.0)]),
        "oscillator" => (210.0, vec![
            S::input("fm", "FM", 105.0, 332.0),
            S::output("out", "OUT", 105.0, 374.0),
        ]),
        "multi_vco" => (300.0, vec![
            S::input("pitch", "V/OCT", 60.0, 332.0),
            S::input("fm", "FM", 120.0, 332.0),
            S::input("sync", "SYNC", 180.0, 332.0),
            S::input("pwm", "PWM", 240.0, 332.0),
            S::output("sine", "SIN", 60.0, 374.0),
            S::output("triangle", "TRI", 120.0, 374.0),
            S::output("saw", "SAW", 180.0, 374.0),
            S::output("square", "SQR", 240.0, 374.0),
        ]),
        "multi_lfo" => (300.0, vec![
            S::input("fm", "FM", 60.0, 332.0),
            S::input("pwm", "PWM", 120.0, 332.0),
            S::input("reset", "RESET", 180.0, 332.0),
            S::input("clock", "CLK", 240.0, 332.0),
            S::output("sine", "SIN", 60.0, 374.0),
            S::output("triangle", "TRI", 120.0, 374.0),
            S::output("saw", "SAW", 180.0, 374.0),
            S::output("square", "SQR", 240.0, 374.0),
        ]),
        "wavetable_vco" => (360.0, vec![
            S::input("fm", "FM", 36.0, 367.0),
            S::input("sync", "SYNC", 108.0, 367.0),
            S::input("position_cv", "POS CV", 180.0, 367.0),
            S::input("pitch", "V/OCT", 252.0, 367.0),
            S::output("out", "OUT", 324.0, 367.0),
        ]),
        "wavetable_lfo" => (360.0, vec![
            S::input("fm", "FM", 36.0, 367.0),
            S::input("reset", "RESET", 108.0, 367.0),
            S::input("position_cv", "POS CV", 180.0, 367.0),
            S::input("clock", "CLOCK", 252.0, 367.0),
            S::output("out", "OUT", 324.0, 367.0),
        ]),
        "stock_vcf" => (270.0, vec![
            S::input("cutoff_cv", "FREQ CV", 37.5, 337.0),
            S::input("resonance_cv", "RES CV", 102.5, 337.0),
            S::input("drive_cv", "DRIVE CV", 167.5, 337.0),
            S::input("signal", "IN", 232.5, 337.0),
            S::output("lowpass", "LP", 85.0, 374.0),
            S::output("highpass", "HP", 185.0, 374.0),
        ]),
        "stock_vca" => (180.0, vec![
            S::input("cv", "CV", 32.0, 365.0),
            S::input("signal", "IN", 87.0, 365.0),
            S::output("out", "OUT", 146.0, 365.0),
        ]),
        _ => panic!("unreviewed large socket kind {kind}"),
    }
}

#[derive(Default)]
struct Painted {
    rings: Vec<(Pos2, f32, Color32)>,
    texts: Vec<(String, Rect)>,
    plates: Vec<Rect>,
    wells: Vec<Rect>,
    targets: Vec<(SocketSpec, Rect)>,
    controls: Vec<Rect>,
}

fn collect(shape: &egui::Shape, painted: &mut Painted) {
    match shape {
        egui::Shape::Vec(shapes) => {
            for child in shapes { collect(child, painted); }
        }
        egui::Shape::Circle(circle) => painted.rings.push((circle.center, circle.radius, circle.fill)),
        egui::Shape::Text(text) if !text.galley.text().is_empty() => painted.texts.push((
            text.galley.text().into(), text.galley.rect.translate(text.pos.to_vec2()),
        )),
        egui::Shape::Rect(rect) if rect.fill == crate::panel_skin::INK => painted.plates.push(rect.rect),
        egui::Shape::Rect(rect) if rect.fill == Color32::from_rgb(47, 55, 48) => painted.wells.push(rect.rect),
        _ => {}
    }
}

fn socket_center(bounds: Rect, zoom: f32, spec: SocketSpec) -> Pos2 {
    bounds.min + vec2(spec.x, spec.y) * zoom
}

fn rendered(kind: &str, value_case: &str, zoom: f32, browser: bool) -> Painted {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"].as_array().unwrap().iter()
        .find(|module| module["kind"] == kind).unwrap();
    let (width, sockets) = contract(kind);
    assert_eq!(panel_width(kind), width, "{kind}: saved face width");
    assert_eq!(port_material_scale(kind), 1.30, "{kind}: material scale");
    let mut preview = ModulePreview::new(module);
    preview.patch.nodes[0].id = "subject".into();
    preview.renderer.positions.clear();
    preview.renderer.positions.insert("subject".into(), Pos2::ZERO);
    for (name, spec) in module["params"].as_object().unwrap() {
        if spec["hidden"] != true {
            preview.patch.nodes[0].params.insert(
                name.clone(), spec[value_case].as_f64().unwrap() as f32,
            );
        }
    }
    let patch = preview.patch.clone();
    let bounds = Rect::from_min_size(pos2(24.0, 12.0), vec2(width, PANEL_HEIGHT) * zoom);
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    let mut rack = Rack { pan: Vec2::ZERO, zoom, ..Rack::default() };
    rack.positions.insert("subject".into(), Pos2::ZERO);
    let mut painted = Painted::default();
    for _ in 0..2 {
        painted = Painted::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0))),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                    if browser {
                        preview.show(ui, bounds, &catalog);
                    } else {
                        let mut child = ui.new_child(egui::UiBuilder::new()
                            .id_salt("large-socket-rack").max_rect(bounds));
                        // The real rack clips interaction to the canvas, not
                        // to one panel face. At 25% zoom an 18-pixel accessible
                        // jack target may extend beyond its painted module.
                        child.set_clip_rect(ui.clip_rect());
                        let drawn = rack.render_modules(&mut child, &patch, &catalog, bounds,
                            48_000, 0, RenderMode::Interactive {
                                drag_interactive: false, allow_manual_press: false,
                            });
                        assert!(drawn.action.is_none());
                        for socket in &sockets {
                            let direction = if socket.output { "output" } else { "input" };
                            let id = child.id().with((direction, "subject", socket.name));
                            let response = ctx.read_response(id).unwrap_or_else(||
                                panic!("{kind}.{}: missing actual jack response", socket.name));
                            painted.targets.push((*socket, response.interact_rect));
                        }
                        for (name, _) in ordered_parameters(&patch.nodes[0], Some(module)) {
                            for suffix in ["knob", "value"] {
                                let id = child.id().with((suffix, "subject", name));
                                if let Some(response) = ctx.read_response(id) {
                                    painted.controls.push(response.interact_rect);
                                }
                            }
                        }
                        if kind == "stock_vcf" {
                            for index in 0..6 {
                                for local in [stock_vcf_knob_rect(index), stock_vcf_value_rect(index)] {
                                    painted.controls.push(Rect::from_min_max(
                                        bounds.min + local.min.to_vec2() * zoom,
                                        bounds.min + local.max.to_vec2() * zoom,
                                    ));
                                }
                            }
                        } else if kind == "stock_vca" {
                            for local in [stock_vca_knob_rect(), stock_vca_value_rect(), stock_vca_mode_rect()] {
                                painted.controls.push(Rect::from_min_max(
                                    bounds.min + local.min.to_vec2() * zoom,
                                    bounds.min + local.max.to_vec2() * zoom,
                                ));
                            }
                        }
                    }
                });
            },
        );
        for shape in output.shapes { collect(&shape.shape, &mut painted); }
    }
    painted
}

fn separated_from_ring(rect: Rect, center: Pos2, radius: f32, margin: f32) -> bool {
    let nearest = pos2(center.x.clamp(rect.left(), rect.right()),
        center.y.clamp(rect.top(), rect.bottom()));
    nearest.distance(center) > radius + margin
}

fn assert_large_socket_kind(kind: &str) {
    let catalog = openpatch_core::catalog();
        let (width, sockets) = contract(kind);
        let module = catalog["modules"].as_array().unwrap().iter()
            .find(|module| module["kind"] == kind).unwrap();
        let input_names = sockets.iter().filter(|s| !s.output).map(|s| s.name).collect::<Vec<_>>();
        let output_names = sockets.iter().filter(|s| s.output).map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(module["inputs"].as_array().unwrap().iter()
            .map(|name| name.as_str().unwrap()).collect::<Vec<_>>(), input_names, "{kind} inputs");
        assert_eq!(module["outputs"].as_array().unwrap().iter()
            .map(|name| name.as_str().unwrap()).collect::<Vec<_>>(), output_names, "{kind} outputs");
        for socket in &sockets {
            let actual = port_position(kind, !socket.output,
                if socket.output {
                    output_names.iter().position(|name| *name == socket.name).unwrap()
                } else {
                    input_names.iter().position(|name| *name == socket.name).unwrap()
                }, if socket.output { output_names.len() } else { input_names.len() }, width);
            assert_eq!(actual, pos2(socket.x, socket.y), "{kind}.{} endpoint", socket.name);
        }
        for zoom in [0.25, 0.35, 0.5, 1.0] {
            let bounds = Rect::from_min_size(pos2(24.0, 12.0), vec2(width, PANEL_HEIGHT) * zoom);
            for value_case in ["min", "default", "max"] {
                let rack = rendered(kind, value_case, zoom, false);
                let browser = rendered(kind, value_case, zoom, true);
                let context = format!("{kind} {value_case} {zoom}");
                assert_eq!(rack.targets.len(), sockets.len(), "{context}: actual jack targets");
                for (socket, target) in &rack.targets {
                    let center = socket_center(bounds, zoom, *socket);
                    assert!(target.contains(center), "{context}.{}: target misses anchor", socket.name);
                    let expected_width = (29.0 * 1.30 * zoom).max(18.0);
                    let expected_height = (if kind == "stock_vcf" { 36.0 } else { 29.0 * 1.30 }
                        * zoom).max(18.0);
                    assert!((target.width() - expected_width).abs() < 0.01
                        && (target.height() - expected_height).abs() < 0.01,
                        "{context}.{}: actual target {target:?}", socket.name);
                    assert!(bounds.contains(center), "{context}.{}: endpoint outside face", socket.name);
                    for control in &rack.controls {
                        assert!(!control.contains(center),
                            "{context}.{}: control owns exact jack center", socket.name);
                    }
                    let rim = if socket.output { 13.0 } else { 10.3 } * 1.30 * zoom;
                    assert!(center.x - rim >= bounds.left() && center.x + rim <= bounds.right()
                        && center.y - rim >= bounds.top() && center.y + rim <= bounds.bottom(),
                        "{context}.{}: enlarged rim leaves face", socket.name);
                    let rim_color = if socket.output { crate::panel_skin::INK }
                        else { Color32::from_rgb(190, 191, 182) };
                    assert!(rack.rings.iter().any(|(painted, radius, color)|
                        painted.distance(center) < 0.02
                            && (*radius - rim).abs() < 0.02 && *color == rim_color),
                        "{context}.{}: missing actual enlarged socket surround", socket.name);
                    assert!(browser.rings.iter().any(|(painted, radius, color)|
                        painted.distance(center) < 0.02
                            && (*radius - rim).abs() < 0.02 && *color == rim_color),
                        "{context}.{}: browser socket differs", socket.name);
                    let (_, caption) = rack.texts.iter()
                        .filter(|(text, _)| text == socket.label)
                        .min_by(|a, b| a.1.center().distance(center).total_cmp(&b.1.center().distance(center)))
                        .unwrap_or_else(|| panic!("{context}.{}: missing {} caption", socket.name, socket.label));
                    assert!(caption.center().distance(center) <= 44.0 * zoom,
                        "{context}.{}: caption is not beside its socket: {caption:?}", socket.name);
                    assert!(bounds.expand(0.2).contains_rect(*caption),
                        "{context}.{}: caption leaves original face: {caption:?}", socket.name);
                    for control in &rack.controls {
                        assert!(!caption.intersects(*control),
                            "{context}.{}: port print crosses parameter target {control:?}", socket.name);
                    }
                    if socket.output {
                        let plate = rack.plates.iter().find(|plate|
                            plate.contains_rect(*caption) && plate.width() < 80.0 * zoom);
                        let Some(plate) = plate else {
                            panic!("{context}.{}: output caption lacks legible plate", socket.name)
                        };
                        assert!(bounds.expand(0.2).contains_rect(*plate),
                            "{context}.{}: output caption plate leaves face", socket.name);
                        for other in &sockets {
                            let other_center = socket_center(bounds, zoom, *other);
                            let other_rim = if other.output { 13.0 } else { 10.3 } * 1.30 * zoom;
                            assert!(separated_from_ring(*plate, other_center, other_rim, 0.0),
                                "{context}.{}: plate crosses {} surround", socket.name, other.name);
                        }
                    }
                }
                for well in &rack.wells {
                    for socket in &sockets {
                        let center = socket_center(bounds, zoom, *socket);
                        let rim = if socket.output { 13.0 } else { 10.3 } * 1.30 * zoom;
                        assert!(separated_from_ring(*well, center, rim, 0.0),
                            "{context}: numeric well crosses {} enlarged surround", socket.name);
                    }
                }
                for (text, rect) in &rack.texts {
                    assert!(bounds.expand(0.5).contains_rect(*rect),
                        "{context}: print {text:?} leaves original face: {rect:?}");
                    for socket in &sockets {
                        let center = socket_center(bounds, zoom, *socket);
                        let rim = if socket.output { 13.0 } else { 10.3 } * 1.30 * zoom;
                        assert!(separated_from_ring(*rect, center, rim, 0.0),
                            "{context}: print {text:?} crosses {} enlarged surround", socket.name);
                    }
                }
                for (index, (rack_text, rack_rect)) in rack.texts.iter().enumerate() {
                    let (browser_text, browser_rect) = &browser.texts[index];
                    assert_eq!(rack_text, browser_text, "{context}: rack/browser label order");
                    assert!(rack_rect.min.distance(browser_rect.min) < 0.02
                        && rack_rect.max.distance(browser_rect.max) < 0.02,
                        "{context}: rack/browser {rack_text:?} placement");
                }
                assert_eq!(rack.texts.len(), browser.texts.len(), "{context}: browser print count");
                if kind == "compare" {
                    for x in [30.0, 90.0, 150.0, 210.0] {
                        let center = bounds.min + vec2(x, 383.0) * zoom;
                        for dx in [-3.5, 3.5] {
                            assert!(rack.rings.iter().any(|(painted, radius, color)|
                                painted.distance(center + vec2(dx, 0.0) * zoom) < 0.02
                                    && (*radius - 3.0 * zoom).abs() < 0.02
                                    && *color == Color32::from_gray(78)),
                                "{context}: paired output light no longer clears enlarged lower rim");
                        }
                    }
                }
            }
        }
}

macro_rules! large_socket_face_case {
    ($test:ident, $kind:literal) => {
        #[test]
        fn $test() {
            assert_large_socket_kind($kind);
        }
    };
}

large_socket_face_case!(large_socket_analog_vco, "analog_vco");
large_socket_face_case!(large_socket_ladder_vcf, "ladder_vcf");
large_socket_face_case!(large_socket_fade, "fade");
large_socket_face_case!(large_socket_compare, "compare");
large_socket_face_case!(large_socket_process, "process");
large_socket_face_case!(large_socket_rescale, "rescale");
large_socket_face_case!(large_socket_gate, "gate");
large_socket_face_case!(large_socket_attenuverter, "attenuverter");
large_socket_face_case!(large_socket_white_noise, "white_noise");
large_socket_face_case!(large_socket_oscillator, "oscillator");
large_socket_face_case!(large_socket_multi_vco, "multi_vco");
large_socket_face_case!(large_socket_multi_lfo, "multi_lfo");
large_socket_face_case!(large_socket_wavetable_vco, "wavetable_vco");
large_socket_face_case!(large_socket_wavetable_lfo, "wavetable_lfo");
large_socket_face_case!(large_socket_stock_vcf, "stock_vcf");
large_socket_face_case!(large_socket_stock_vca, "stock_vca");

fn pointer_button(position: Pos2, button: PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn touch(position: Pos2, phase: egui::TouchPhase) -> egui::Event {
    egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(7),
        phase,
        pos: position,
        force: None,
    }
}

fn output_texts(output: &egui::FullOutput) -> Vec<String> {
    fn visit(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(shapes) => {
                for child in shapes { visit(child, out); }
            }
            egui::Shape::Text(text) if !text.galley.text().is_empty() => {
                out.push(text.galley.text().into());
            }
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for shape in &output.shapes { visit(&shape.shape, &mut texts); }
    texts
}

fn one_module(kind: &str, zoom: f32) -> (egui::Context, Rack, Patch) {
    let patch: Patch = serde_json::from_value(json!({
        "version": 2,
        "nodes": [{"id":"subject", "kind":kind}]
    })).unwrap();
    let mut rack = Rack { zoom, ..Rack::default() };
    rack.positions.insert("subject".into(), Pos2::ZERO);
    let ctx = egui::Context::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    (ctx, rack, patch)
}

fn click_jack(ctx: &egui::Context, rack: &mut Rack, patch: &Patch, port: &str, input: bool) {
    let center = fixture_jack(rack, patch, "subject", port, input);
    frame(ctx, rack, patch, 0.1, vec![egui::Event::PointerMoved(center)]);
    frame(ctx, rack, patch, 0.2,
        vec![pointer_button(center, PointerButton::Primary, true)]);
    frame(ctx, rack, patch, 0.3,
        vec![pointer_button(center, PointerButton::Primary, false)]);
}

#[test]
fn release_over_socket_after_pressing_elsewhere_does_not_start_a_cable() {
    for zoom in [0.25, 0.35, 1.0] {
        let (ctx, mut rack, patch) = one_module("stock_vca", zoom);
        let jack = fixture_jack(&rack, &patch, "subject", "cv", true);
        let background = pos2(700.0, 500.0);
        frame(&ctx, &mut rack, &patch, 0.1,
            vec![egui::Event::PointerMoved(background)]);
        frame(&ctx, &mut rack, &patch, 0.2,
            vec![pointer_button(background, PointerButton::Primary, true)]);
        frame(&ctx, &mut rack, &patch, 0.3,
            vec![egui::Event::PointerMoved(jack)]);
        let action = frame(&ctx, &mut rack, &patch, 0.4,
            vec![pointer_button(jack, PointerButton::Primary, false)]);
        assert!(action.is_none(), "{zoom}: release-only jack produced an edit");
        assert!(rack.pending.is_none() && rack.cable_press.is_none() && !rack.dragging_cable,
            "{zoom}: release-only jack stole cable ownership");
    }
}

#[test]
fn owned_overlapping_socket_stays_the_origin_during_drag_out_and_clears_on_release() {
    for (zoom, bias) in [(0.25, 3.0), (0.35, 5.0)] {
        let (ctx, mut rack, patch) = one_module("stock_vcf", zoom);
        let input = fixture_jack(&rack, &patch, "subject", "resonance_cv", true);
        let output = fixture_jack(&rack, &patch, "subject", "lowpass", false);
        let shared = input + vec2(0.0, bias);
        assert!(Rect::from_center_size(input, Vec2::splat(18.0)).contains(shared));
        assert!(Rect::from_center_size(output, Vec2::splat(18.0)).contains(shared));
        assert!(shared.distance(input) < shared.distance(output));
        frame(&ctx, &mut rack, &patch, 0.1, vec![egui::Event::PointerMoved(shared)]);
        frame(&ctx, &mut rack, &patch, 0.2,
            vec![pointer_button(shared, PointerButton::Primary, true)]);
        let outside = pos2(400.0, 400.0);
        frame(&ctx, &mut rack, &patch, 0.3, vec![egui::Event::PointerMoved(outside)]);
        assert_eq!(rack.pending.as_ref().map(|pending| pending.origin.clone()),
            Some(CableEnd::Input { node: "subject".into(), port: "resonance_cv".into() }),
            "{zoom}: moving outside switched the original shared-target owner");
        assert!(rack.dragging_cable, "{zoom}: owned jack did not start a drag");
        let action = frame(&ctx, &mut rack, &patch, 0.4,
            vec![pointer_button(outside, PointerButton::Primary, false)]);
        assert!(action.is_none(), "{zoom}: empty-port drag-out produced a cable edit");
        assert!(rack.pending.is_none() && rack.cable_press.is_none() && !rack.dragging_cable,
            "{zoom}: drag-out left a stale cable owner");
    }
}

#[test]
fn focus_or_modal_interruption_cancels_socket_ownership_before_next_release() {
    for interruption in ["focus", "modal"] {
        let (ctx, mut rack, patch) = one_module("stock_vca", 0.25);
        click_jack(&ctx, &mut rack, &patch, "cv", true);
        assert!(rack.pending.is_some(), "{interruption}: fixture did not pick jack");
        if interruption == "focus" {
            frame_state(&ctx, &mut rack, &patch, 0.4,
                vec![egui::Event::WindowFocused(false)], false, true);
        } else {
            frame_state(&ctx, &mut rack, &patch, 0.4, vec![], true, false);
        }
        assert!(rack.pending.is_none() && rack.cable_press.is_none() && !rack.dragging_cable,
            "{interruption}: interrupted gesture kept socket owner");
        let next = fixture_jack(&rack, &patch, "subject", "signal", true);
        let action = frame(&ctx, &mut rack, &patch, 0.5,
            vec![pointer_button(next, PointerButton::Primary, false)]);
        assert!(action.is_none() && rack.pending.is_none(),
            "{interruption}: stale release resumed interrupted cable");
    }
}

#[test]
fn removed_or_reused_module_id_cannot_retain_a_socket_origin() {
    for replacement in ["removed", "reused"] {
        let (ctx, mut rack, patch) = one_module("stock_vcf", 0.25);
        click_jack(&ctx, &mut rack, &patch, "resonance_cv", true);
        assert_eq!(rack.pending.as_ref().map(|pending| pending.origin.clone()),
            Some(CableEnd::Input { node: "subject".into(), port: "resonance_cv".into() }));
        let mut changed = patch.clone();
        if replacement == "removed" {
            changed.nodes.clear();
        } else {
            changed.nodes[0].kind = "stock_vca".into();
        }
        frame(&ctx, &mut rack, &changed, 0.4, vec![]);
        assert!(rack.pending.is_none() && rack.cable_press.is_none() && !rack.dragging_cable,
            "{replacement}: invalid original jack survived graph change");
        if replacement == "removed" {
            changed.nodes = patch.nodes.clone();
            changed.nodes[0].kind = "stock_vca".into();
            frame(&ctx, &mut rack, &changed, 0.5, vec![]);
        }
        let next = fixture_jack(&rack, &changed, "subject", "cv", true);
        frame(&ctx, &mut rack, &changed, 0.6,
            vec![egui::Event::PointerMoved(next)]);
        frame(&ctx, &mut rack, &changed, 0.7,
            vec![pointer_button(next, PointerButton::Primary, true)]);
        frame(&ctx, &mut rack, &changed, 0.8,
            vec![pointer_button(next, PointerButton::Primary, false)]);
        assert_eq!(rack.pending.as_ref().map(|pending| pending.origin.clone()),
            Some(CableEnd::Input { node: "subject".into(), port: "cv".into() }),
            "{replacement}: new kind did not take fresh canonical jack ownership");
    }
}

#[test]
fn every_enlarged_socket_exact_center_keeps_its_canonical_cable_identity_at_low_zoom() {
    for kind in KINDS {
        let (_, sockets) = contract(kind);
        for zoom in [0.25, 0.35] {
            for socket in &sockets {
                let (ctx, mut rack, patch) = one_module(kind, zoom);
                let center = fixture_jack(&rack, &patch, "subject", socket.name, !socket.output);
                frame(&ctx, &mut rack, &patch, 0.1, vec![egui::Event::PointerMoved(center)]);
                frame(&ctx, &mut rack, &patch, 0.2,
                    vec![pointer_button(center, PointerButton::Primary, true)]);
                frame(&ctx, &mut rack, &patch, 0.3,
                    vec![pointer_button(center, PointerButton::Primary, false)]);
                assert_eq!(rack.pending.as_ref().map(|pending| pending.origin.clone()),
                    Some(socket.end()), "{kind}.{} {zoom}: exact-center primary pick", socket.name);
            }
        }
    }
}

fn occupied_vcf(zoom: f32) -> (egui::Context, Rack, Patch) {
    let patch: Patch = serde_json::from_value(json!({
        "version":2,
        "nodes":[
            {"id":"subject","kind":"stock_vcf"},
            {"id":"source","kind":"constant"},
            {"id":"sink","kind":"audio_stereo"}
        ],
        "cables":[
            {"id":"feed","from":"source","output":"out","to":"subject","input":"resonance_cv"},
            {"id":"filtered","from":"subject","output":"lowpass","to":"sink","input":"left"}
        ]
    })).unwrap();
    let mut rack = Rack { zoom, ..Rack::default() };
    rack.positions.insert("subject".into(), pos2(500.0, 0.0));
    rack.positions.insert("source".into(), Pos2::ZERO);
    rack.positions.insert("sink".into(), pos2(1200.0, 0.0));
    let ctx = egui::Context::default();
    frame(&ctx, &mut rack, &patch, 0.0, vec![]);
    (ctx, rack, patch)
}

#[test]
fn actual_touch_hold_opens_nearest_overlapping_vcf_and_ordinary_stock_oscillator_menus() {
    for kind in ["stock_vcf", "stock_vco", "stock_lfo"] {
        let (ctx, mut rack, patch, position, expected_port) = if kind == "stock_vcf" {
            let (ctx, rack, patch) = occupied_vcf(0.25);
            let position = vcf_shared_input(&rack, &patch);
            (ctx, rack, patch, position, "resonance_cv")
        } else {
            let (ctx, rack, patch) = one_module(kind, 0.5);
            let port = if kind == "stock_vco" { "pitch" } else { "fm" };
            let position = fixture_jack(&rack, &patch, "subject", port, true);
            (ctx, rack, patch, position, port)
        };
        frame(&ctx, &mut rack, &patch, 0.1,
            vec![egui::Event::PointerMoved(position)]);
        frame(&ctx, &mut rack, &patch, 0.2,
            vec![touch(position, egui::TouchPhase::Start),
                egui::Event::PointerMoved(position),
                pointer_button(position, PointerButton::Primary, true)]);
        frame(&ctx, &mut rack, &patch, 0.5, vec![]);
        frame(&ctx, &mut rack, &patch, 0.9, vec![]);
        let (_, threshold) = frame_output(&ctx, &mut rack, &patch, 1.05, vec![]);
        let (_, opened) = frame_output(&ctx, &mut rack, &patch, 1.1, vec![]);
        let (_, settled) = frame_output(&ctx, &mut rack, &patch, 1.15, vec![]);
        let headings = [threshold, opened, settled].iter().flat_map(output_texts)
            .collect::<Vec<_>>();
        let heading = format!("subject.{expected_port} ·");
        assert!(rack.cable_menu.is_some() && headings.iter().any(|text| text.contains(&heading)),
            "{kind}: real touch hold did not open the canonical {expected_port} menu: {headings:?}");
        frame(&ctx, &mut rack, &patch, 1.2,
            vec![touch(position, egui::TouchPhase::End),
                pointer_button(position, PointerButton::Primary, false),
                egui::Event::PointerGone]);
    }
}

#[test]
fn ordinary_mouse_hold_without_touch_event_does_not_open_a_socket_menu() {
    let (ctx, mut rack, patch) = occupied_vcf(0.25);
    let position = vcf_shared_input(&rack, &patch);
    frame(&ctx, &mut rack, &patch, 0.1,
        vec![egui::Event::PointerMoved(position)]);
    frame(&ctx, &mut rack, &patch, 0.2,
        vec![pointer_button(position, PointerButton::Primary, true)]);
    frame(&ctx, &mut rack, &patch, 0.5, vec![]);
    frame(&ctx, &mut rack, &patch, 0.9, vec![]);
    let (_, held) = frame_output(&ctx, &mut rack, &patch, 1.05, vec![]);
    let (_, settled) = frame_output(&ctx, &mut rack, &patch, 1.1, vec![]);
    assert!(rack.cable_menu.is_none() && rack.requested_cable_menu.is_none(),
        "mouse hold without touch event opened a cable menu");
    let headings = [held, settled].iter().flat_map(output_texts).collect::<Vec<_>>();
    assert!(!headings.iter().any(|text| text.contains("subject.resonance_cv ·")),
        "mouse hold painted a touch-only context menu: {headings:?}");
    frame(&ctx, &mut rack, &patch, 1.2,
        vec![pointer_button(position, PointerButton::Primary, false)]);
}

#[test]
fn queued_touch_menu_is_discarded_on_interruption_before_the_next_frame() {
    for interruption in ["focus", "disabled modal", "revision", "popup", "removed", "kind reuse"] {
        let (ctx, mut rack, patch) = one_module("stock_vcf", 0.25);
        let position = vcf_shared_input(&rack, &patch);
        frame_revision_state(&ctx, &mut rack, &patch, 0.1,
            vec![egui::Event::PointerMoved(position)], true, true, 0);
        frame_revision_state(&ctx, &mut rack, &patch, 0.2,
            vec![touch(position, egui::TouchPhase::Start),
                egui::Event::PointerMoved(position),
                pointer_button(position, PointerButton::Primary, true)], true, true, 0);
        frame_revision_state(&ctx, &mut rack, &patch, 0.5, vec![], true, true, 0);
        frame_revision_state(&ctx, &mut rack, &patch, 0.9, vec![], true, true, 0);
        frame_revision_state(&ctx, &mut rack, &patch, 1.05, vec![], true, true, 0);
        assert!(rack.deferred_touch_menu.as_ref().is_some_and(|(end, kind, revision)|
            end == &CableEnd::Input { node: "subject".into(), port: "resonance_cv".into() }
                && kind == "stock_vcf" && *revision == 0),
            "{interruption}: actual touch did not queue the typed nearest-jack menu");
        assert!(rack.requested_cable_menu.is_none() && rack.cable_menu.is_none(),
            "{interruption}: touch bypassed the guarded deferred queue");

        let mut changed = patch.clone();
        let mut revision = 0;
        let mut focused = true;
        let mut enabled = true;
        let mut events = Vec::new();
        let popup_id = egui::Id::new("external-touch-interruption-popup");
        match interruption {
            "focus" => {
                focused = false;
                events.push(egui::Event::WindowFocused(false));
            }
            "disabled modal" => enabled = false,
            "revision" => revision = 1,
            "popup" => {
                egui::Popup::open_id(&ctx, popup_id);
                assert!(egui::Popup::is_any_open(&ctx), "popup interruption did not open");
            }
            "removed" => changed.nodes.clear(),
            "kind reuse" => changed.nodes[0].kind = "stock_vca".into(),
            _ => unreachable!(),
        }
        frame_revision_state(&ctx, &mut rack, &changed, 1.1,
            events, focused, enabled, revision);
        assert!(rack.deferred_touch_menu.is_none() && rack.requested_cable_menu.is_none()
            && rack.cable_menu.is_none(),
            "{interruption}: invalid queued touch menu survived the interruption");

        if interruption == "popup" {
            egui::Popup::close_id(&ctx, popup_id);
        }
        if interruption == "removed" {
            changed.nodes = patch.nodes.clone();
            changed.nodes[0].kind = "stock_vca".into();
        }
        let (_, resumed) = frame_revision_state(&ctx, &mut rack, &changed, 1.15,
            vec![], true, true, revision);
        let (_, released) = frame_revision_state(&ctx, &mut rack, &changed, 1.2,
            vec![touch(position, egui::TouchPhase::End),
                pointer_button(position, PointerButton::Primary, false),
                egui::Event::PointerGone], true, true, revision);
        let (_, settled) = frame_revision_state(&ctx, &mut rack, &changed, 1.25,
            vec![], true, true, revision);
        assert!(rack.deferred_touch_menu.is_none() && rack.requested_cable_menu.is_none()
            && rack.cable_menu.is_none(),
            "{interruption}: stale touch menu opened after interaction resumed");
        let headings = [resumed, released, settled].iter().flat_map(output_texts)
            .collect::<Vec<_>>();
        assert!(!headings.iter().any(|text| text.contains("subject.resonance_cv ·")
            || text.contains("subject.cv ·")),
            "{interruption}: stale touch menu painted after interruption: {headings:?}");
    }
}

fn vcf_shared_input(rack: &Rack, patch: &Patch) -> Pos2 {
    let input = fixture_jack(rack, patch, "subject", "resonance_cv", true);
    let output = fixture_jack(rack, patch, "subject", "lowpass", false);
    let shared = input + vec2(0.0, 3.0);
    assert!(Rect::from_center_size(input, Vec2::splat(18.0)).contains(shared));
    assert!(Rect::from_center_size(output, Vec2::splat(18.0)).contains(shared));
    assert!(shared.distance(input) < shared.distance(output));
    shared
}

fn assert_no_secondary_menu(rack: &Rack, outputs: &[egui::FullOutput], case: &str) {
    assert!(rack.cable_secondary_press.is_none(), "{case}: secondary press owner survived");
    assert!(rack.cable_menu.is_none(), "{case}: cable menu opened unexpectedly");
    assert!(rack.requested_cable_menu.is_none(), "{case}: deferred cable menu remained");
    let headings = outputs.iter().flat_map(output_texts).collect::<Vec<_>>();
    assert!(!headings.iter().any(|text| text.contains("subject.resonance_cv ·")
        || text.contains("subject.cutoff_cv ·") || text.contains("subject.cv ·")
        || text.contains("subject.lowpass ·")),
        "{case}: interrupted gesture painted a cable menu: {headings:?}");
}

#[test]
fn secondary_release_over_jack_needs_a_jack_owned_press() {
    let (ctx, mut rack, patch) = occupied_vcf(0.25);
    let input = fixture_jack(&rack, &patch, "subject", "cutoff_cv", true);
    let near_outside = input + vec2(-10.0, 0.0);
    let near_inside = input + vec2(-8.0, 0.0);
    assert!(!Rect::from_center_size(input, Vec2::splat(18.0)).contains(near_outside));
    assert!(Rect::from_center_size(input, Vec2::splat(18.0)).contains(near_inside));
    let canvas = rack.viewport.unwrap();
    let catalog = openpatch_core::catalog();
    assert_eq!(rack.cable_press_target(&patch, &catalog, canvas, canvas, near_outside), None,
        "press just outside the outermost jack must not hit any other port");
    assert_eq!(rack.cable_press_target(&patch, &catalog, canvas, canvas, near_inside),
        Some(CableEnd::Input { node: "subject".into(), port: "cutoff_cv".into() }),
        "release two pixels later must be inside the intended jack target");
    frame(&ctx, &mut rack, &patch, 0.1,
        vec![egui::Event::PointerMoved(near_outside)]);
    frame(&ctx, &mut rack, &patch, 0.2,
        vec![pointer_button(near_outside, PointerButton::Secondary, true)]);
    assert!(rack.cable_secondary_press.is_none(), "outside press captured a cable menu");
    frame(&ctx, &mut rack, &patch, 0.3,
        vec![egui::Event::PointerMoved(near_inside)]);
    let (_, released) = frame_output(&ctx, &mut rack, &patch, 0.4,
        vec![pointer_button(near_inside, PointerButton::Secondary, false)]);
    let (_, shown) = frame_output(&ctx, &mut rack, &patch, 0.5, vec![]);
    assert_no_secondary_menu(&rack, &[released, shown], "outside press, jack release");
}

#[test]
fn secondary_owned_shared_jack_drag_out_does_not_open_a_menu() {
    let (ctx, mut rack, patch) = occupied_vcf(0.25);
    let shared = vcf_shared_input(&rack, &patch);
    frame(&ctx, &mut rack, &patch, 0.1, vec![egui::Event::PointerMoved(shared)]);
    frame(&ctx, &mut rack, &patch, 0.2,
        vec![pointer_button(shared, PointerButton::Secondary, true)]);
    assert!(rack.cable_secondary_press.as_ref().is_some_and(|(end, kind)|
        end == &CableEnd::Input { node: "subject".into(), port: "resonance_cv".into() }
            && kind == "stock_vcf"), "nearest shared jack did not own secondary press");
    let outside = pos2(700.0, 500.0);
    frame(&ctx, &mut rack, &patch, 0.3, vec![egui::Event::PointerMoved(outside)]);
    let (_, released) = frame_output(&ctx, &mut rack, &patch, 0.4,
        vec![pointer_button(outside, PointerButton::Secondary, false)]);
    let (_, shown) = frame_output(&ctx, &mut rack, &patch, 0.5, vec![]);
    assert_no_secondary_menu(&rack, &[released, shown], "owned secondary drag out");
}

#[test]
fn secondary_focus_modal_and_actual_popup_interruptions_discard_the_press() {
    for interruption in ["focus", "disabled modal", "popup"] {
        let (ctx, mut rack, patch) = occupied_vcf(0.25);
        let shared = vcf_shared_input(&rack, &patch);
        frame(&ctx, &mut rack, &patch, 0.1,
            vec![egui::Event::PointerMoved(shared)]);
        frame(&ctx, &mut rack, &patch, 0.2,
            vec![pointer_button(shared, PointerButton::Secondary, true)]);
        assert!(rack.cable_secondary_press.is_some(), "{interruption}: press not captured");
        match interruption {
            "focus" => {
                frame_state(&ctx, &mut rack, &patch, 0.3,
                    vec![egui::Event::WindowFocused(false)], false, true);
            }
            "disabled modal" => {
                frame_state(&ctx, &mut rack, &patch, 0.3, vec![], true, false);
            }
            _ => {
                let layer = ctx.layer_id_at(shared);
                egui::Popup::open_id(&ctx, egui::Id::new("external-large-socket-popup"));
                assert!(egui::Popup::is_any_open(&ctx), "actual popup did not open");
                assert_eq!(ctx.layer_id_at(shared), layer,
                    "popup fixture changed the rack pointer layer");
                frame(&ctx, &mut rack, &patch, 0.3, vec![]);
            }
        }
        assert!(rack.cable_secondary_press.is_none(),
            "{interruption}: interrupted secondary capture remained");
        let (_, released) = frame_output(&ctx, &mut rack, &patch, 0.4,
            vec![pointer_button(shared, PointerButton::Secondary, false)]);
        let (_, shown) = frame_output(&ctx, &mut rack, &patch, 0.5, vec![]);
        assert_no_secondary_menu(&rack, &[released, shown], interruption);
    }
}

#[test]
fn secondary_press_cannot_survive_removal_or_same_id_kind_reuse() {
    for replacement in ["removed", "reused"] {
        let (ctx, mut rack, patch) = one_module("stock_vcf", 0.25);
        let shared = vcf_shared_input(&rack, &patch);
        frame(&ctx, &mut rack, &patch, 0.1,
            vec![egui::Event::PointerMoved(shared)]);
        frame(&ctx, &mut rack, &patch, 0.2,
            vec![pointer_button(shared, PointerButton::Secondary, true)]);
        assert!(rack.cable_secondary_press.is_some(), "{replacement}: press not captured");
        let mut changed = patch.clone();
        if replacement == "removed" {
            changed.nodes.clear();
        } else {
            changed.nodes[0].kind = "stock_vca".into();
        }
        frame(&ctx, &mut rack, &changed, 0.3, vec![]);
        assert!(rack.cable_secondary_press.is_none(),
            "{replacement}: stale secondary owner survived module identity change");
        if replacement == "removed" {
            changed.nodes = patch.nodes.clone();
            changed.nodes[0].kind = "stock_vca".into();
            frame(&ctx, &mut rack, &changed, 0.4, vec![]);
        }
        let next = fixture_jack(&rack, &changed, "subject", "cv", true);
        let (_, released) = frame_output(&ctx, &mut rack, &changed, 0.5,
            vec![pointer_button(next, PointerButton::Secondary, false)]);
        let (_, shown) = frame_output(&ctx, &mut rack, &changed, 0.6, vec![]);
        assert_no_secondary_menu(&rack, &[released, shown], replacement);
        frame(&ctx, &mut rack, &changed, 0.7,
            vec![egui::Event::PointerMoved(next)]);
        frame(&ctx, &mut rack, &changed, 0.8,
            vec![pointer_button(next, PointerButton::Secondary, true)]);
        let (_, fresh_release) = frame_output(&ctx, &mut rack, &changed, 0.9,
            vec![pointer_button(next, PointerButton::Secondary, false)]);
        let (_, fresh_shown) = frame_output(&ctx, &mut rack, &changed, 1.0, vec![]);
        let headings = [fresh_release, fresh_shown].iter().flat_map(output_texts)
            .collect::<Vec<_>>();
        assert!(headings.iter().any(|text| text.contains("subject.cv ·")),
            "{replacement}: fresh kind could not open its own new socket menu: {headings:?}");
    }
}

#[test]
fn shared_hover_guidance_uses_nearest_compatible_socket() {
    for (zoom, bias) in [(0.25, 3.0), (0.35, 5.0)] {
        let (ctx, mut rack, mut patch) = occupied_vcf(zoom);
        patch.cables.retain(|cable| cable.id.as_deref() != Some("feed"));
        frame(&ctx, &mut rack, &patch, 0.1, vec![]);
        let source = fixture_jack(&rack, &patch, "source", "out", false);
        let input = fixture_jack(&rack, &patch, "subject", "resonance_cv", true);
        let output = fixture_jack(&rack, &patch, "subject", "lowpass", false);
        let shared = input + vec2(0.0, bias);
        assert!(Rect::from_center_size(input, Vec2::splat(18.0)).contains(shared));
        assert!(Rect::from_center_size(output, Vec2::splat(18.0)).contains(shared));
        assert!(shared.distance(input) < shared.distance(output));
        frame(&ctx, &mut rack, &patch, 0.2,
            vec![egui::Event::PointerMoved(source)]);
        frame(&ctx, &mut rack, &patch, 0.3,
            vec![pointer_button(source, PointerButton::Primary, true)]);
        frame(&ctx, &mut rack, &patch, 0.4,
            vec![pointer_button(source, PointerButton::Primary, false)]);
        assert_eq!(rack.pending.as_ref().map(|pending| pending.origin.clone()),
            Some(CableEnd::Output { node: "source".into(), port: "out".into() }));
        let (_, output) = frame_output(&ctx, &mut rack, &patch, 0.5,
            vec![egui::Event::PointerMoved(shared)]);
        assert_eq!(output.platform_output.cursor_icon, egui::CursorIcon::PointingHand,
            "{zoom}: shared hover must guide the valid nearest input, not the invalid output");
    }
}

#[test]
fn crowded_vcf_targets_choose_nearest_occupied_endpoint_for_press_drop_and_menu() {
    for (zoom, bias) in [(0.25, 3.0), (0.35, 5.0)] {
        let (ctx, mut rack, patch) = occupied_vcf(zoom);
        let input = fixture_jack(&rack, &patch, "subject", "resonance_cv", true);
        let output = fixture_jack(&rack, &patch, "subject", "lowpass", false);
        let shared = input + vec2(0.0, bias);
        let input_target = Rect::from_center_size(input,
            vec2((37.7 * zoom).max(18.0), (36.0 * zoom).max(18.0)));
        let output_target = Rect::from_center_size(output,
            vec2((37.7 * zoom).max(18.0), (36.0 * zoom).max(18.0)));
        assert!(input_target.contains(shared) && output_target.contains(shared),
            "{zoom}: fixture must hit genuinely overlapping jack targets");
        assert!(shared.distance(input) < shared.distance(output),
            "{zoom}: shared point must favor the input by center distance");
        frame(&ctx, &mut rack, &patch, 0.1, vec![egui::Event::PointerMoved(shared)]);
        frame(&ctx, &mut rack, &patch, 0.2,
            vec![pointer_button(shared, PointerButton::Primary, true)]);
        frame(&ctx, &mut rack, &patch, 0.3,
            vec![pointer_button(shared, PointerButton::Primary, false)]);
        assert_eq!(rack.pending.as_ref().map(|pending| pending.origin.clone()),
            Some(CableEnd::Input { node:"subject".into(), port:"resonance_cv".into() }),
            "{zoom}: occupied nearest input must own the shared primary pick");

        let (ctx, mut rack, patch) = occupied_vcf(zoom);
        let input = fixture_jack(&rack, &patch, "subject", "resonance_cv", true);
        let shared = input + vec2(0.0, bias);
        frame(&ctx, &mut rack, &patch, 0.1, vec![egui::Event::PointerMoved(shared)]);
        frame(&ctx, &mut rack, &patch, 0.2,
            vec![pointer_button(shared, PointerButton::Secondary, true)]);
        let (_, released) = frame_output(&ctx, &mut rack, &patch, 0.3,
            vec![pointer_button(shared, PointerButton::Secondary, false)]);
        let (_, shown) = frame_output(&ctx, &mut rack, &patch, 0.4, vec![]);
        let headings = [released, shown].iter().flat_map(output_texts)
            .collect::<Vec<_>>();
        assert!(headings.iter().any(|text| text == "subject.resonance_cv · 1 cables"),
            "{zoom}: right-click menu must describe the nearest occupied input: {headings:?}");

        let (ctx, mut rack, mut patch) = occupied_vcf(zoom);
        let mut fresh = patch.nodes.iter().find(|node| node.id == "source").unwrap().clone();
        fresh.id = "fresh".into();
        patch.nodes.push(fresh);
        rack.positions.insert("fresh".into(), pos2(250.0, 0.0));
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(patch, None).unwrap();
        let patch = session.patch().clone();
        let revision = session.revision();
        frame_revision_state(&ctx, &mut rack, &patch, 0.5, vec![], true, true, revision);
        let source = fixture_jack(&rack, &patch, "fresh", "out", false);
        let input = fixture_jack(&rack, &patch, "subject", "resonance_cv", true);
        let shared = input + vec2(0.0, bias);
        frame_revision_state(&ctx, &mut rack, &patch, 1.0,
            vec![egui::Event::PointerMoved(source),
                pointer_button(source, PointerButton::Primary, true)],
            true, true, revision);
        frame_revision_state(&ctx, &mut rack, &patch, 1.1,
            vec![egui::Event::PointerMoved(shared)], true, true, revision);
        let action = frame_revision_state(&ctx, &mut rack, &patch, 1.2,
            vec![pointer_button(shared, PointerButton::Primary, false)],
            true, true, revision).0;
        assert!(matches!(&action, Some(RackAction::Command("connect", args))
            if args["from"] == "fresh" && args["to"] == "subject"
                && args["input"] == "resonance_cv"),
            "{zoom}: new source drop inside two jack squares must choose nearest canonical input");
        run_cable_action(&mut session, action.unwrap());
        assert!(session.patch().cables.iter().any(|cable|
            cable.id.as_deref() == Some("feed") && cable.from == "source"
                && cable.to == "subject" && cable.input == "resonance_cv"),
            "{zoom}: adding a second source must retain the prior cable");
        assert!(session.patch().cables.iter().any(|cable|
            cable.from == "fresh" && cable.to == "subject"
                && cable.input == "resonance_cv"),
            "{zoom}: fresh route was not applied to the session");
    }
}

#[test]
fn narrow_vca_adjacent_target_overlap_preserves_first_input_and_occupied_menu() {
    let zoom = 0.25;
    let patch: Patch = serde_json::from_value(json!({
        "version":2,
        "nodes":[{"id":"subject","kind":"stock_vca"},
            {"id":"source","kind":"constant"}],
        "cables":[{"id":"level_cv","from":"source","output":"out",
            "to":"subject","input":"cv"}]
    })).unwrap();
    let mut initial = Rack { zoom, ..Rack::default() };
    initial.positions.insert("subject".into(), pos2(500.0, 0.0));
    initial.positions.insert("source".into(), Pos2::ZERO);
    for secondary in [false, true] {
        let ctx = egui::Context::default();
        let mut rack = Rack { zoom, ..Rack::default() };
        rack.positions = initial.positions.clone();
        frame(&ctx, &mut rack, &patch, 0.0, vec![]);
        let cv = fixture_jack(&rack, &patch, "subject", "cv", true);
        let signal = fixture_jack(&rack, &patch, "subject", "signal", true);
        let shared = cv + vec2(6.0, 0.0);
        let cv_target = Rect::from_center_size(cv, Vec2::splat(18.0));
        let signal_target = Rect::from_center_size(signal, Vec2::splat(18.0));
        assert!(cv_target.contains(shared) && signal_target.contains(shared));
        assert!(shared.distance(cv) < shared.distance(signal));
        frame(&ctx, &mut rack, &patch, 0.1, vec![egui::Event::PointerMoved(shared)]);
        let button = if secondary { PointerButton::Secondary } else { PointerButton::Primary };
        frame(&ctx, &mut rack, &patch, 0.2, vec![pointer_button(shared, button, true)]);
        let (_, released) = frame_output(&ctx, &mut rack, &patch, 0.3,
            vec![pointer_button(shared, button, false)]);
        if secondary {
            let (_, shown) = frame_output(&ctx, &mut rack, &patch, 0.4, vec![]);
            let headings = [released, shown].iter().flat_map(output_texts)
                .collect::<Vec<_>>();
            assert!(headings.iter().any(|text| text == "subject.cv · 1 cables"),
                "occupied CV right-click chose neighboring signal input: {headings:?}");
        } else {
            assert_eq!(rack.pending.as_ref().map(|pending| pending.origin.clone()),
                Some(CableEnd::Input { node:"subject".into(), port:"cv".into() }),
                "occupied CV primary pick chose neighboring signal input");
        }
    }
}

#[test]
fn signal_atlas_voice_socket_captions_and_controls_clear_enlarged_rims() {
    for kind in ["lfo", "lowpass", "gain", "analog_vca", "adsr", "svf_filter"] {
        assert_large_socket_kind(kind);
    }
}
