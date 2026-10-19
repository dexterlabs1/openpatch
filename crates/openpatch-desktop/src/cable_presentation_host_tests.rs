use super::*;

fn settings(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path.join(".host-settings.json")).unwrap()).unwrap()
}
fn frame(
    ctx: &egui::Context,
    app: &mut OpenPatch,
    time: f64,
    size: Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            focused: true,
            time: Some(time),
            events,
            ..Default::default()
        },
        |ctx| {
            app.toolbar(ctx);
            app.global_shortcuts(ctx);
            app.rack_workspace(ctx, None);
        },
    )
}
fn click(ctx: &egui::Context, app: &mut OpenPatch, at: egui::Pos2, time: f64, size: Vec2) {
    for (offset, pressed) in [(0.0, true), (0.05, false)] {
        frame(
            ctx,
            app,
            time + offset,
            size,
            vec![egui::Event::PointerMoved(at), pointer_event(at, pressed)],
        );
    }
}
fn open_view(ctx: &egui::Context, app: &mut OpenPatch, size: Vec2) -> egui::FullOutput {
    frame(ctx, app, 0.0, size, vec![]);
    let initial = frame(ctx, app, 0.1, size, vec![]);
    click(ctx, app, preset_text(&initial, "View"), 1.0, size);
    frame(ctx, app, 1.2, size, vec![])
}
fn control(ctx: &egui::Context, key: &str) -> egui::Response {
    let response = ctx
        .data(|data| {
            data.get_temp::<egui::Response>(egui::Id::new(("cable-presentation-control", key)))
        })
        .expect("actual rendered cable control");
    ctx.read_response(response.id).unwrap_or(response)
}
fn rail_point(ctx: &egui::Context, key: &str, fraction: f32) -> egui::Pos2 {
    let rect = control(ctx, key).rect;
    egui::pos2(rect.left() + rect.width() * fraction, rect.center().y)
}
fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}
fn custom() -> CablePresentation {
    CablePresentation {
        opacity: 0.23,
        tension: 0.81,
        plug_brightness: 0.37,
        ..Default::default()
    }
}

#[test]
fn cable_preferences_roundtrip_and_invalid_settings_never_enter_document_or_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let before = app.shared.lock().unwrap().snapshot();
    let document = app.capture_document();
    let layout_revision = app.rack.layout_revision();
    let dirty = app.dirty();
    assert_eq!(app.rack.cable_presentation(), CablePresentation::default());
    app.set_cable_presentation(custom());
    app.save_host_settings();
    let saved = settings(dir.path());
    for (key, expected) in [
        ("cable_opacity", 0.23_f32),
        ("cable_tension", 0.81),
        ("plug_light_brightness", 0.37),
    ] {
        assert_eq!(saved[key].as_f64().unwrap() as f32, expected);
    }
    let mut reopened = host(dir.path());
    reopened.apply_host_preferences(&saved);
    assert_eq!(reopened.rack.cable_presentation(), custom());
    for invalid in [
        Value::Null,
        json!({}),
        json!({"cable_opacity":"faint","cable_tension":false,"plug_light_brightness":null}),
    ] {
        app.apply_host_preferences(&invalid);
        assert_eq!(app.rack.cable_presentation(), CablePresentation::default());
    }
    app.apply_host_preferences(
        &json!({"cable_opacity":-5,"cable_tension":9,"plug_light_brightness":0}),
    );
    assert_eq!(
        app.rack.cable_presentation(),
        CablePresentation {
            opacity: 0.0,
            tension: 1.0,
            plug_brightness: 0.0,
            ..Default::default()
        }
    );
    app.apply_host_preferences(
        &json!({"cable_opacity":1e200,"cable_tension":-1e200,"plug_light_brightness":1e200}),
    );
    assert_eq!(
        app.rack.cable_presentation(),
        CablePresentation {
            opacity: 1.0,
            tension: 0.0,
            plug_brightness: 1.0,
            ..Default::default()
        }
    );
    app.set_cable_presentation(CablePresentation {
        opacity: f32::NAN,
        tension: f32::INFINITY,
        plug_brightness: f32::NEG_INFINITY,
        ..Default::default()
    });
    assert_eq!(app.rack.cable_presentation(), CablePresentation::default());
    assert_eq!(app.rack.layout_revision(), layout_revision);
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.dirty(), dirty);
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert!(app.audio.is_none());
}

