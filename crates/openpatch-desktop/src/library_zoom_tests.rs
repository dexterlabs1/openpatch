use serde_json::json;

fn catalog(kinds: &[&str]) -> Value {
    let all = openpatch_core::catalog();
    json!({"modules": kinds.iter().map(|kind| modules(&all).find(|module| module["kind"] == *kind).unwrap().clone()).collect::<Vec<_>>()})
}
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::panel_skin::install_fonts(&ctx);
    ctx.style_mut(|style| style.animation_time = 0.0);
    ctx
}
#[allow(clippy::too_many_arguments)]
fn draw(
    ctx: &egui::Context,
    library: &mut ModuleLibrary,
    open: &mut bool,
    catalog: &Value,
    size: Vec2,
    time: f64,
    events: Vec<egui::Event>,
    enabled: bool,
) -> (Option<LibraryAction>, egui::FullOutput) {
    let mut action = None;
    let modifiers = events
        .iter()
        .rev()
        .find_map(|event| match event {
            egui::Event::Key { modifiers, .. } | egui::Event::PointerButton { modifiers, .. } => {
                Some(*modifiers)
            }
            _ => None,
        })
        .unwrap_or_default();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, size)),
            time: Some(time),
            modifiers,
            events,
            focused: true,
            ..Default::default()
        },
        |ctx| {
            action = library.show(ctx, open, catalog, 2, enabled);
            egui::CentralPanel::default().show(ctx, |_ui| {});
        },
    );
    (action, output)
}
fn pointer(point: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
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
fn text(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {label}"))
}
fn foreground_text(ctx: &egui::Context, output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .rev()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                let point = text.pos + text.galley.size() * 0.5;
                (clipped.clip_rect.contains(point)
                    && ctx
                        .layer_id_at(point)
                        .is_some_and(|layer| layer.order == egui::Order::Foreground))
                .then_some(point)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing visible foreground preset {label}"))
}

fn cards(output: &egui::FullOutput) -> Vec<Rect> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect)
                if [Color32::from_rgb(29, 33, 32), Color32::from_rgb(48, 53, 51)]
                    .contains(&rect.fill) =>
            {
                Some(rect.rect)
            }
            _ => None,
        })
        .collect()
}
fn card_viewport(output: &egui::FullOutput) -> Rect {
    output
        .shapes
        .iter()
        .find_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect)
                if [Color32::from_rgb(29, 33, 32), Color32::from_rgb(48, 53, 51)]
                    .contains(&rect.fill) =>
            {
                Some(clipped.clip_rect)
            }
            _ => None,
        })
        .expect("actual scroll-area card background clip")
}

fn panels(output: &egui::FullOutput) -> Vec<(Rect, Rect)> {
    output
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect)
                if [
                    Color32::from_rgb(139, 140, 133),
                    Color32::from_rgb(66, 67, 64),
                    Color32::from_rgb(108, 108, 92),
                ]
                .contains(&rect.fill) =>
            {
                Some((rect.rect, clipped.clip_rect))
            }
            _ => None,
        })
        .collect()
}
fn click(
    ctx: &egui::Context,
    library: &mut ModuleLibrary,
    open: &mut bool,
    catalog: &Value,
    point: egui::Pos2,
    time: f64,
    enabled: bool,
) -> (Option<LibraryAction>, egui::FullOutput) {
    draw(
        ctx,
        library,
        open,
        catalog,
        vec2(940.0, 680.0),
        time,
        vec![egui::Event::PointerMoved(point), pointer(point, true)],
        enabled,
    );
    draw(
        ctx,
        library,
        open,
        catalog,
        vec2(940.0, 680.0),
        time + 0.01,
        vec![pointer(point, false)],
        enabled,
    )
}

