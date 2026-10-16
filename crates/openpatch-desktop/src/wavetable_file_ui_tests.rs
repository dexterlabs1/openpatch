use super::*;

fn wave_file_host(path: &Path) -> OpenPatch {
    let mut app = host(path);
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,
        "nodes":[{"id":"wave","kind":"wavetable_lfo"}]}})
    ));
    app
}

fn raw_wave(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

fn open_wave_file(app: &mut OpenPatch, mode: FileMode) {
    let command = if mode == FileMode::WavetableImport {
        "open_wavetable_import"
    } else {
        "open_wavetable_export"
    };
    assert!(app.command(
        command,
        json!({"id":"wave","expected_revision":app.revision})
    ));
    assert!(app.file_mode == Some(mode));
}

fn wait_for_wave_import(app: &mut OpenPatch) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.wavetable_import.is_some() {
        app.poll_wavetable_import();
        assert!(Instant::now() < deadline, "wavetable worker timed out");
        if app.wavetable_import.is_some() {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

fn draw_wave_file(app: &mut OpenPatch, events: Vec<egui::Event>) -> egui::FullOutput {
    let ctx = egui::Context::default();
    let _ = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            focused: true,
            ..Default::default()
        },
        |ctx| app.files_window(ctx),
    );
    ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1000.0, 800.0),
            )),
            focused: true,
            events,
            ..Default::default()
        },
        |ctx| app.files_window(ctx),
    )
}

fn file_labels(output: &egui::FullOutput) -> Vec<String> {
    fn visit(shape: &egui::Shape, labels: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(text) => labels.push(text.galley.text().into()),
            egui::Shape::Vec(children) => {
                for child in children {
                    visit(child, labels);
                }
            }
            _ => {}
        }
    }
    let mut labels = Vec::new();
    for shape in &output.shapes {
        visit(&shape.shape, &mut labels);
    }
    labels
}

#[test]
fn wavetable_file_commands_open_guarded_dialogs_and_list_regular_supported_sources() {
    let directory = tempfile::tempdir().unwrap();
    for filename in [
        "float.f32",
        "integer.s16",
        "upper.WAV",
        "hidden.json",
        ".private.f32",
    ] {
        std::fs::write(directory.path().join(filename), []).unwrap();
    }
    std::fs::create_dir(directory.path().join("directory.f32")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        directory.path().join("float.f32"),
        directory.path().join("linked.f32"),
    )
    .unwrap();
    let mut app = wave_file_host(directory.path());
    let before = app.shared.lock().unwrap().snapshot();
    open_wave_file(&mut app, FileMode::WavetableImport);
    assert_eq!(app.files, ["float.f32", "integer.s16", "upper.WAV"]);
    let target = app.wavetable_file_target.as_ref().unwrap();
    assert_eq!(target.id, "wave");
    assert_eq!(target.revision, app.revision);
    assert_eq!(target.current_points, 1024);
    assert!(target.wave_points.is_empty());
    let labels = file_labels(&draw_wave_file(&mut app, vec![]));
    assert!(labels.iter().any(|label| label == &app.workspace));
    assert!(labels.iter().any(|label| label.contains("Wave points")));
    assert!(labels.iter().any(|label| label.contains(".s24/.i24")));
    assert!(!app.library_enabled());
    app.keyboard_open = true;
    assert!(!app.performance_keyboard_enabled(&egui::Context::default(), true));
    open_wave_file(&mut app, FileMode::WavetableExport);
    assert!(app.filename.ends_with(".f32"));
    let labels = file_labels(&draw_wave_file(&mut app, vec![]));
    assert!(labels
        .iter()
        .any(|label| label.contains("preserves every source sample bit")));
    assert!(labels.iter().any(|label| label.contains("PCM16 and clips")));
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
}

