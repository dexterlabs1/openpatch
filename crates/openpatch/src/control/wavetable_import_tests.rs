use super::transport_authorization_tests::{Boundary, Pause};
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

static NEXT_IMPORT_TEST: AtomicU64 = AtomicU64::new(0);

struct ImportFixture {
    directory: PathBuf,
    session: SharedSession,
}
impl ImportFixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!(
            "openpatch-detached-wave-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_IMPORT_TEST.fetch_add(1, Ordering::Relaxed)
        ));
        let mut session = Session::new(&directory).unwrap();
        session
            .execute("add_module", json!({"id":"wt","kind":"wavetable_vco"}))
            .unwrap();
        let samples: Vec<u8> = (0..32)
            .flat_map(|index| {
                ((index as f32 / 32.0) * std::f32::consts::TAU)
                    .sin()
                    .to_le_bytes()
            })
            .collect();
        fs::write(directory.join("source.f32"), samples).unwrap();
        Self {
            directory,
            session: session.into_shared(),
        }
    }
    fn args(&self) -> Value {
        json!({"id":"wt","filename":"source.f32","wave_points":32})
    }
}
impl Drop for ImportFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn import_worker(
    fixture: &ImportFixture,
    authorization: Arc<AtomicBool>,
) -> std::thread::JoinHandle<Result<Value, String>> {
    let session = fixture.session.clone();
    let args = fixture.args();
    std::thread::spawn(move || {
        execute_shared_authorized(&session, "import_wavetable", args, Some(&authorization))
    })
}

#[test]
fn prepared_import_releases_session_lock_and_rejects_concurrent_edit() {
    let fixture = ImportFixture::new();
    let mut pause = Pause::install(&fixture.session, Boundary::WavetableImportPrepared);
    let worker = import_worker(&fixture, Arc::new(AtomicBool::new(true)));
    pause.wait();
    // This lock must succeed while the imported candidate/ticket is retained.
    let live = fixture
        .session
        .try_lock()
        .expect("table preparation must not hold the global Session lock");
    assert!(live.patch().nodes[0].content.is_none());
    drop(live);
    execute_shared(
        &fixture.session,
        "set_parameter",
        json!({"id":"wt","parameter":"tune","value":12}),
    )
    .unwrap();
    let before = fixture.session.lock().unwrap().snapshot();
    pause.release();
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .contains("revision conflict"));
    assert_eq!(fixture.session.lock().unwrap().snapshot(), before);
    assert_eq!(
        fixture.session.lock().unwrap().patch().nodes[0].params["tune"],
        12.0
    );
}

struct ImportBackend(AtomicUsize);
impl LiveModuleEvents for ImportBackend {
    fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
        Err("unsupported".into())
    }
    fn snapshot(&self) -> Value {
        json!({})
    }
    fn sync_patch(&self, _: &Patch, _: u64) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
}

#[test]
fn stopped_transport_rejects_prepared_import_without_history_or_backend_changes() {
    let fixture = ImportFixture::new();
    let backend = Arc::new(ImportBackend(AtomicUsize::new(0)));
    fixture
        .session
        .lock()
        .unwrap()
        .set_live_module_events(Some(backend.clone()))
        .unwrap();
    let baseline_calls = backend.0.load(Ordering::Relaxed);
    let before = fixture.session.lock().unwrap().snapshot();
    let authorization = Arc::new(AtomicBool::new(true));
    let mut pause = Pause::install(&fixture.session, Boundary::WavetableImportPrepared);
    let worker = import_worker(&fixture, authorization.clone());
    pause.wait();
    let live = fixture.session.try_lock().unwrap();
    authorization.store(false, Ordering::Release);
    pause.release();
    drop(live);
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .contains("bridge stopped"));
    assert_eq!(fixture.session.lock().unwrap().snapshot(), before);
    assert_eq!(backend.0.load(Ordering::Relaxed), baseline_calls);
}

#[test]
fn detached_import_commits_once_without_rereading_source_and_noop_preserves_redo() {
    let fixture = ImportFixture::new();
    let before_revision = fixture.session.lock().unwrap().revision();
    let before_undo = fixture.session.lock().unwrap().undo.len();
    let mut pause = Pause::install(&fixture.session, Boundary::WavetableImportPrepared);
    let worker = import_worker(&fixture, Arc::new(AtomicBool::new(true)));
    pause.wait();
    let source = fixture.directory.join("source.f32");
    let original = fs::read(&source).unwrap();
    // A second read during live commit would now fail decoding.
    fs::write(&source, [0_u8]).unwrap();
    pause.release();
    let receipt = worker.join().unwrap().unwrap();
    assert_eq!(receipt["changed"], true);
    assert_eq!(receipt["revision"], before_revision + 1);
    let live = fixture.session.lock().unwrap();
    assert_eq!(live.undo.len(), before_undo + 1);
    let asset_id = receipt["content"]["asset_id"].as_str().unwrap();
    assert_eq!(live.patch().assets[asset_id].sample_count(), 32);
    drop(live);
    fs::write(&source, original).unwrap();
    execute_shared(
        &fixture.session,
        "set_parameter",
        json!({"id":"wt","parameter":"tune","value":12}),
    )
    .unwrap();
    execute_shared(&fixture.session, "undo", json!({})).unwrap();
    let before = fixture.session.lock().unwrap().snapshot();
    assert!(fixture.session.lock().unwrap().can_redo());
    let result = execute_shared(&fixture.session, "import_wavetable", fixture.args()).unwrap();
    assert_eq!(result["changed"], false);
    assert_eq!(fixture.session.lock().unwrap().snapshot(), before);
    assert!(fixture.session.lock().unwrap().can_redo());
}

#[test]
fn no_op_import_still_rechecks_authorization_after_preparation() {
    let fixture = ImportFixture::new();
    execute_shared(&fixture.session, "import_wavetable", fixture.args()).unwrap();
    let before = fixture.session.lock().unwrap().snapshot();
    let authorization = Arc::new(AtomicBool::new(true));
    let mut pause = Pause::install(&fixture.session, Boundary::WavetableImportPrepared);
    let worker = import_worker(&fixture, authorization.clone());
    pause.wait();
    authorization.store(false, Ordering::Release);
    pause.release();
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .contains("bridge stopped"));
    assert_eq!(fixture.session.lock().unwrap().snapshot(), before);
}

#[test]
fn no_op_import_still_rechecks_revision_after_preparation() {
    let fixture = ImportFixture::new();
    execute_shared(&fixture.session, "import_wavetable", fixture.args()).unwrap();
    let mut pause = Pause::install(&fixture.session, Boundary::WavetableImportPrepared);
    let worker = import_worker(&fixture, Arc::new(AtomicBool::new(true)));
    pause.wait();
    execute_shared(
        &fixture.session,
        "set_parameter",
        json!({"id":"wt","parameter":"tune","value":7}),
    )
    .unwrap();
    let before = fixture.session.lock().unwrap().snapshot();
    pause.release();
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .contains("revision conflict"));
    assert_eq!(fixture.session.lock().unwrap().snapshot(), before);
}