#[test]
fn library_zoom_metadata_agrees_with_all_actual_default_panel_sizes_and_assignment_is_not_a_ui_edit(
) {
    let catalog = openpatch_core::catalog();
    assert_eq!(modules(&catalog).count(), 80);
    let stock_lfo = modules(&catalog).find(|module| module["kind"] == "stock_lfo").expect("Stock LFO is covered by library metadata");
    assert_eq!(ModulePreview::new(stock_lfo).size(), vec2(360.0, PANEL_HEIGHT));
    let stock_vco = modules(&catalog).find(|module| module["kind"] == "stock_vco").expect("Stock VCO is covered by library metadata");
    assert_eq!(ModulePreview::new(stock_vco).size(), vec2(360.0, PANEL_HEIGHT));
    let stock_seq3 = modules(&catalog).find(|module| module["kind"] == "stock_seq3").expect("Stock SEQ3 is covered by library metadata");
    assert_eq!(ModulePreview::new(stock_seq3).size(), vec2(780.0, PANEL_HEIGHT));
    let stock_quantizer=modules(&catalog).find(|module|module["kind"]=="stock_quantizer").expect("Stock Quantizer metadata");
    assert_eq!(ModulePreview::new(stock_quantizer).size(),vec2(300.0,PANEL_HEIGHT));
    let stock_process=modules(&catalog).find(|module|module["kind"]=="stock_process").expect("Stock Process metadata");
    assert_eq!(ModulePreview::new(stock_process).size(),vec2(300.0,PANEL_HEIGHT));
    for module in modules(&catalog) {
        let kind = module["kind"].as_str().unwrap();
        let preview = ModulePreview::new(module);
        for zoom in [0.25, 0.5, 1.0, 2.0] {
            let actual = preview.size() * zoom;
            assert_eq!(
                actual,
                vec2(crate::rack::panel_width(kind) * zoom, PANEL_HEIGHT * zoom),
                "{kind} at {zoom}"
            );
        }
    }
    let mut library = ModuleLibrary::default();
    assert_eq!(library.preview_zoom(), 0.5);
    for (value, expected) in [
        (0.0, 0.25),
        (5.0, 2.0),
        (f32::NAN, 0.5),
        (f32::INFINITY, 0.5),
        (f32::NEG_INFINITY, 0.5),
    ] {
        library.set_preview_zoom(value);
        assert_eq!(library.preview_zoom(), expected);
        assert!(!library.take_preview_zoom_changed());
    }
}

#[test]
fn library_zoom_actual_artwork_is_natural_scale_even_when_wider_or_taller_than_small_dock() {
    for kind in ["stock_vca", "seq3", "blank"] {
        let catalog = catalog(&[kind]);
        for zoom in [0.25, 0.5, 1.0, 2.0] {
            let ctx = context();
            let mut library = ModuleLibrary::default();
            let mut open = true;
            library.set_preview_zoom(zoom);
            draw(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                vec2(940.0, 680.0),
                0.0,
                vec![],
                true,
            );
            let (_, output) = draw(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                vec2(940.0, 680.0),
                0.1,
                vec![],
                true,
            );
            let card = cards(&output);
            let panel = panels(&output);
            assert_eq!(card.len(), 1, "{kind} at {zoom}");
            assert_eq!(panel.len(), 1);
            assert!((panel[0].0.width() - crate::rack::panel_width(kind) * zoom).abs() < 0.001);
            assert!((panel[0].0.height() - PANEL_HEIGHT * zoom).abs() < 0.001);
            assert!((card[0].width() - (panel[0].0.width() + 16.0).max(108.0)).abs() < 0.001);
            assert!((card[0].height() - (PANEL_HEIGHT * zoom + 62.0)).abs() < 0.001);
            assert!((panel[0].0.top() - card[0].top() - 30.0).abs() < 0.001);
            assert!((panel[0].0.center().x - card[0].center().x).abs() < 0.001);
            if kind == "seq3" && zoom >= 0.5 {
                assert!(
                    panel[0].0.width() > panel[0].1.width(),
                    "wide hardware must overflow, never refit"
                );
            }
            assert!(library.previews[kind].matches(&catalog["modules"][0]));
            assert!(open);
            assert!(!library.take_preview_zoom_changed());
        }
    }
}

