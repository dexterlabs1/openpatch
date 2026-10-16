use super::*;

fn frame(
    ctx: &egui::Context,
    app: &mut OpenPatch,
    size: egui::Vec2,
    time: f64,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(time),
            focused: true,
            events,
            ..Default::default()
        },
        |ctx| {
            app.toolbar(ctx);
            let available = ctx.available_rect();
            ctx.data_mut(|data| data.insert_temp(egui::Id::new("toolbar-test-canvas"), available));
            app.global_shortcuts(ctx);
            app.rack_workspace(ctx, None);
            app.audio_settings(ctx);
        },
    )
}

fn rect(ctx: &egui::Context, key: &str) -> egui::Rect {
    ctx.data(|data| data.get_temp(egui::Id::new(key))).unwrap()
}

fn click(ctx: &egui::Context, app: &mut OpenPatch, size: egui::Vec2, at: egui::Pos2, time: f64) {
    for (offset, pressed) in [(0.0, true), (0.05, false)] {
        frame(
            ctx,
            app,
            size,
            time + offset,
            vec![egui::Event::PointerMoved(at), pointer_event(at, pressed)],
        );
    }
}

fn gear(ctx: &egui::Context) -> egui::Rect {
    ctx.data(|data| {
        data.get_temp::<(egui::Rect, egui::LayerId)>(egui::Id::new("audio-settings-anchor"))
    })
    .unwrap()
    .0
}

fn settle(ctx: &egui::Context, app: &mut OpenPatch, size: egui::Vec2, time: f64) {
    for offset in [0.0, 0.1, 0.2] {
        frame(ctx, app, size, time + offset, vec![]);
    }
}

#[test]
fn settings_pointer_dropdown_is_anchored_toggleable_and_dismisses_outside_or_escape() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    let ctx = egui::Context::default();
    let size = egui::vec2(1440.0, 900.0);
    // This sentinel must survive opening: the headless route cannot enumerate hardware.
    app.output_devices = vec!["Headless output fixture".into()];
    settle(&ctx, &mut app, size, 0.0);
    click(&ctx, &mut app, size, gear(&ctx).center(), 1.0);
    settle(&ctx, &mut app, size, 1.2);
    assert!(app.settings_open);
    assert_eq!(app.output_devices, ["Headless output fixture"]);
    let popup = rect(&ctx, "audio-settings-dropdown-rect");
    assert!(popup.top() >= gear(&ctx).bottom());
    assert!((popup.right() - gear(&ctx).right()).abs() < 2.0);
    click(&ctx, &mut app, size, gear(&ctx).center(), 2.0);
    assert!(
        !app.settings_open,
        "The selected gear must still close settings"
    );
    click(&ctx, &mut app, size, gear(&ctx).center(), 3.0);
    settle(&ctx, &mut app, size, 3.2);
    click(&ctx, &mut app, size, egui::pos2(100.0, 750.0), 4.0);
    assert!(!app.settings_open);
    click(&ctx, &mut app, size, gear(&ctx).center(), 5.0);
    settle(&ctx, &mut app, size, 5.2);
    frame(
        &ctx,
        &mut app,
        size,
        6.0,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(!app.settings_open);
}

#[test]
fn nested_device_dropdown_selection_keeps_settings_open_and_outside_still_dismisses() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    app.output_devices = (0..20).map(|i| format!("Fixture output {i}")).collect();
    let ctx = egui::Context::default();
    let size = egui::vec2(1100.0, 800.0);
    settle(&ctx, &mut app, size, 0.0);
    click(&ctx, &mut app, size, gear(&ctx).center(), 1.0);
    settle(&ctx, &mut app, size, 1.2);
    let device_id = ctx
        .data(|data| data.get_temp::<egui::Id>(egui::Id::new("settings-audio-device-id")))
        .unwrap();
    click(
        &ctx,
        &mut app,
        size,
        rect(&ctx, "settings-audio-device-rect").center(),
        2.0,
    );
    settle(&ctx, &mut app, size, 2.2);
    assert!(egui::ComboBox::is_open(&ctx, device_id));
    assert!(app.settings_open);
    // The child's first row is the already selected system default. This exercises
    // a real selection without opening an audio stream from a headless host.
    let child = egui::AreaState::load(&ctx, device_id.with("popup"))
        .unwrap()
        .rect();
    click(
        &ctx,
        &mut app,
        size,
        child.left_top() + egui::vec2(35.0, 15.0),
        3.0,
    );
    assert!(!egui::ComboBox::is_open(&ctx, device_id));
    assert!(
        app.settings_open,
        "Selecting a child row must retain the settings dropdown"
    );
    assert_eq!(app.output_device, None);
    // A control in the parent is still interactive after the child closes.
    let output = frame(&ctx, &mut app, size, 3.2, vec![]);
    let keyboard = preset_text(&output, "Show performance keyboard");
    click(&ctx, &mut app, size, keyboard, 4.0);
    assert!(app.keyboard_open);
    assert!(app.settings_open);
    click(
        &ctx,
        &mut app,
        size,
        rect(&ctx, "settings-audio-device-rect").center(),
        5.0,
    );
    settle(&ctx, &mut app, size, 5.2);
    click(&ctx, &mut app, size, egui::pos2(50.0, 700.0), 6.0);
    assert!(
        !app.settings_open,
        "An outside click must also dismiss an open child"
    );
}