#[test]
fn view_menu_sliders_preview_every_drag_frame_and_save_only_at_release_without_patch_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.save_host_settings();
    let ctx = egui::Context::default();
    let size = egui::vec2(1100.0, 800.0);
    let output = open_view(&ctx, &mut app, size);
    for label in ["Cable opacity", "Cable tension", "Plug light brightness"] {
        preset_text(&output, label);
    }
    let before = app.shared.lock().unwrap().snapshot();
    let document = app.capture_document();
    let dirty = app.dirty();
    let layout_revision = app.rack.layout_revision();
    let saved = std::fs::read(dir.path().join(".host-settings.json")).unwrap();
    let from = rail_point(&ctx, "cable_opacity", 0.7);
    let middle = rail_point(&ctx, "cable_opacity", 0.4);
    let to = rail_point(&ctx, "cable_opacity", 0.2);
    frame(
        &ctx,
        &mut app,
        2.0,
        size,
        vec![egui::Event::PointerMoved(from), pointer_event(from, true)],
    );
    frame(
        &ctx,
        &mut app,
        2.1,
        size,
        vec![egui::Event::PointerMoved(middle)],
    );
    let preview = app.rack.cable_presentation().opacity;
    assert!(preview > 0.2 && preview < 0.6);
    assert_eq!(
        std::fs::read(dir.path().join(".host-settings.json")).unwrap(),
        saved,
        "held preview wrote settings"
    );
    frame(
        &ctx,
        &mut app,
        2.2,
        size,
        vec![egui::Event::PointerMoved(to)],
    );
    assert!(app.rack.cable_presentation().opacity < preview);
    assert_eq!(
        std::fs::read(dir.path().join(".host-settings.json")).unwrap(),
        saved
    );
    frame(&ctx, &mut app, 2.3, size, vec![pointer_event(to, false)]);
    assert_eq!(
        settings(dir.path())["cable_opacity"].as_f64().unwrap() as f32,
        app.rack.cable_presentation().opacity
    );
    // A subsequent idle frame must not write again after the release.
    let mut sentinel = settings(dir.path());
    sentinel["idle-write-sentinel"] = json!(true);
    std::fs::write(
        dir.path().join(".host-settings.json"),
        serde_json::to_vec(&sentinel).unwrap(),
    )
    .unwrap();
    frame(&ctx, &mut app, 2.4, size, vec![]);
    assert_eq!(settings(dir.path())["idle-write-sentinel"], true);
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert_eq!(app.rack.layout_revision(), layout_revision);
    assert!(app.capture_document().equivalent(&document));
    assert_eq!(app.dirty(), dirty);
}