#[test]
fn wavetable_import_override_and_exact_export_use_real_tools_and_refuse_overwrite() {
    let directory = tempfile::tempdir().unwrap();
    let mut samples = vec![0.125_f32; 65];
    samples[..4].copy_from_slice(&[0.0, -0.0, 2.5, -3.0]);
    let bytes = raw_wave(&samples);
    std::fs::write(directory.path().join("source.f32"), &bytes).unwrap();
    let mut app = wave_file_host(directory.path());
    let revision = app.revision;
    let history_before = app.shared.lock().unwrap().snapshot()["history"]["undo_steps"]
        .as_u64()
        .unwrap();
    open_wave_file(&mut app, FileMode::WavetableImport);
    app.filename = "source.f32".into();
    app.wavetable_file_target.as_mut().unwrap().wave_points = "32".into();
    app.save_or_load(FileMode::WavetableImport);
    assert!(app.wavetable_import.is_some());
    wait_for_wave_import(&mut app);
    assert!(app.file_mode.is_none(), "{}", app.status);
    assert!(app.wavetable_file_target.is_none());
    assert_eq!(app.revision, revision + 1);
    let content = app.patch.nodes[0].content.as_ref().unwrap();
    assert!(matches!(
        content,
        openpatch_core::ModuleContent::Wavetable {
            wave_points: 32,
            ..
        }
    ));
    assert_eq!(
        app.shared.lock().unwrap().snapshot()["history"]["undo_steps"],
        history_before + 1
    );
    let snapshot = app.shared.lock().unwrap().snapshot();
    open_wave_file(&mut app, FileMode::WavetableExport);
    app.filename = "exact.f32".into();
    app.save_or_load(FileMode::WavetableExport);
    assert!(app.file_mode.is_none(), "{}", app.status);
    assert_eq!(
        std::fs::read(directory.path().join("exact.f32")).unwrap(),
        bytes
    );
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    open_wave_file(&mut app, FileMode::WavetableExport);
    app.filename = "exact.f32".into();
    app.save_or_load(FileMode::WavetableExport);
    assert!(app.file_mode == Some(FileMode::WavetableExport));
    assert_eq!(
        std::fs::read(directory.path().join("exact.f32")).unwrap(),
        bytes
    );
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
    app.filename = "clipped.wav".into();
    app.save_or_load(FileMode::WavetableExport);
    assert!(app.file_mode.is_none(), "{}", app.status);
    let wav = std::fs::read(directory.path().join("clipped.wav")).unwrap();
    let decoded = openpatch_core::wavetable_file::decode_wavetable(&wav, "wav").unwrap();
    assert_eq!(decoded.samples.len(), 65);
    assert!(decoded.samples[2] <= 1.0 && decoded.samples[2] > 0.99);
    assert_eq!(decoded.samples[3], -1.0);
    assert_eq!(app.shared.lock().unwrap().snapshot(), snapshot);
}

#[test]
fn wavetable_auto_geometry_uses_wav_hint_then_keeps_current_length_for_raw() {
    let directory = tempfile::tempdir().unwrap();
    let samples = vec![0.25_f32; 64];
    let wav = openpatch_core::wavetable_file::encode_wavetable_wav(&samples, 32).unwrap();
    std::fs::write(directory.path().join("hint.wav"), wav).unwrap();
    std::fs::write(directory.path().join("source.f32"), raw_wave(&samples)).unwrap();
    let mut app = wave_file_host(directory.path());
    for filename in ["hint.wav", "source.f32"] {
        open_wave_file(&mut app, FileMode::WavetableImport);
        app.filename = filename.into();
        assert!(app
            .wavetable_file_target
            .as_ref()
            .unwrap()
            .wave_points
            .is_empty());
        app.save_or_load(FileMode::WavetableImport);
        wait_for_wave_import(&mut app);
        assert!(app.file_mode.is_none(), "{}", app.status);
        assert!(matches!(
            app.patch.nodes[0].content,
            Some(openpatch_core::ModuleContent::Wavetable {
                wave_points: 32,
                ..
            })
        ));
    }
}

#[test]
fn wavetable_file_apply_keeps_opening_revision_across_intervening_edits() {
    for mode in [FileMode::WavetableImport, FileMode::WavetableExport] {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("source.f32"), raw_wave(&[0.25; 64])).unwrap();
        let mut app = wave_file_host(directory.path());
        open_wave_file(&mut app, mode);
        let opening_revision = app.revision;
        app.filename = if mode == FileMode::WavetableImport {
            "source.f32"
        } else {
            "new.f32"
        }
        .into();
        app.wavetable_file_target.as_mut().unwrap().wave_points = "32".into();
        assert!(app.command(
            "set_parameter",
            json!({"id":"wave","parameter":"position","value":0.5})
        ));
        assert_eq!(
            app.wavetable_file_target.as_ref().unwrap().revision,
            opening_revision
        );
        let changed = app.shared.lock().unwrap().snapshot();
        app.save_or_load(mode);
        wait_for_wave_import(&mut app);
        assert!(app.file_mode == Some(mode));
        assert!(
            app.status.to_ascii_lowercase().contains("revision"),
            "{}",
            app.status
        );
        assert_eq!(app.shared.lock().unwrap().snapshot(), changed);
        assert!(!directory.path().join("new.f32").exists());
    }
}

