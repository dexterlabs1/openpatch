// Offline product fixture. Never creates an audio device or changes the saved template.
#[derive(Clone, Default)]
struct NoiseSmokeCapture {
    index: usize,
    warm_draws: u8,
    pending: Option<crate::smoke_schedule::Receipt>,
    query_sent: bool,
}

impl OpenPatch {
    fn prepare_stock_noise_fixture(&mut self) -> openpatch_core::ScopeSnapshot {
        let patch: Patch =
            serde_json::from_str(include_str!("../../../examples/stock-noise-resonator.json"))
                .expect("bundled Stock Noise resonator");
        let mut engine = patch.compile().expect("Stock Noise resonator compiles");
        engine.set_port_lights_enabled(true);
        let mut audio = vec![[0.0; 2]; 96_032];
        engine.process_audio_block(&mut audio);
        assert!(audio
            .iter()
            .flatten()
            .all(|v| v.is_finite() && v.abs() < 1.0));
        assert!(audio.iter().any(|v| v[0].abs() > 0.00001));
        let scope = engine
            .module_scope(engine.resolve_module_scope("scope").unwrap())
            .unwrap()
            .clone();
        assert_eq!((scope.x_channels, scope.y_channels), (1, 1));
        assert!(scope.x[0].iter().any(|v| v.max.abs() > 0.001));
        assert!(scope.y[0].iter().any(|v| v.max.abs() > 0.00001));
        let lights = patch
            .nodes
            .iter()
            .filter_map(|node| {
                let handle = engine.resolve_module_lights(&node.id).ok()?;
                Some((node.id.clone(), engine.module_lights(handle)?))
            })
            .collect();
        self.smoke_motion_origin = None;
        self.keyboard_open = false;
        self.settings_open = false;
        self.audio_performance_open = false;
        self.select_analysis(None);
        execute_shared(&self.shared, "replace_patch", json!({"patch":patch})).unwrap();
        self.sync();
        self.replace_rack(Rack::default());
        self.rack
            .restore_layout(&Self::stock_noise_layout())
            .unwrap();
        self.rack.set_agent_follow(false);
        self.rack.set_module_states(lights, BTreeMap::new());
        self.rack
            .set_scope_states(BTreeMap::from([("scope".into(), Arc::new(scope.clone()))]));
        self.rack.set_port_lights(Some(
            crate::audio::offline_frame(&patch, &engine, self.revision).unwrap(),
        ));
        self.browser_open = false;
        self.title = "Noise into matter".into();
        self.status = "Actual offline Engine · pink excitation through VCA into sixteen mechanical modes · Scope X: white noise, Y: body velocity · no live audio".into();
        scope
    }

    // Append captures only after all 139 existing receipts have been saved.
    // The old logical-frame schedule and its <2960 guard remain unchanged.
    fn smoke_noise_tail(&mut self, ctx: &egui::Context, path: &Path, captures: usize) {
        if captures == 142 {
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let id = egui::Id::new("native-smoke-noise-tail");
        let mut stage = ctx
            .data(|d| d.get_temp::<NoiseSmokeCapture>(id))
            .unwrap_or_default();
        if stage.index != captures {
            assert!(stage.pending.is_none());
            stage = NoiseSmokeCapture {
                index: captures,
                ..Default::default()
            };
            match captures {
                139 => {
                    egui::Popup::close_all(ctx);
                    self.library = ModuleLibrary::default();
                    self.prepare_stock_noise_fixture();
                    self.rack.set_room_brightness(1.0);
                }
                140 => {
                    self.rack.set_room_brightness(0.0);
                    self.status = "Stock Noise · lights out · actual cable voltage lights and Scope remain emissive · no invented Noise module lamps".into();
                }
                141 => {
                    self.rack.set_room_brightness(1.0);
                    self.library = ModuleLibrary::default();
                    self.library.set_preview_zoom(0.7);
                    self.browse();
                    assert!(self.library.inspect("stock_noise", &self.catalog));
                    self.library.request_focus();
                    self.title = "Stock Noise · shared browser panel".into();
                }
                _ => panic!("Unexpected Noise tail capture"),
            }
        } else if let Some(expected) = stage.pending {
            let image = ctx.input(|i| {
                i.events.iter().find_map(|event| {
                    let egui::Event::Screenshot {
                        image, user_data, ..
                    } = event
                    else {
                        return None;
                    };
                    let token = user_data
                        .data
                        .as_ref()
                        .and_then(|d| d.downcast_ref::<crate::smoke_schedule::Receipt>())
                        .copied();
                    expected.accepts(token, captures).then(|| image.clone())
                })
            });
            if let Some(image) = image {
                let name = match captures {
                    139 => "rack-stock-noise.png",
                    140 => "rack-stock-noise-lights-out.png",
                    141 => "stock-noise-library.png",
                    _ => unreachable!(),
                };
                let target = path.with_file_name(name);
                image::save_buffer(
                    &target,
                    image.as_raw(),
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                )
                .expect("Save Noise screenshot");
                eprintln!(
                    "Native smoke: capture {}/142 at frame 2959: {}",
                    captures + 1,
                    target.display()
                );
                self.smoke_test.as_mut().unwrap().2 = captures + 1;
                stage.pending = None;
                if captures == 141 {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        } else if stage.warm_draws < 2 {
            stage.warm_draws += 1;
        } else {
            let receipt = crate::smoke_schedule::Receipt {
                frame: 2959,
                capture_index: captures,
            };
            stage.pending = Some(receipt);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
                receipt,
            )));
        }
        ctx.data_mut(|d| d.insert_temp(id, stage));
    }
}