#[test]
fn view_keyboard_edits_and_primary_double_click_reset_use_actual_controls_and_persist() {
    for (name, reset) in [
        ("cable_opacity", 1.0),
        ("cable_tension", 0.5),
        ("plug_light_brightness", 1.0),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.set_cable_presentation(custom());
        let ctx = egui::Context::default();
        let size = egui::vec2(1100.0, 800.0);
        open_view(&ctx, &mut app, size);
        let before = app.shared.lock().unwrap().snapshot();
        let document = app.capture_document();
        let slider = control(&ctx, name);
        ctx.memory_mut(|memory| memory.request_focus(slider.id));
        frame(
            &ctx,
            &mut app,
            2.0,
            size,
            vec![key(egui::Key::ArrowRight, true)],
        );
        frame(
            &ctx,
            &mut app,
            2.01,
            size,
            vec![key(egui::Key::ArrowRight, false)],
        );
        let changed = match name {
            "cable_opacity" => app.rack.cable_presentation().opacity,
            "cable_tension" => app.rack.cable_presentation().tension,
            _ => app.rack.cable_presentation().plug_brightness,
        };
        let original = match name {
            "cable_opacity" => custom().opacity,
            "cable_tension" => custom().tension,
            _ => custom().plug_brightness,
        };
        assert!(changed > original, "{name} keyboard ignored");
        assert_eq!(settings(dir.path())[name].as_f64().unwrap() as f32, changed);
        let at = rail_point(&ctx, name, 0.35);
        click(&ctx, &mut app, at, 3.0, size);
        assert!(
            egui::Popup::is_any_open(&ctx),
            "{name} first click closed View"
        );
        click(&ctx, &mut app, at, 3.15, size);
        let value = match name {
            "cable_opacity" => app.rack.cable_presentation().opacity,
            "cable_tension" => app.rack.cable_presentation().tension,
            _ => app.rack.cable_presentation().plug_brightness,
        };
        assert_eq!(value, reset, "{name} double-click reset");
        assert_eq!(settings(dir.path())[name].as_f64().unwrap() as f32, reset);
        assert_eq!(app.shared.lock().unwrap().snapshot(), before);
        assert!(app.capture_document().equivalent(&document));
    }
}

#[test]
fn plug_slider_zero_and_reenable_are_host_only_and_do_not_change_saved_patch_or_audio() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    let size = egui::vec2(1100.0, 800.0);
    open_view(&ctx, &mut app, size);
    let before = app.shared.lock().unwrap().snapshot();
    let document = app.capture_document();
    let mut expected = [0.0; 257];
    app.patch.compile().unwrap().process_block(&mut expected);
    let left = rail_point(&ctx, "plug_light_brightness", 0.0);
    click(&ctx, &mut app, left, 2.0, size);
    assert_eq!(app.rack.cable_presentation().plug_brightness, 0.0);
    assert_eq!(settings(dir.path())["plug_light_brightness"], 0.0);
    assert!(egui::Popup::is_any_open(&ctx), "zero click closed View");
    let middle = rail_point(&ctx, "plug_light_brightness", 0.5);
    click(&ctx, &mut app, middle, 3.0, size);
    assert!(app.rack.cable_presentation().plug_brightness > 0.0);
    let mut actual = [0.0; 257];
    app.patch.compile().unwrap().process_block(&mut actual);
    assert_eq!(actual, expected);
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert!(app.capture_document().equivalent(&document));
    assert!(app.audio.is_none());
}

#[test]
fn cable_preferences_survive_template_new_example_open_import_revert_recovery_history_and_view_reset(
) {
    for brightness in [0.0, 0.37] {
        let dir = tempfile::tempdir().unwrap();
        let expected =
            Document::new(openpatch_core::demo_patch(), Rack::default().layout()).unwrap();
        {
            let store = Store::new(dir.path()).unwrap();
            store.save_recovery(&expected).unwrap();
            store.save_new("saved.openpatch", &expected).unwrap();
            let template = crate::startup::factory_document();
            let mut layout = template.layout().clone();
            layout["zoom"] = json!(0.75);
            let template = template.with_layout(layout).unwrap();
            store.save_template(&template, None).unwrap();
        }
        std::fs::write(
            dir.path().join("legacy.json"),
            serde_json::to_vec(&expected.patch()).unwrap(),
        )
        .unwrap();
        let mut app = host(dir.path());
        let presentation = CablePresentation {
            plug_brightness: brightness,
            ..custom()
        };
        app.set_cable_presentation(presentation);
        let recovery = app.recovery_names[0].clone();
        app.load_document(&recovery, true);
        assert_eq!(app.rack.cable_presentation(), presentation);
        app.new_from_template(false);
        assert_eq!(app.rack.cable_presentation(), presentation);
        assert_eq!(app.rack.layout()["zoom"], 0.75);
        app.replace_now(Patch::new_v2(), "Untitled");
        assert_eq!(app.rack.cable_presentation(), presentation);
        app.replace_now(openpatch_core::demo_patch(), "Factory example");
        assert_eq!(app.rack.cable_presentation(), presentation);
        app.load_document("saved.openpatch", false);
        assert_eq!(app.rack.cable_presentation(), presentation);
        edit(&mut app, 330.0);
        app.load_document("saved.openpatch", false);
        assert_eq!(app.rack.cable_presentation(), presentation);
        assert!(!app.dirty());
        app.load_document("legacy.json", false);
        assert_eq!(app.rack.cable_presentation(), presentation);
        assert!(app.active_file.is_none());
        edit(&mut app, 440.0);
        app.history(false);
        assert_eq!(app.rack.cable_presentation(), presentation);
        app.history(true);
        assert_eq!(app.rack.cable_presentation(), presentation);
        app.rack.reset_view();
        assert_eq!(app.rack.cable_presentation(), presentation);
        app.save_host_settings();
        let mut reopened = host(dir.path());
        reopened.apply_host_preferences(&settings(dir.path()));
        assert_eq!(reopened.rack.cable_presentation(), presentation);
    }
}