#[test]
fn wavetable_file_cancel_and_invalid_geometry_leave_patch_and_files_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    let bytes = raw_wave(&[0.25; 64]);
    std::fs::write(directory.path().join("source.f32"), &bytes).unwrap();
    let mut app = wave_file_host(directory.path());
    let before = app.shared.lock().unwrap().snapshot();
    open_wave_file(&mut app, FileMode::WavetableImport);
    app.filename = "source.f32".into();
    app.wavetable_file_target.as_mut().unwrap().wave_points = "thirty-two".into();
    app.save_or_load(FileMode::WavetableImport);
    assert!(app.file_mode == Some(FileMode::WavetableImport));
    assert!(app.status.contains("Wave points must be an integer"));
    draw_wave_file(
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(app.file_mode.is_none());
    assert!(app.wavetable_file_target.is_none());
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert_eq!(
        std::fs::read(directory.path().join("source.f32")).unwrap(),
        bytes
    );
    assert!(!app.command(
        "open_wavetable_export",
        json!({"id":"wave","expected_revision":app.revision-1})
    ));
    assert!(app.file_mode.is_none());
    assert!(app.status.contains("Patch changed"));
}

#[test]
fn wavetable_background_import_returns_while_session_busy_and_cancels_before_publication() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join("source.f32"), raw_wave(&[0.25; 64])).unwrap();
    let mut app = wave_file_host(directory.path());
    open_wave_file(&mut app, FileMode::WavetableImport);
    app.filename = "source.f32".into();
    app.wavetable_file_target.as_mut().unwrap().wave_points = "32".into();
    let before = app.shared.lock().unwrap().snapshot();
    let shared = app.shared.clone();
    // The worker cannot even acquire its first session snapshot yet. Starting
    // and dismissing the modal must still finish without joining that worker.
    let session = shared.lock().unwrap();
    app.save_or_load(FileMode::WavetableImport);
    let allowed = app.wavetable_import.as_ref().unwrap().allowed.clone();
    assert!(allowed.load(Ordering::Acquire));
    draw_wave_file(
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(app.file_mode.is_none());
    assert!(!allowed.load(Ordering::Acquire));
    assert_eq!(session.snapshot(), before);
    drop(session);
    wait_for_wave_import(&mut app);
    assert_eq!(app.shared.lock().unwrap().snapshot(), before);
    assert_eq!(app.status, "Wavetable import cancelled");
    assert!(app.file_mode.is_none());
}

#[test]
fn wavetable_import_caps_workers_and_ignores_late_results_after_cancel() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = wave_file_host(directory.path());
    open_wave_file(&mut app, FileMode::WavetableImport);
    app.filename = "source.f32".into();
    let (sender, receiver) = mpsc::channel();
    let allowed = Arc::new(AtomicBool::new(true));
    app.wavetable_import = Some(WavetableImportJob {
        result: receiver,
        allowed: allowed.clone(),
        filename: "source.f32".into(),
    });
    let labels = file_labels(&draw_wave_file(&mut app, vec![]));
    assert!(labels.iter().any(|label| label == "Preparing wavetable…"));
    app.save_or_load(FileMode::WavetableImport);
    assert!(Arc::ptr_eq(
        &app.wavetable_import.as_ref().unwrap().allowed,
        &allowed
    ));
    assert!(app.shared.try_lock().is_ok());
    // A control edit can finish while the UI is waiting on the result channel.
    assert!(app.command(
        "set_parameter",
        json!({"id":"wave","parameter":"position","value":0.5})
    ));
    draw_wave_file(
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    assert!(!allowed.load(Ordering::Acquire));
    open_wave_file(&mut app, FileMode::WavetableImport);
    app.filename = "new-source.f32".into();
    app.save_or_load(FileMode::WavetableImport);
    assert!(
        Arc::ptr_eq(&app.wavetable_import.as_ref().unwrap().allowed, &allowed),
        "a cancelled worker still occupies the single worker slot until it exits"
    );
    let status = app.status.clone();
    sender.send(Err("late cancellation error".into())).unwrap();
    app.poll_wavetable_import();
    assert!(app.wavetable_import.is_none());
    assert!(app.file_mode == Some(FileMode::WavetableImport));
    assert_eq!(app.filename, "new-source.f32");
    assert_eq!(app.status, status);
}

#[test]
fn wavetable_worker_disconnection_is_retryable_and_app_drop_revokes_authorization() {
    let directory = tempfile::tempdir().unwrap();
    let mut app = wave_file_host(directory.path());
    open_wave_file(&mut app, FileMode::WavetableImport);
    let (sender, receiver) = mpsc::channel();
    app.wavetable_import = Some(WavetableImportJob {
        result: receiver,
        allowed: Arc::new(AtomicBool::new(true)),
        filename: "source.f32".into(),
    });
    drop(sender);
    app.poll_wavetable_import();
    assert!(app.wavetable_import.is_none());
    assert!(app.file_mode == Some(FileMode::WavetableImport));
    assert!(app.status.contains("worker stopped unexpectedly"));
    let (_sender, receiver) = mpsc::channel();
    let allowed = Arc::new(AtomicBool::new(true));
    app.wavetable_import = Some(WavetableImportJob {
        result: receiver,
        allowed: allowed.clone(),
        filename: "source.f32".into(),
    });
    drop(app);
    assert!(!allowed.load(Ordering::Acquire));
}