#[test]
fn library_zoom_flow_uses_viewport_after_wide_content_and_does_not_build_offscreen_caches() {
    let catalog = catalog(&["scope", "audio_out", "analog_vca", "stock_vca"]);
    let ctx = context();
    let mut library = ModuleLibrary::default();
    let mut open = true;
    library.set_preview_zoom(0.25);
    draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.0,
        vec![],
        true,
    );
    let (_, output) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.1,
        vec![],
        true,
    );
    let first = cards(&output);
    assert_eq!(first.len(), 4);
    assert!(
        first[1].top() > first[0].top(),
        "first narrow card must wrap after real wide card"
    );
    assert_eq!(
        first[1].top(),
        first[2].top(),
        "two narrow cards share responsive row"
    );
    assert!(first[2].right() <= 940.0);
    // Large zoom creates horizontal overflow. Returning to compact must reflow
    // against the unchanged viewport, not retain the wide content's old extent.
    library.set_preview_zoom(2.0);
    draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.2,
        vec![],
        true,
    );
    library.set_preview_zoom(0.25);
    for index in 0..3 {
        draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            0.3 + f64::from(index) * 0.1,
            vec![],
            true,
        );
    }
    let (_, output) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.7,
        vec![],
        true,
    );
    assert_eq!(
        cards(&output),
        first,
        "stable reflow after horizontal overflow"
    );
    let all = openpatch_core::catalog();
    let ctx = context();
    let mut library = ModuleLibrary::default();
    let mut open = true;
    library.set_preview_zoom(2.0);
    draw(
        &ctx,
        &mut library,
        &mut open,
        &all,
        vec2(940.0, 680.0),
        0.0,
        vec![],
        true,
    );
    assert!(!library.previews.is_empty());
    assert!(
        library.previews.len() < modules(&all).count() / 2,
        "offscreen metadata planning must not instantiate renderers"
    );
}

#[test]
fn library_zoom_popup_selection_is_other_focus_never_adds_or_navigates_cards_and_escape_closes_popup_first(
) {
    let catalog = catalog(&["stock_vca", "seq3"]);
    let ctx = context();
    let mut library = ModuleLibrary::default();
    let mut open = true;
    library.set_favorites(&["seq3".into()], &catalog);
    library.inspect("stock_vca", &catalog);
    // Close inspection for pointer space, preserving its selection for later.
    library.inspected = None;
    draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.0,
        vec![],
        true,
    );
    let (_, initial) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.1,
        vec![],
        true,
    );
    let highlighted = library.highlighted.clone();
    click(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        text(&initial, "50%"),
        1.0,
        true,
    );
    assert!(egui::Popup::is_any_open(&ctx));
    // A new Popup Area first measures invisibly. Locate only after its actual
    // foreground pass; a native VCA also has a background 100% readout.
    let (_, popup) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        1.1,
        vec![],
        true,
    );
    let (action, _) = click(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        foreground_text(&ctx, &popup, "100%"),
        1.2,
        true,
    );
    assert!(action.is_none());
    assert_eq!(library.preview_zoom(), 1.0);
    assert!(library.take_preview_zoom_changed());
    assert!(!library.take_preview_zoom_changed());
    assert_eq!(library.highlighted, highlighted);
    assert_eq!(library.favorites(), vec!["seq3"]);
    assert_eq!(library.focused_last_frame, Some(FocusKind::Other));
    for (index, pressed_key) in [
        egui::Key::ArrowUp,
        egui::Key::ArrowDown,
        egui::Key::ArrowLeft,
        egui::Key::ArrowRight,
    ]
    .into_iter()
    .enumerate()
    {
        let time = 2.0 + index as f64;
        let (action, _) = draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            time,
            vec![key(pressed_key, true)],
            true,
        );
        assert!(action.is_none());
        assert_eq!(library.highlighted, highlighted);
        assert!(
            !ctx.input(|input| input.key_pressed(pressed_key)),
            "arrow must not leak to rack"
        );
        draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            time + 0.1,
            vec![key(pressed_key, false)],
            true,
        );
    }
    let (action, _) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        7.0,
        vec![key(egui::Key::Enter, true)],
        true,
    );
    assert!(action.is_none());
    assert!(egui::Popup::is_any_open(&ctx));
    let (action, _) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        7.1,
        vec![key(egui::Key::Enter, false), key(egui::Key::Escape, true)],
        true,
    );
    assert!(action.is_none());
    assert!(open);
    assert!(!egui::Popup::is_any_open(&ctx));
    assert_eq!(library.highlighted, highlighted);
}