#[test]
fn toolbar_reserves_essential_controls_and_settings_scrolls_in_small_viewports() {
    for width in [700.0, 850.0, 1100.0, 1140.0, 1440.0] {
        let dir = tempfile::tempdir().unwrap();
        let mut app = host(dir.path());
        app.title =
            "A very long project title that must never push actions out of the toolbar".repeat(3);
        let ctx = egui::Context::default();
        let size = egui::vec2(width, 320.0);
        settle(&ctx, &mut app, size, 0.0);
        let controls = [
            rect(&ctx, "toolbar-save-rect"),
            rect(&ctx, "toolbar-play-rect"),
            rect(&ctx, "toolbar-mcp-rect"),
            gear(&ctx),
        ];
        for (i, control) in controls.iter().enumerate() {
            assert!(
                control.left() >= 0.0 && control.right() <= width,
                "{width}: essential control clipped: {control:?}"
            );
            for other in &controls[i + 1..] {
                assert!(
                    !control.intersects(*other),
                    "{width}: essential controls overlap"
                );
            }
        }
        let rack = rect(&ctx, "toolbar-test-canvas");
        if width >= 1140.0 {
            assert!(rack.top() < 60.0, "Wide toolbar should use one band");
        } else {
            assert!(rack.top() >= 70.0 && rack.top() < 100.0);
        }
        click(&ctx, &mut app, size, gear(&ctx).center(), 1.0);
        settle(&ctx, &mut app, size, 1.2);
        let popup = rect(&ctx, "audio-settings-dropdown-rect");
        assert!(app.settings_open);
        assert!(popup.left() >= 0.0 && popup.right() <= width + 0.5);
        assert!(
            popup.top() >= gear(&ctx).bottom() && popup.bottom() <= size.y + 0.5,
            "{width}: settings must fit below the gear: {popup:?}"
        );
    }
}

#[test]
fn settings_owns_rack_pointer_gestures_until_dismissed() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = host(dir.path());
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,"nodes":[{"id":"source","kind":"constant"}]}})
    ));
    app.rack
        .restore_layout(
            &json!({"version":1,"zoom":1.0,"pan":[20.0,20.0],"positions":{"source":[0.0,0.0]}}),
        )
        .unwrap();
    let ctx = egui::Context::default();
    let size = egui::vec2(1100.0, 800.0);
    settle(&ctx, &mut app, size, 0.0);
    click(&ctx, &mut app, size, gear(&ctx).center(), 1.0);
    settle(&ctx, &mut app, size, 1.2);
    let before = app.rack.layout();
    let snapshot = app.shared.lock().unwrap().snapshot();
    let from = rect(&ctx, "toolbar-test-canvas").min + egui::vec2(55.0, 60.0);
    let to = from + egui::vec2(120.0, 0.0);
    let drag = |app: &mut OpenPatch, time: f64| {
        frame(
            &ctx,
            app,
            size,
            time,
            vec![egui::Event::PointerMoved(from), pointer_event(from, true)],
        );
        frame(
            &ctx,
            app,
            size,
            time + 0.1,
            vec![egui::Event::PointerMoved(to)],
        );
        frame(&ctx, app, size, time + 0.2, vec![pointer_event(to, false)]);
    };
    drag(&mut app, 2.0);
    assert_eq!(app.rack.layout(), before);
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    assert!(
        !app.settings_open,
        "Releasing the outside gesture dismisses settings"
    );
    // Dismissal happens after rack rendering. Give egui an enabled-rack pass
    // to register its header hit targets before starting the next gesture.
    settle(&ctx, &mut app, size, 2.3);
    // Prove this is a real draggable header, not an inert blank spot.
    drag(&mut app, 3.0);
    assert_ne!(app.rack.layout(), before);
}