#[test]
fn native_wavetable_preset_dialog_captures_saves_reads_and_applies_embedded_samples() {
    let directory = tempfile::tempdir().unwrap();
    let mut samples = vec![0.125_f32; 65];
    samples[..4].copy_from_slice(&[0.0, -0.0, 2.5, -3.0]);
    let source = raw_wave(&samples);
    std::fs::write(directory.path().join("source.f32"), &source).unwrap();
    let mut app = wave_file_host(directory.path());
    open_wave_file(&mut app, FileMode::WavetableImport);
    app.filename = "source.f32".into();
    app.wavetable_file_target.as_mut().unwrap().wave_points = "32".into();
    app.save_or_load(FileMode::WavetableImport);
    wait_for_wave_import(&mut app);
    assert!(app.file_mode.is_none(), "{}", app.status);
    assert!(app.command(
        "set_parameter",
        json!({"id":"wave","parameter":"position","value":0.375})
    ));
    assert!(app.command("set_bypass", json!({"id":"wave","bypassed":true})));
    let content = app.patch.nodes[0].content.clone();
    let mut save = PresetDialog::new(
        "wave".into(),
        "wavetable_lfo".into(),
        app.revision,
        PresetMode::Save,
    );
    save.name = "embedded-wave".into();
    save.await_release = false;
    app.preset_dialog = Some(save);
    let ctx = egui::Context::default();
    preset_frame(&ctx, &mut app, 0.0, vec![]);
    let output = preset_frame(&ctx, &mut app, 0.2, vec![]);
    let reviewed = app
        .preset_dialog
        .as_ref()
        .unwrap()
        .capture
        .clone()
        .expect("portable wave capture");
    let assets = reviewed["assets"].as_object().unwrap();
    assert_eq!(assets.len(), 1);
    let encoded = assets.values().next().unwrap()["data"].as_str().unwrap();
    assert!(!encoded.is_empty());
    let labels = file_labels(&output);
    assert!(labels
        .iter()
        .any(|label| label.contains("controls, bypass and an embedded table")));
    assert!(labels
        .iter()
        .any(|label| label == "Embedded wavetable: source.f32"));
    assert!(labels.iter().any(|label| label == "32 points per wave"));
    assert!(
        labels.iter().all(|label| !label.contains(encoded)),
        "encoded sample payload is never preview text"
    );
    let save_button = preset_text(&output, "Save preset");
    preset_click(&ctx, &mut app, save_button, 1.0);
    assert!(app.preset_dialog.is_none());
    assert!(directory.path().join("embedded-wave.opreset").exists());

    // Remove the entire previous asset map. Applying the saved preset must be
    // self-contained rather than relying on samples left in the live patch.
    assert!(app.command(
        "replace_patch",
        json!({"patch":{"version":2,
        "nodes":[{"id":"wave","kind":"wavetable_lfo"}]}})
    ));
    assert!(app.patch.assets.is_empty());
    let layout = app.rack.layout();
    let mut browse = PresetDialog::new(
        "wave".into(),
        "wavetable_lfo".into(),
        app.revision,
        PresetMode::Browse,
    );
    browse.await_release = false;
    app.preset_dialog = Some(browse);
    preset_frame(&ctx, &mut app, 2.0, vec![]);
    let output = preset_frame(&ctx, &mut app, 2.2, vec![]);
    let filename = preset_text(&output, "embedded-wave.opreset");
    preset_click(&ctx, &mut app, filename, 3.0);
    assert_eq!(
        app.preset_dialog
            .as_ref()
            .unwrap()
            .preview
            .as_ref()
            .unwrap(),
        &reviewed
    );
    let output = preset_frame(&ctx, &mut app, 4.0, vec![]);
    let apply = preset_text(&output, "Apply preset");
    preset_click(&ctx, &mut app, apply, 5.0);
    assert!(app.preset_dialog.is_none(), "{}", app.status);
    assert_eq!(app.patch.nodes[0].content, content);
    assert_eq!(app.patch.nodes[0].params["position"], 0.375);
    assert!(app.patch.nodes[0].bypassed);
    assert_eq!(app.rack.layout(), layout);
    assert_eq!(app.patch.assets.len(), 1);
    execute_shared(
        &app.shared,
        "export_wavetable",
        json!({
            "id":"wave","filename":"roundtrip.f32","expected_revision":app.revision
        }),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(directory.path().join("roundtrip.f32")).unwrap(),
        source
    );
}