#[test]
fn library_zoom_disabled_popup_cannot_change_zoom_and_reselecting_current_preset_is_not_an_edit() {
    let catalog = catalog(&["stock_vca"]);
    for enabled in [true, false] {
        let ctx = context();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            0.0,
            vec![],
            enabled,
        );
        let (_, output) = draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            0.1,
            vec![],
            enabled,
        );
        click(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            text(&output, "50%"),
            1.0,
            enabled,
        );
        if enabled {
            let (_, popup) = draw(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                vec2(940.0, 680.0),
                1.1,
                vec![],
                true,
            );
            // Current selected text appears both on the button and in the menu;
            // take the menu's last matching text shape.
            let current = popup
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(t) if t.galley.text() == "50%" => {
                        Some(t.pos + t.galley.size() * 0.5)
                    }
                    _ => None,
                })
                .next_back()
                .unwrap();
            click(&ctx, &mut library, &mut open, &catalog, current, 1.2, true);
        } else {
            assert!(!egui::Popup::is_any_open(&ctx));
        }
        assert_eq!(library.preview_zoom(), 0.5);
        assert!(!library.take_preview_zoom_changed());
    }
}

#[test]
fn library_zoom_disabling_an_already_open_popup_closes_the_actual_owned_menu() {
    let catalog = catalog(&["stock_vca"]);
    let ctx = context();
    let mut library = ModuleLibrary::default();
    let mut open = true;
    draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.0,
        vec![],
        true,
    );
    let (_, initial) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.1,
        vec![],
        true,
    );
    click(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        text(&initial, "50%"),
        1.0,
        true,
    );
    draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        1.1,
        vec![],
        true,
    );
    let combo_id = ctx
        .memory(|memory| memory.focused())
        .expect("actual pointer-opened combo focus");
    assert!(egui::ComboBox::is_open(&ctx, combo_id));
    let (action, _) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        1.2,
        vec![],
        false,
    );
    assert!(action.is_none());
    assert!(
        !egui::ComboBox::is_open(&ctx, combo_id),
        "disabled browser closes its actual zoom popup, not a guessed ID"
    );
    assert!(open);
    assert_eq!(library.preview_zoom(), 0.5);
    assert!(!library.take_preview_zoom_changed());
}

#[test]
fn library_zoom_actual_card_response_keeps_click_drag_and_focus_at_all_scales() {
    for kind in ["stock_vca", "random"] {
        let catalog = catalog(&[kind]);
        for zoom in [0.25, 0.5, 0.72, 1.0, 2.0] {
            let ctx = context();
            let mut library = ModuleLibrary::default();
            let mut open = true;
            library.set_preview_zoom(zoom);
            draw(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                vec2(940.0, 680.0),
                0.0,
                vec![],
                true,
            );
            let (_, output) = draw(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                vec2(940.0, 680.0),
                0.1,
                vec![],
                true,
            );
            let card = cards(&output)[0];
            let target = card.min + vec2(12.0, 12.0);
            let (action, _) = click(&ctx, &mut library, &mut open, &catalog, target, 1.0, true);
            assert_eq!(
                action,
                Some(LibraryAction::Add(kind.into())),
                "click at zoom {zoom}"
            );
            assert_eq!(library.focused_last_frame, Some(FocusKind::Tile));
            let response = ctx
                .read_response(ctx.memory(|memory| memory.focused()).unwrap())
                .unwrap();
            assert_eq!(
                response.rect,
                Rect::from_min_max(card.min, card.max - vec2(0.0, 32.0))
            );
            draw(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                vec2(940.0, 680.0),
                2.0,
                vec![egui::Event::PointerMoved(target), pointer(target, true)],
                true,
            );
            let (action, _) = draw(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                vec2(940.0, 680.0),
                2.1,
                vec![egui::Event::PointerMoved(target + vec2(35.0, 24.0))],
                true,
            );
            assert!(action.is_none());
            assert_eq!(
                egui::DragAndDrop::payload::<LibraryDrag>(&ctx).map(|payload| payload.kind.clone()),
                Some(kind.into())
            );
            let (action, _) = draw(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                vec2(940.0, 680.0),
                2.2,
                vec![pointer(target + vec2(35.0, 24.0), false)],
                true,
            );
            assert!(action.is_none(), "drag release cannot become click add");
        }
    }
}

