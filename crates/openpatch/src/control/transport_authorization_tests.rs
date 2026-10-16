//! Per-session, one-shot test barriers; never compiled into a production build.
use super::*;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boundary {
    Dispatch,
    ConstructionRegistration,
    WavetableImportPrepared,
}

struct Hook {
    id: u64,
    session: usize,
    boundary: Boundary,
    reached: Sender<()>,
    resume: Receiver<()>,
}

static HOOKS: Mutex<Vec<Hook>> = Mutex::new(Vec::new());
static NEXT_HOOK: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Pause {
    id: u64,
    // Retain identity until cleanup; pointer reuse cannot match another test.
    _session: SharedSession,
    reached: Receiver<()>,
    resume: Option<Sender<()>>,
}

impl Pause {
    pub(crate) fn install(session: &SharedSession, boundary: Boundary) -> Self {
        let id = NEXT_HOOK.fetch_add(1, Ordering::Relaxed);
        let (reached_tx, reached) = mpsc::channel();
        let (resume, resume_rx) = mpsc::channel();
        HOOKS.lock().unwrap().push(Hook {
            id,
            session: Arc::as_ptr(session) as usize,
            boundary,
            reached: reached_tx,
            resume: resume_rx,
        });
        Self {
            id,
            _session: session.clone(),
            reached,
            resume: Some(resume),
        }
    }

    pub(crate) fn wait(&self) {
        self.reached
            .recv_timeout(Duration::from_secs(5))
            .expect("request did not reach its commit boundary");
    }

    pub(crate) fn release(&mut self) {
        if let Some(resume) = self.resume.take() {
            let _ = resume.send(());
        }
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        HOOKS.lock().unwrap().retain(|hook| hook.id != self.id);
        self.release();
    }
}

pub(crate) fn pause_if_requested(session: &SharedSession, boundary: Boundary) {
    let hook = {
        let mut hooks = HOOKS.lock().unwrap();
        hooks
            .iter()
            .position(|hook| {
                hook.session == Arc::as_ptr(session) as usize && hook.boundary == boundary
            })
            .map(|index| hooks.remove(index))
    };
    if let Some(hook) = hook {
        if hook.reached.send(()).is_ok() {
            hook.resume
                .recv_timeout(Duration::from_secs(5))
                .expect("test did not release the paused request");
        }
    }
}

pub(crate) fn construction_request(revision: u64) -> Value {
    json!({"expected_revision":revision,"interval_ms":2000,"steps":[
        {"label":"Add after preflight","edits":[{"tool":"add_module","arguments":{"id":"late","kind":"constant"}}]}
    ]})
}

fn session() -> (std::path::PathBuf, SharedSession) {
    let directory = std::env::temp_dir().join(format!(
        "openpatch-authorization-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_HOOK.fetch_add(1, Ordering::Relaxed)
    ));
    let session = Session::new(&directory).unwrap().into_shared();
    (directory, session)
}

#[test]
fn stopped_transport_rejects_a_mutation_waiting_for_the_session_lock() {
    let (directory, session) = session();
    let authorization = Arc::new(AtomicBool::new(true));
    let mut pause = Pause::install(&session, Boundary::Dispatch);
    let worker_session = session.clone();
    let worker_authorization = authorization.clone();
    let worker = std::thread::spawn(move || {
        execute_shared_authorized(
            &worker_session,
            "add_module",
            json!({"id":"late","kind":"constant"}),
            Some(&worker_authorization),
        )
    });
    pause.wait();
    let live = session.lock().unwrap();
    let before = live.snapshot();
    authorization.store(false, Ordering::Release);
    pause.release();
    drop(live);
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .contains("bridge stopped"));
    assert_eq!(before, session.lock().unwrap().snapshot());
    // Native/stdin dispatch has no local-bridge token and remains authorized.
    execute_shared(
        &session,
        "add_module",
        json!({"id":"stdio","kind":"constant"}),
    )
    .unwrap();
    assert_eq!(session.lock().unwrap().revision(), 1);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stopped_transport_cannot_register_a_plan_after_detached_preflight() {
    let (directory, session) = session();
    let authorization = Arc::new(AtomicBool::new(true));
    let mut pause = Pause::install(&session, Boundary::ConstructionRegistration);
    let worker_session = session.clone();
    let worker_authorization = authorization.clone();
    let worker = std::thread::spawn(move || {
        execute_shared_authorized(
            &worker_session,
            "start_construction",
            construction_request(0),
            Some(&worker_authorization),
        )
    });
    pause.wait(); // Detached preflight has passed; registration has not locked.
    let live = session.lock().unwrap();
    let before = live.snapshot();
    authorization.store(false, Ordering::Release);
    pause.release();
    drop(live);
    assert!(worker
        .join()
        .unwrap()
        .unwrap_err()
        .contains("bridge stopped"));
    let live = session.lock().unwrap();
    assert_eq!(live.snapshot(), before);
    assert_eq!(live.construction_snapshot()["status"], "idle");
    drop(live);
    let stdio = execute_shared(&session, "start_construction", construction_request(0)).unwrap();
    execute_shared(
        &session,
        "cancel_construction",
        json!({"plan_id":stdio["plan_id"]}),
    )
    .unwrap();
    std::fs::remove_dir_all(directory).unwrap();
}