#[test]
fn small_window_view_menu_scrolls_to_every_control_and_bottom_actions_remain_reachable() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    let size = egui::vec2(940.0, 300.0);
    let initial = open_view(&ctx, &mut app, size);
    let view = preset_text(&initial, "Room brightness");
    // Actual wheel events in the popup scroll its clipped contents; the rack
    // behind it must not receive the gesture as zoom/pan or patch editing.
    let before = app.shared.lock().unwrap().snapshot();
    let layout = app.rack.layout();
    let mut output = initial;
    for index in 0..8 {
        output = frame(
            &ctx,
            &mut app,
            2.0 + index as f64 * 0.1,
            size,
            vec![
                egui::Event::PointerMoved(view),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -80.0),
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    let bottom = preset_text(&output, "Shader editor");
    assert!(
        bottom.y > 0.0 && bottom.y < size.y,
        "View menu extends outside small window"
    );
    for key in ["cable_opacity", "cable_tension", "plug_light_brightness"] {
        assert!(control(&ctx, key).rect.height() > 0.0);
    }
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert_eq!(app.rack.layout(), layout);
    click(&ctx, &mut app, bottom, 3.0, size);
    assert!(app.shader_open);
}

#[test]
fn appended_smoke_scenes_use_actual_view_popup_and_fresh_offline_signal_fixture() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    let size = egui::vec2(1100.0, 800.0);
    app.prepare_port_lights_fixture();
    app.smoke_test = Some((dir.path().join("unused.png"), 1930, 78));
    // No graphics context or screenshot events: exercise the actual scene
    // transition and menu render while retaining the real offline DSP snapshot.
    frame(&ctx, &mut app, 0.0, size, vec![]);
    app.smoke(&ctx);
    // A newly opened egui Area has an initial invisible sizing pass, as in
    // open_view(). The native fixture has four settled frames before1940.
    frame(&ctx, &mut app, 0.1, size, vec![]);
    let output = frame(&ctx, &mut app, 0.2, size, vec![]);
    assert!(egui::Popup::is_any_open(&ctx));
    for label in ["Cable opacity", "Cable tension", "Plug light brightness"] {
        preset_text(&output, label);
    }
    app.smoke_test.as_mut().unwrap().1 = 1942;
    app.smoke(&ctx);
    assert_eq!(
        app.rack.cable_presentation(),
        CablePresentation {
            opacity: 0.15,
            tension: 1.0,
            plug_brightness: 1.0,
            ..Default::default()
        }
    );
    assert!(app.title.contains("Clear panels"));
    assert!(!egui::Popup::is_any_open(&ctx));
    assert!(app.patch.cables.len() > 5);
    app.smoke_test.as_mut().unwrap().1 = 1956;
    app.smoke(&ctx);
    assert_eq!(
        app.rack.cable_presentation(),
        CablePresentation {
            opacity: 1.0,
            tension: 0.0,
            plug_brightness: 1.0,
            ..Default::default()
        }
    );
    assert_eq!(app.rack.zoom(), 0.72);
    assert!(app.title.contains("Loose cables"));
    assert!(app.audio.is_none());
}