#[test]
fn library_zoom_wide_horizontal_scroll_keeps_header_fixed_and_reflow_reveals_leading_edge() {
    let catalog = catalog(&["seq3"]);
    let ctx = context();
    let mut library = ModuleLibrary::default();
    let mut open = true;
    library.set_preview_zoom(2.0);
    draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.0,
        vec![],
        true,
    );
    let (_, before) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.1,
        vec![],
        true,
    );
    let card = cards(&before)[0];
    let header = text(&before, "Preview");
    let viewport = card_viewport(&before);
    assert!(card.width() > viewport.width());
    draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        1.0,
        vec![
            egui::Event::PointerMoved(pos2(viewport.center().x, viewport.top() + 40.0)),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(-240.0, 0.0),
                modifiers: egui::Modifiers::NONE,
            },
        ],
        true,
    );
    let mut scrolled = before;
    for index in 0..5 {
        scrolled = draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            1.1 + f64::from(index) * 0.1,
            vec![],
            true,
        )
        .1;
    }
    assert!(
        cards(&scrolled)[0].left() < card.left() - 10.0,
        "oversized artwork is reachable by horizontal scrolling"
    );
    assert_eq!(
        text(&scrolled, "Preview"),
        header,
        "header never scrolls with cards"
    );
    library.set_preview_zoom(1.0);
    // Scroll animation has its own duration independent of style.animation_time.
    // Advance beyond its maximum duration plus the next paint of the final offset.
    let settle_frames = (ctx.style().scroll_animation.duration.max / 0.1).ceil() as i32 + 3;
    for index in 0..settle_frames {
        scrolled = draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            2.0 + f64::from(index) * 0.1,
            vec![],
            true,
        )
        .1;
    }
    let revealed = cards(&scrolled)[0];
    assert!(
        revealed.left() >= viewport.left() - 2.0,
        "zoom reflow reveals the card's leading edge instead of centering its overflow: card={revealed:?}, viewport={viewport:?}"
    );
    assert_eq!(library.highlighted.as_deref(), Some("seq3"));
    assert!(library.scroll_to_kind.is_none());
}

#[test]
fn library_zoom_keyboard_open_arrow_or_tab_then_enter_selects_actual_preset_without_adding() {
    for navigation in [egui::Key::ArrowDown, egui::Key::Tab] {
        let catalog = catalog(&["stock_vca", "seq3"]);
        let ctx = context();
        let mut library = ModuleLibrary::default();
        let mut open = true;
        draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            0.0,
            vec![],
            true,
        );
        let (_, initial) = draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            0.1,
            vec![],
            true,
        );
        click(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            text(&initial, "50%"),
            1.0,
            true,
        );
        draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            1.2,
            vec![key(egui::Key::Escape, true)],
            true,
        );
        draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            1.3,
            vec![key(egui::Key::Escape, false)],
            true,
        );
        assert_eq!(
            library.focused_last_frame,
            Some(FocusKind::Other),
            "Escape must restore zoom control ownership before {navigation:?}"
        );
        let highlighted = library.highlighted.clone();
        let (action, _) = draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            2.0,
            vec![key(egui::Key::Enter, true)],
            true,
        );
        assert!(action.is_none());
        assert!(egui::Popup::is_any_open(&ctx));
        draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            2.1,
            vec![key(egui::Key::Enter, false)],
            true,
        );
        let (action, _) = draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            2.2,
            vec![key(navigation, true)],
            true,
        );
        assert!(action.is_none());
        draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            2.3,
            vec![key(navigation, false)],
            true,
        );
        let (action, _) = draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            2.4,
            vec![key(egui::Key::Enter, true)],
            true,
        );
        assert!(
            action.is_none(),
            "{navigation:?} menu selection cannot add a module"
        );
        assert!(
            (library.preview_zoom() - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.00001,
            "{navigation:?} selects next actual preset"
        );
        assert!(library.take_preview_zoom_changed());
        assert_eq!(library.highlighted, highlighted);
        assert!(open);
        assert!(
            !egui::Popup::is_any_open(&ctx),
            "{navigation:?} activated preset closes the owned menu"
        );
    }
}

