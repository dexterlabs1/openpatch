use super::*;

fn mixer_patch(channels: f32) -> Patch {
    serde_json::from_value(json!({
        "version": 2,
        "nodes": [
            {"id":"mixer", "kind":"studio_mixer", "params":{"channels":channels,"level1":0.8}},
            {"id":"source", "kind":"constant", "params":{"value":1.0}}
        ],
        "cables": [{"from":"source", "output":"out", "to":"mixer", "input":"in16"}]
    }))
    .unwrap()
}

fn mixer_pointer(at: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: at,
        button: PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn mixer_key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn mixer_shape_texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
    match shape {
        egui::Shape::Vec(children) => {
            for child in children {
                mixer_shape_texts(child, out);
            }
        }
        egui::Shape::Text(text) => out.push((
            text.galley.text().to_owned(),
            Rect::from_min_size(text.pos, text.galley.size()),
        )),
        _ => {}
    }
}

struct MixerFixture {
    ctx: egui::Context,
    rack: Rack,
    session: openpatch::control::Session,
    _workspace: tempfile::TempDir,
    time: f64,
}

impl MixerFixture {
    fn new() -> Self {
        let workspace = tempfile::tempdir().unwrap();
        let mut session = openpatch::control::Session::new(workspace.path()).unwrap();
        session.replace_patch(mixer_patch(4.0), Some(0)).unwrap();
        let mut fixture = Self {
            ctx: egui::Context::default(),
            rack: Rack::default(),
            session,
            _workspace: workspace,
            time: 0.0,
        };
        fixture.draw(vec![]);
        fixture
    }

    fn draw(&mut self, events: Vec<egui::Event>) -> Option<Value> {
        let revision = self.session.revision();
        let (action, _) = frame_revision_state(
            &self.ctx,
            &mut self.rack,
            self.session.patch(),
            self.time,
            events,
            true,
            true,
            revision,
        );
        self.time += 0.05;
        match action {
            None => None,
            Some(RackAction::Command("set_parameter", args)) => {
                assert_eq!(args["expected_revision"], revision);
                self.session.execute("set_parameter", args.clone()).unwrap();
                Some(args)
            }
            Some(_) => panic!("unexpected mixer action"),
        }
    }

    fn origin(&self) -> Pos2 {
        self.rack.viewport.unwrap().min
            + self.rack.pan
            + self.rack.positions["mixer"].to_vec2() * self.rack.zoom
    }

    fn value(&self, parameter: &str) -> f32 {
        self.session
            .patch()
            .nodes
            .iter()
            .find(|n| n.id == "mixer")
            .unwrap()
            .params[parameter]
    }

    fn click(&mut self, local: Vec2) {
        let at = self.origin() + local * self.rack.zoom;
        self.draw(vec![egui::Event::PointerMoved(at), mixer_pointer(at, true)]);
        self.draw(vec![mixer_pointer(at, false)]);
    }
}

#[test]
fn mixer_channel_buttons_mute_solo_and_fader_emit_guarded_edits() {
    let mut fixture = MixerFixture::new();
    assert_eq!(module_width(&fixture.session.patch().nodes[0]), 516.0);
    fixture.click(vec2(4.0 * 72.0 + 24.0, 83.0));
    assert_eq!(fixture.value("channels"), 3.0);
    assert_eq!(module_width(&fixture.session.patch().nodes[0]), 444.0);
    fixture.draw(vec![]);
    fixture.time = 1.0;
    fixture.click(vec2(3.0 * 72.0 + 64.0, 83.0));
    assert_eq!(fixture.value("channels"), 4.0);
    fixture.draw(vec![]);
    fixture.time = 2.0;
    fixture.click(vec2(36.0, 351.0));
    assert_eq!(fixture.value("solo1"), 1.0);
    fixture.time = 3.0;
    fixture.click(vec2(36.0, 329.0));
    assert_eq!(fixture.value("mute1"), 1.0);
    let at = fixture.origin() + vec2(30.0, 268.0);
    fixture.draw(vec![egui::Event::PointerMoved(at), mixer_pointer(at, true)]);
    fixture.draw(vec![egui::Event::PointerMoved(at + vec2(0.0, 20.0))]);
    fixture.draw(vec![mixer_pointer(at + vec2(0.0, 20.0), false)]);
    assert!(fixture.value("level1") < 0.8);
    assert_eq!(fixture.session.patch().cables[0].input, "in16");
}