#[test]
fn library_zoom_tab_and_shift_tab_reveal_offscreen_favorite_and_inspect_at_two_hundred_percent() {
    let catalog = catalog(&["seq3"]);
    let ctx = context();
    let mut library = ModuleLibrary::default();
    let mut open = true;
    library.set_preview_zoom(2.0);
    draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.0,
        vec![],
        true,
    );
    let (_, initial) = draw(
        &ctx,
        &mut library,
        &mut open,
        &catalog,
        vec2(940.0, 680.0),
        0.1,
        vec![],
        true,
    );
    let viewport = card_viewport(&initial);
    let target = cards(&initial)[0].min + vec2(12.0, 12.0);
    let (action, _) = click(&ctx, &mut library, &mut open, &catalog, target, 1.0, true);
    assert_eq!(action, Some(LibraryAction::Add("seq3".into())));
    let tile_id = ctx.memory(|memory| memory.focused()).unwrap();
    let star_id = tile_id.with("favorite");
    let inspect_id = tile_id.with("inspect");
    assert!(!viewport.contains_rect(ctx.read_response(star_id).unwrap().rect));
    for (step, shift, expected) in [
        (0, false, star_id),
        (1, false, inspect_id),
        (2, true, star_id),
    ] {
        let time = 2.0 + f64::from(step);
        let event = |pressed| egui::Event::Key {
            key: egui::Key::Tab,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers {
                shift,
                ..egui::Modifiers::NONE
            },
        };
        let (action, _) = draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            time,
            vec![event(true)],
            true,
        );
        assert!(action.is_none());
        draw(
            &ctx,
            &mut library,
            &mut open,
            &catalog,
            vec2(940.0, 680.0),
            time + 0.1,
            vec![event(false)],
            true,
        );
        for frame in 0..5 {
            let (action, _) = draw(
                &ctx,
                &mut library,
                &mut open,
                &catalog,
                vec2(940.0, 680.0),
                time + 0.2 + f64::from(frame) * 0.1,
                vec![],
                true,
            );
            assert!(action.is_none());
        }
        assert_eq!(
            ctx.memory(|memory| memory.focused()),
            Some(expected),
            "actual Tab traversal"
        );
        let rect = ctx.read_response(expected).unwrap().rect;
        assert!(
            viewport.contains_rect(rect),
            "focused footer must be visible: {rect:?}, viewport={viewport:?}"
        );
        assert_eq!(library.focused_last_frame, Some(FocusKind::Other));
    }
    assert!(library.favorites().is_empty());
    assert!(library.inspected.is_none());
}

#[test]
fn stock_mix_is_a_mixing_module_without_replacing_legacy_mix6() {
    assert_eq!(Category::of_kind("stock_mix"), Category::Mixing);
    assert_eq!(Category::of_kind("mix6"), Category::Mixing);
    let catalog = openpatch_core::catalog();
    for kind in ["stock_mix", "mix6"] {
        assert!(modules(&catalog).any(|module| module["kind"] == kind));
    }
}