#[test]
fn focused_mixer_fader_ignores_keys_while_palette_or_editor_owns_input() {
    let mut fixture = MixerFixture::new();
    let at = fixture.origin() + vec2(30.0, 268.0);
    fixture.draw(vec![egui::Event::PointerMoved(at), mixer_pointer(at, true)]);
    fixture.draw(vec![mixer_pointer(at, false)]);
    assert!(fixture.ctx.memory(|memory| memory.focused().is_some()));
    let before = fixture.value("level1");
    fixture.rack.set_shortcuts_enabled(false);
    for key in [egui::Key::ArrowUp, egui::Key::Home, egui::Key::End] {
        assert!(fixture.draw(vec![mixer_key(key)]).is_none());
        assert_eq!(fixture.value("level1"), before);
    }
    fixture.rack.set_shortcuts_enabled(true);
    let node = fixture
        .session
        .patch()
        .nodes
        .iter()
        .find(|node| node.id == "mixer")
        .unwrap();
    let catalog = openpatch_core::catalog();
    let spec = &catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "studio_mixer")
        .unwrap()["params"]["level1"];
    fixture.rack.editing = Some(studio_mixer_parameter_edit(
        node,
        "level1",
        spec,
        fixture.session.revision(),
    ));
    for key in [egui::Key::ArrowUp, egui::Key::Home, egui::Key::End] {
        assert!(fixture.draw(vec![mixer_key(key)]).is_none());
        assert_eq!(fixture.value("level1"), before);
    }
}

#[test]
fn mixer_preview_right_click_cannot_open_strip_settings() {
    let catalog = openpatch_core::catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == "studio_mixer")
        .unwrap();
    let mut preview = ModulePreview::new(module);
    let ctx = egui::Context::default();
    let bounds = Rect::from_min_size(pos2(20.0, 20.0), preview.size());
    let at = bounds.min + vec2(36.0, 83.0);
    for (time, pressed) in [(0.0, true), (0.1, false)] {
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 700.0))),
                time: Some(time),
                events: vec![
                    egui::Event::PointerMoved(at),
                    egui::Event::PointerButton {
                        pos: at,
                        button: PointerButton::Secondary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| preview.show(ui, bounds, &catalog));
            },
        );
        assert!(!egui::Popup::is_any_open(&ctx));
    }
    assert!(preview.renderer.editing.is_none());
}

#[test]
fn wide_source_identifier_elides_inside_its_strip_header() {
    let wide = "WWWWWWWWWWWWW";
    let mut patch = mixer_patch(4.0);
    patch.nodes[1].id = wide.into();
    patch.cables[0].from = wide.into();
    for zoom in [0.25, 1.0, 2.0] {
        let ctx = egui::Context::default();
        crate::panel_skin::install_fonts(&ctx);
        let mut rack = Rack {
            zoom,
            ..Rack::default()
        };
        let (_, output) = frame_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true);
        let mut texts = Vec::new();
        for shape in &output.shapes {
            mixer_shape_texts(&shape.shape, &mut texts);
        }
        let (caption_text, caption) = texts
            .iter()
            .find(|(text, _)| text.starts_with("1 · "))
            .unwrap_or_else(|| panic!("missing first strip caption at zoom {zoom}"));
        let origin =
            rack.viewport.unwrap().min + rack.pan + rack.positions["mixer"].to_vec2() * zoom;
        let header =
            Rect::from_center_size(origin + vec2(36.0, 83.0) * zoom, vec2(68.0, 23.0) * zoom);
        assert!(
            caption.left() >= header.left() - 0.1,
            "{caption:?} at {zoom}"
        );
        assert!(
            caption.right() <= header.right() + 0.1,
            "{caption:?} at {zoom}"
        );
        assert!(!caption_text.contains(wide));
    }
}

#[test]
fn hidden_channel_cables_remain_saved_without_outside_hit_targets() {
    let patch = mixer_patch(3.0);
    let mixer = &patch.nodes[0];
    assert!(!studio_mixer_input_visible(mixer, "in16"));
    assert!(studio_mixer_input_visible(mixer, "in3"));
    assert!(studio_mixer_input_visible(mixer, "return_a"));
    assert_eq!(patch.cables[0].input, "in16");
    let mut rack = Rack::default();
    let ctx = egui::Context::default();
    let _ = frame_state(&ctx, &mut rack, &patch, 0.0, vec![], true, true);
    rack.pan = vec2(-500.0, 0.0);
    let canvas = rack.viewport.unwrap();
    let phantom = canvas.min
        + rack.pan
        + (rack.positions["mixer"].to_vec2()
            + studio_mixer_port(true, 15, module_width(mixer)).to_vec2())
            * rack.zoom;
    assert!(!Rect::from_min_size(
        canvas.min + rack.pan + rack.positions["mixer"].to_vec2() * rack.zoom,
        vec2(module_width(mixer), PANEL_HEIGHT) * rack.zoom
    )
    .contains(phantom));
    assert!(canvas.contains(phantom));
    assert!(rack
        .cable_press_target(&patch, &openpatch_core::catalog(), canvas, canvas, phantom)
        .is_none());
    let expanded = mixer_patch(16.0);
    assert!(studio_mixer_input_visible(&expanded.nodes[0], "in16"));
    assert_eq!(module_width(&expanded.nodes[0]), 1380.0);
    assert_eq!(expanded.cables[0].input, patch.cables[0].input);
    let last_input = studio_mixer_port(true, 15, module_width(&expanded.nodes[0]));
    assert!(last_input.x < module_width(&expanded.nodes[0]));
    for output in 0..8 {
        let jack = studio_mixer_port(false, output, module_width(&expanded.nodes[0]));
        assert!(jack.x > 0.0 && jack.x < module_width(&expanded.nodes[0]));
    }
}

#[test]
fn mixer_growth_moves_only_the_right_hand_collision_chain() {
    let mut patch = mixer_patch(3.0);
    patch.nodes.extend([
        serde_json::from_value(json!({"id":"near","kind":"constant"})).unwrap(),
        serde_json::from_value(json!({"id":"chain","kind":"constant"})).unwrap(),
        serde_json::from_value(json!({"id":"far","kind":"constant"})).unwrap(),
        serde_json::from_value(json!({"id":"other_row","kind":"constant"})).unwrap(),
    ]);
    let mut rack = Rack::default();
    let old_width = module_width(&patch.nodes[0]);
    let constant_width = module_width(&patch.nodes[2]);
    rack.positions.insert("mixer".into(), pos2(0.0, 0.0));
    rack.positions.insert("near".into(), pos2(old_width, 0.0));
    rack.positions
        .insert("chain".into(), pos2(old_width + constant_width, 0.0));
    rack.positions
        .insert("far".into(), pos2(old_width + constant_width * 3.0, 0.0));
    rack.positions
        .insert("other_row".into(), pos2(old_width, ROW));
    patch.nodes[0].params.insert("channels".into(), 4.0);
    rack.settle_studio_mixer_growth(&patch, "mixer", old_width)
        .unwrap();
    assert_eq!(rack.positions["near"], pos2(old_width + 72.0, 0.0));
    assert_eq!(
        rack.positions["chain"],
        pos2(old_width + 72.0 + constant_width, 0.0)
    );
    assert_eq!(
        rack.positions["far"],
        pos2(old_width + constant_width * 3.0, 0.0)
    );
    assert_eq!(rack.positions["other_row"], pos2(old_width, ROW));
}
