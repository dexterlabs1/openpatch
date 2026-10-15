//! Reviewed, progressive graph transactions on a control thread. No DSP/UI work.
use super::*;
use std::sync::{Condvar, Weak};
use std::thread;
use std::time::Duration;

const MAX_STEPS: usize = 64;
const MAX_STEP_EDITS: usize = 8;
const MAX_RECEIPT_TARGETS: usize = 128;

/// Presentation evidence for one actual committed transaction, never a plan's
/// predicted edits. Values remain typed until the additive JSON snapshot edge.
#[derive(Clone, Debug)]
struct CommitReceipt {
    plan_id: u64,
    step_index: usize,
    before_revision: u64,
    revision: u64,
    committed_at: Instant,
    invalidate_existing_cables: bool,
    label: String,
    new_modules: Vec<ModuleArrival>,
    new_cables: Vec<CableArrival>,
    parameter_changes: Vec<ParameterChange>,
    moved_cables: Vec<CableMove>,
}

#[derive(Clone, Debug)]
struct ModuleArrival {
    id: String,
    kind: String,
}

#[derive(Clone, Debug)]
struct CableArrival {
    id: String,
    from: String,
    output: String,
    to: String,
    input: String,
}

/// Exact committed routes. Cosmetic color/stack order are intentionally absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CableRoute {
    pub(super) from: String,
    output: String,
    pub(super) to: String,
    input: String,
}

impl From<&Cable> for CableRoute {
    fn from(cable: &Cable) -> Self {
        Self {
            from: cable.from.clone(),
            output: cable.output.clone(),
            to: cable.to.clone(),
            input: cable.input.clone(),
        }
    }
}

impl CableRoute {
    fn to_json(&self) -> Value {
        json!({"from":self.from,"output":self.output,"to":self.to,"input":self.input})
    }
}

#[derive(Clone, Debug)]
struct CableMove {
    id: String,
    before: CableRoute,
    after: CableRoute,
}

impl CableMove {
    fn to_json(&self) -> Value {
        json!({"id":self.id,"before":self.before.to_json(),"after":self.after.to_json()})
    }
}

#[derive(Clone, Debug)]
pub(super) struct MoveReceipt {
    before_revision: u64,
    revision: u64,
    committed_at: Instant,
    moved_cables: Vec<CableMove>,
}

impl MoveReceipt {
    pub(super) fn from_commit(
        before_revision: u64,
        revision: u64,
        before: &Patch,
        after: &Patch,
        effects: &EditEffects,
    ) -> Option<Self> {
        if effects.cable_moves.is_empty() {
            return None;
        }
        let old: BTreeMap<_, _> = before
            .cables
            .iter()
            .filter_map(|c| Some((c.id.as_ref()?, c)))
            .collect();
        let retired: BTreeSet<_> = effects.retired.iter().collect();
        let moved_cables: Vec<_> = after
            .cables
            .iter()
            .filter_map(|cable| {
                let id = cable.id.as_ref()?;
                let touched = effects.cable_moves.get(id)?;
                if touched.iter().any(|node| retired.contains(node)) {
                    return None;
                }
                // All endpoint module instances must survive, including temporary
                // endpoints visited by a coalesced sequence of moves.
                if touched.iter().any(|id| {
                    let old = before.nodes.iter().find(|node| &node.id == id);
                    let new = after.nodes.iter().find(|node| &node.id == id);
                    !matches!((old, new), (Some(a), Some(b)) if a.kind == b.kind)
                }) {
                    return None;
                }
                let before = CableRoute::from(*old.get(id)?);
                let after = CableRoute::from(cable);
                (before != after).then(|| CableMove {
                    id: id.clone(),
                    before,
                    after,
                })
            })
            .take(MAX_RECEIPT_TARGETS)
            .collect();
        (!moved_cables.is_empty()).then(|| Self {
            before_revision,
            revision,
            committed_at: Instant::now(),
            moved_cables,
        })
    }

    fn to_json(&self) -> Value {
        let age_ms = u64::try_from(self.committed_at.elapsed().as_millis()).unwrap_or(u64::MAX);
        json!({"before_revision":self.before_revision,"revision":self.revision,"age_ms":age_ms,
            "moved_cables":self.moved_cables.iter().map(CableMove::to_json).collect::<Vec<_>>()})
    }
}

#[derive(Clone, Debug)]
struct ParameterChange {
    id: String,
    kind: String,
    parameter: String,
    old_effective: f32,
    new_effective: f32,
}

impl CommitReceipt {
    fn from_commit(
        plan: &Plan,
        revision: u64,
        before: &Patch,
        after: &Patch,
        history: &HistoryEntry,
        moves: Option<&MoveReceipt>,
    ) -> Self {
        let mut receipt = Self {
            plan_id: plan.id,
            step_index: plan.completed,
            before_revision: plan.expected_revision,
            revision,
            committed_at: Instant::now(),
            // Endpoint moves also retire cable state. Clearing old decorative
            // cues conservatively avoids transferring them to a new lifecycle.
            invalidate_existing_cables: !history.retired_cables.is_empty(),
            label: plan.steps[plan.completed].label.clone(),
            new_modules: Vec::new(),
            new_cables: Vec::new(),
            parameter_changes: Vec::new(),
            moved_cables: Vec::new(),
        };
        let before_nodes: BTreeMap<_, _> =
            before.nodes.iter().map(|node| (&node.id, node)).collect();
        let retired: BTreeSet<_> = history.retired_modules.iter().collect();
        // Count all arrivals, including those outside the presentation budget,
        // so an overflow arrival can never become a continuous parameter cue.
        let arrivals: BTreeSet<_> = after
            .nodes
            .iter()
            .filter(|node| {
                retired.contains(&node.id)
                    || before_nodes
                        .get(&node.id)
                        .is_none_or(|old| old.kind != node.kind)
            })
            .map(|node| &node.id)
            .collect();
        let mut remaining = MAX_RECEIPT_TARGETS;
        for node in &after.nodes {
            if remaining == 0 {
                break;
            }
            if arrivals.contains(&node.id) {
                receipt.new_modules.push(ModuleArrival {
                    id: node.id.clone(),
                    kind: node.kind.clone(),
                });
                remaining -= 1;
            }
        }
        let before_cables: BTreeSet<_> = before
            .cables
            .iter()
            .filter_map(|cable| cable.id.as_ref())
            .collect();
        for cable in &after.cables {
            if remaining == 0 {
                break;
            }
            let Some(id) = &cable.id else { continue };
            // Retirement also includes endpoint moves. Keep cable arrivals to
            // IDs absent before the step; explicit moves have separate evidence.
            if !before_cables.contains(id) {
                receipt.new_cables.push(CableArrival {
                    id: id.clone(),
                    from: cable.from.clone(),
                    output: cable.output.clone(),
                    to: cable.to.clone(),
                    input: cable.input.clone(),
                });
                remaining -= 1;
            }
        }
        if let Some(moves) = moves.filter(|moves| {
            moves.revision == revision && moves.before_revision == plan.expected_revision
        }) {
            receipt
                .moved_cables
                .extend(moves.moved_cables.iter().take(remaining).cloned());
            remaining -= receipt.moved_cables.len();
        }
        if remaining == 0 {
            return receipt;
        }
        let catalog = catalog();
        let descriptors: BTreeMap<_, _> = catalog["modules"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|module| Some((module["kind"].as_str()?, module["params"].as_object()?)))
            .collect();
        // Module ID then catalog parameter key order is deterministic regardless
        // of the order, number or spelling of individual edits in the step.
        let nodes: BTreeMap<_, _> = after.nodes.iter().map(|node| (&node.id, node)).collect();
        for (id, node) in nodes {
            if arrivals.contains(id) {
                continue;
            }
            let Some(old) = before_nodes.get(id) else {
                continue;
            };
            let Some(params) = descriptors.get(node.kind.as_str()) else {
                continue;
            };
            for (parameter, descriptor) in *params {
                if remaining == 0 {
                    return receipt;
                }
                if descriptor["hidden"] == true {
                    continue;
                }
                let Some(default) = descriptor["default"].as_f64().map(|value| value as f32) else {
                    continue;
                };
                let old_effective = old.params.get(parameter).copied().unwrap_or(default);
                let new_effective = node.params.get(parameter).copied().unwrap_or(default);
                if old_effective != new_effective {
                    receipt.parameter_changes.push(ParameterChange {
                        id: id.clone(),
                        kind: node.kind.clone(),
                        parameter: parameter.clone(),
                        old_effective,
                        new_effective,
                    });
                    remaining -= 1;
                }
            }
        }
        receipt
    }

    fn to_json(&self) -> Value {
        self.to_json_at(Instant::now())
    }

    fn to_json_at(&self, now: Instant) -> Value {
        let age_ms = u64::try_from(now.saturating_duration_since(self.committed_at).as_millis())
            .unwrap_or(u64::MAX);
        json!({"plan_id":self.plan_id,"step_index":self.step_index,
            "before_revision":self.before_revision,"revision":self.revision,"label":self.label,
            "age_ms":age_ms,"invalidate_existing_cables":self.invalidate_existing_cables,
            "new_modules":self.new_modules.iter().map(|module| json!({"id":module.id,"kind":module.kind})).collect::<Vec<_>>(),
            "new_cables":self.new_cables.iter().map(|cable| json!({"id":cable.id,"from":cable.from,"output":cable.output,"to":cable.to,"input":cable.input})).collect::<Vec<_>>(),
            "moved_cables":self.moved_cables.iter().map(CableMove::to_json).collect::<Vec<_>>(),
            "parameter_changes":self.parameter_changes.iter().map(|change| json!({"id":change.id,"kind":change.kind,"parameter":change.parameter,"old_effective":change.old_effective,"new_effective":change.new_effective})).collect::<Vec<_>>()})
    }
}

#[derive(Default)]
pub(super) struct Construction {
    next_id: u64,
    plan: Option<Plan>,
    pub(super) last_move: Option<MoveReceipt>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Running,
    Paused,
    Cancelled,
    Completed,
    Conflicted,
    Failed,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
            Self::Conflicted => "conflicted",
            Self::Failed => "failed",
        }
    }
    fn active(self) -> bool {
        matches!(self, Self::Running | Self::Paused)
    }
}

struct Step {
    label: String,
    edits: Value,
}

struct Plan {
    id: u64,
    title: String,
    steps: Vec<Step>,
    status: Status,
    completed: usize,
    base_revision: u64,
    expected_revision: u64,
    last_revision: Option<u64>,
    last_label: Option<String>,
    last_commit: Option<CommitReceipt>,
    error: Option<String>,
    interval_ms: u64,
    next_due: Instant,
    wake: Arc<Wake>,
}

#[derive(Default)]
struct Wake {
    generation: Mutex<u64>,
    changed: Condvar,
}

impl Wake {
    fn notify(&self) {
        let mut generation = self.generation.lock().unwrap_or_else(|e| e.into_inner());
        *generation = generation.wrapping_add(1);
        self.changed.notify_all();
    }
}

impl Construction {
    pub(super) fn wake(&self) {
        if let Some(plan) = &self.plan {
            plan.wake.notify();
        }
    }

    pub(super) fn close(&mut self) {
        if let Some(plan) = &mut self.plan {
            if plan.status.active() {
                plan.status = Status::Cancelled;
                plan.error = Some("session is closing".into());
            }
            plan.wake.notify();
        }
    }

    fn refresh(&mut self, revision: u64, closing: bool) {
        if closing {
            self.close();
            return;
        }
        if let Some(plan) = &mut self.plan {
            if plan.status.active() && plan.expected_revision != revision {
                plan.status = Status::Conflicted;
                plan.error = Some(conflict(plan.expected_revision, revision));
                plan.wake.notify();
            }
        }
    }

    fn check_available(&self) -> Result<(), String> {
        if self.plan.as_ref().is_some_and(|plan| plan.status.active()) {
            Err("a construction plan is already running or paused; cancel it before starting another".into())
        } else {
            Ok(())
        }
    }
}

fn conflict(expected: u64, current: u64) -> String {
    format!("construction revision conflict: expected {expected}, current {current}; submit a newly reviewed plan")
}

impl Session {
    /// Cheap progress metadata only; never serialize the patch or consult audio.
    pub fn construction_snapshot(&self) -> Value {
        let Some(plan) = &self.construction.plan else {
            return json!({"status":"idle","current_revision":self.revision,"last_commit":null,"last_move":self.construction.last_move.as_ref().map(MoveReceipt::to_json)});
        };
        // Report a conflict immediately, even before the woken worker has been
        // scheduled to record that terminal transition.
        let (status, error) = if plan.status.active() && plan.expected_revision != self.revision {
            (
                Status::Conflicted,
                Some(conflict(plan.expected_revision, self.revision)),
            )
        } else {
            (plan.status, plan.error.clone())
        };
        json!({"plan_id":plan.id,"status":status.as_str(),"title":plan.title,
            "completed_steps":plan.completed,"total_steps":plan.steps.len(),
            "expected_revision":plan.expected_revision,"base_revision":plan.base_revision,
            "current_revision":self.revision,"last_revision":plan.last_revision,
            "last_move":self.construction.last_move.as_ref().map(MoveReceipt::to_json),
            "last_label":plan.last_label,"last_commit":plan.last_commit.as_ref().map(CommitReceipt::to_json),
            "next_label":plan.steps.get(plan.completed).map(|step| &step.label),
            "error":error,"interval_ms":plan.interval_ms})
    }

    pub(super) fn detached_snapshot(&self) -> Self {
        Self {
            patch: self.patch.clone(),
            revision: self.revision,
            state_id: self.state_id,
            workspace: self.workspace.clone(),
            undo: VecDeque::new(),
            redo: Vec::new(),
            history_group: None,
            closing: self.closing,
            live_module_events: None,
            construction: Construction::default(),
            midi_learn_worker: false,
            midi_learn_committed: BTreeSet::new(),
        }
    }

    pub(super) fn construction_command(
        &mut self,
        name: &str,
        args: &Value,
    ) -> Result<Value, String> {
        self.construction.refresh(self.revision, self.closing);
        let requested = args
            .get("plan_id")
            .map(|value| {
                value
                    .as_u64()
                    .filter(|id| *id > 0)
                    .ok_or("plan_id must be a positive integer")
            })
            .transpose()?;
        if requested.is_some_and(|id| {
            self.construction
                .plan
                .as_ref()
                .is_none_or(|plan| plan.id != id)
        }) {
            return Err("construction plan_id does not identify the latest retained plan".into());
        }
        if name == "inspect_construction" {
            return Ok(self.construction_snapshot());
        }
        let resume_revision = if name == "resume_construction" {
            Some(
                expected_revision(args)?
                    .ok_or("expected_revision is required to resume construction")?,
            )
        } else {
            None
        };
        let plan = self
            .construction
            .plan
            .as_mut()
            .ok_or("no construction plan exists")?;
        match name {
            "pause_construction" => match plan.status {
                Status::Running => plan.status = Status::Paused,
                Status::Paused => {}
                _ => return Err("only a running construction plan can be paused".into()),
            },
            "resume_construction" => {
                if plan.status != Status::Paused {
                    return Err("only a paused construction plan can be resumed; conflicted plans require a newly reviewed plan".into());
                }
                let expected = resume_revision.unwrap();
                if expected != self.revision || plan.expected_revision != self.revision {
                    plan.status = Status::Conflicted;
                    plan.error = Some(conflict(expected, self.revision));
                    plan.wake.notify();
                    return Err(plan.error.clone().unwrap());
                }
                plan.status = Status::Running;
                plan.next_due = Instant::now() + Duration::from_millis(plan.interval_ms);
            }
            "cancel_construction" => {
                if plan.status == Status::Completed {
                    return Err("a completed construction plan cannot be cancelled".into());
                }
                plan.status = Status::Cancelled;
            }
            _ => return Err(format!("unknown construction command: {name}")),
        }
        plan.wake.notify();
        Ok(self.construction_snapshot())
    }
}

pub(super) fn start(
    session: &SharedSession,
    args: Value,
    authorization: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Value, String> {
    validate_arguments(&tools(), "start_construction", &args)?;
    let base_revision =
        expected_revision(&args)?.ok_or("expected_revision is required to start construction")?;
    let title = optional_string(&args, "title", "Patch construction")?.to_owned();
    if title.len() > 128 {
        return Err("construction title exceeds 128 UTF-8 bytes".into());
    }
    let interval_ms = match args.get("interval_ms") {
        None => 500,
        Some(value) => value
            .as_u64()
            .filter(|n| (100..=2000).contains(n))
            .ok_or("interval_ms must be an integer in 100..2000")?,
    };
    let input = args["steps"].as_array().ok_or("steps must be an array")?;
    if input.is_empty() || input.len() > MAX_STEPS {
        return Err("construction steps must contain 1..64 steps".into());
    }
    let mut steps = Vec::with_capacity(input.len());
    let mut edit_count = 0;
    for (index, input) in input.iter().enumerate() {
        let parsed = (|| {
            let object = input.as_object().ok_or("step must be an object")?;
            if object.keys().any(|key| key != "label" && key != "edits") {
                return Err("step accepts only label and edits".into());
            }
            let label = string(input, "label")?.to_owned();
            if label.len() > 160 {
                return Err("step label exceeds 160 UTF-8 bytes".into());
            }
            let edits = input["edits"]
                .as_array()
                .ok_or("step edits must be an array")?;
            if edits.is_empty() || edits.len() > MAX_STEP_EDITS {
                return Err("each construction step must contain 1..8 edits".into());
            }
            edit_count += edits.len();
            if edit_count > MAX_EDITS {
                return Err("construction exceeds 256 total edits".into());
            }
            Ok(Step {
                label,
                edits: input["edits"].clone(),
            })
        })();
        steps.push(
            parsed.map_err(|error: String| format!("construction step {}: {error}", index + 1))?,
        );
    }
    let mut detached = {
        let mut live = session.lock().map_err(|_| "session lock poisoned")?;
        check_transport_authorization(authorization)?;
        live.check_open()?;
        live.check_revision(Some(base_revision))?;
        let (revision, closing) = (live.revision, live.closing);
        live.construction.refresh(revision, closing);
        live.construction.check_available()?;
        live.detached_snapshot()
    };
    // Every step's transaction validates its graph, so a later step cannot
    // repair an invalid graph that would have been exposed to the live rack.
    // This snapshot has no backend, file operations, plan, or shared lock.
    for (index, step) in steps.iter().enumerate() {
        detached
            .execute(
                "apply_edits",
                json!({"edits":step.edits,"expected_revision":detached.revision}),
            )
            .map_err(|error| {
                format!("construction step {} ({}): {error}", index + 1, step.label)
            })?;
    }
    #[cfg(test)]
    transport_authorization_tests::pause_if_requested(
        session,
        transport_authorization_tests::Boundary::ConstructionRegistration,
    );
    let mut live = session.lock().map_err(|_| "session lock poisoned")?;
    check_transport_authorization(authorization)?;
    live.check_open()?;
    live.check_revision(Some(base_revision))?;
    let (revision, closing) = (live.revision, live.closing);
    live.construction.refresh(revision, closing);
    live.construction.check_available()?;
    let id = live
        .construction
        .next_id
        .checked_add(1)
        .ok_or("construction plan IDs exhausted")?;
    let wake = Arc::new(Wake::default());
    let worker_wake = wake.clone();
    let weak = Arc::downgrade(session);
    // Spawn while holding the session mutex: the worker cannot observe state
    // until registration is complete. Failure leaves the prior plan intact.
    thread::Builder::new()
        .name("openpatch-construction".into())
        .spawn(move || worker(weak, id, worker_wake))
        .map_err(|error| format!("start construction worker: {error}"))?;
    live.construction.wake();
    live.construction.next_id = id;
    live.construction.plan = Some(Plan {
        id,
        title,
        steps,
        status: Status::Running,
        completed: 0,
        base_revision,
        expected_revision: base_revision,
        last_revision: None,
        last_label: None,
        last_commit: None,
        error: None,
        interval_ms,
        next_due: Instant::now() + Duration::from_millis(interval_ms),
        wake,
    });
    Ok(live.construction_snapshot())
}

fn worker(session: Weak<Mutex<Session>>, id: u64, wake: Arc<Wake>) {
    loop {
        let observed = *wake.generation.lock().unwrap_or_else(|e| e.into_inner());
        let due = {
            let Some(shared) = session.upgrade() else {
                return;
            };
            let Ok(mut live) = shared.lock() else {
                return;
            };
            let (revision, closing) = (live.revision, live.closing);
            live.construction.refresh(revision, closing);
            let Some(plan) = live
                .construction
                .plan
                .as_ref()
                .filter(|plan| plan.id == id && plan.status.active())
            else {
                return;
            };
            if plan.status == Status::Paused {
                None
            } else if Instant::now() < plan.next_due {
                Some(plan.next_due)
            } else {
                let edits = plan.steps[plan.completed].edits.clone();
                let label = plan.steps[plan.completed].label.clone();
                let expected = plan.expected_revision;
                let before = live.patch.clone();
                let result = live.execute(
                    "apply_edits",
                    json!({"edits":edits,"expected_revision":expected}),
                );
                let revision = live.revision;
                // apply_edits always commits one ordinary, ungrouped history
                // entry. Its retirement metadata describes actual edit effects,
                // including same-ID module replacement within this step.
                let receipt = result.as_ref().ok().map(|_| {
                    CommitReceipt::from_commit(
                        live.construction.plan.as_ref().unwrap(),
                        revision,
                        &before,
                        &live.patch,
                        live.undo
                            .back()
                            .expect("successful construction has history"),
                        live.construction.last_move.as_ref(),
                    )
                });
                let plan = live.construction.plan.as_mut().unwrap();
                match result {
                    Ok(_) => {
                        plan.completed += 1;
                        plan.expected_revision = revision;
                        plan.last_revision = Some(revision);
                        plan.last_label = Some(label);
                        plan.last_commit = receipt;
                        if plan.completed == plan.steps.len() {
                            plan.status = Status::Completed;
                        } else {
                            plan.next_due =
                                Instant::now() + Duration::from_millis(plan.interval_ms);
                        }
                    }
                    Err(error) => {
                        plan.status = Status::Failed;
                        plan.error = Some(error);
                    }
                }
                if !plan.status.active() {
                    return;
                }
                Some(plan.next_due)
            }
        }; // Release both the session mutex and its upgraded Arc before waiting.
        let generation = wake.generation.lock().unwrap_or_else(|e| e.into_inner());
        if *generation != observed {
            continue;
        }
        match due {
            Some(due) => {
                let timeout = due.saturating_duration_since(Instant::now());
                drop(
                    wake.changed
                        .wait_timeout(generation, timeout)
                        .unwrap_or_else(|e| e.into_inner()),
                );
            }
            None => {
                drop(
                    wake.changed
                        .wait(generation)
                        .unwrap_or_else(|e| e.into_inner()),
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "construction_receipt_tests.rs"]
mod receipt_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    pub(super) fn shared() -> SharedSession {
        Session::new(&std::env::temp_dir()).unwrap().into_shared()
    }

    fn initial_step() -> Value {
        json!({"label":"Add source","edits":[
            {"tool":"add_module","arguments":{"id":"source","kind":"constant","params":{"value":0}}},
            {"tool":"set_output","arguments":{"id":"source"}}
        ]})
    }

    fn parameter_step(value: u32) -> Value {
        json!({"label":format!("Set {value}"),"edits":[
            {"tool":"set_parameter","arguments":{"id":"source","parameter":"value","value":value}}
        ]})
    }

    fn request(session: &SharedSession, interval: u64) -> Value {
        json!({"expected_revision":session.lock().unwrap().revision,"interval_ms":interval,
            "title":"Reviewed test patch","steps":[initial_step(),parameter_step(1),parameter_step(2)]})
    }

    pub(super) fn snapshot(session: &SharedSession) -> Value {
        session.lock().unwrap().construction_snapshot()
    }

    pub(super) fn until(session: &SharedSession, check: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let value = snapshot(session);
            if check(&value) {
                return value;
            }
            assert!(
                Instant::now() < deadline,
                "construction did not settle: {value}"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    // Lifecycle tests drive the real worker across explicit deadlines instead
    // of racing the OS scheduler between two production-sized intervals.
    // Production admission/preflight is covered separately above and below.
    const CONTROLLED_INTERVAL_MS: u64 = 86_400_000;

    fn controlled_worker() -> SharedSession {
        let session = shared();
        controlled_steps(
            session,
            vec![initial_step(), parameter_step(1), parameter_step(2)],
        )
    }

    pub(super) fn controlled_steps(session: SharedSession, steps: Vec<Value>) -> SharedSession {
        let mut live = session.lock().unwrap();
        let base_revision = live.revision;
        let wake = Arc::new(Wake::default());
        live.construction.next_id = 1;
        live.construction.plan = Some(Plan {
            id: 1,
            title: "Controlled worker".into(),
            steps: steps
                .into_iter()
                .map(|step| Step {
                    label: step["label"].as_str().unwrap().to_owned(),
                    edits: step["edits"].clone(),
                })
                .collect(),
            status: Status::Running,
            completed: 0,
            base_revision,
            expected_revision: base_revision,
            last_revision: None,
            last_label: None,
            last_commit: None,
            error: None,
            interval_ms: CONTROLLED_INTERVAL_MS,
            next_due: Instant::now() + Duration::from_millis(CONTROLLED_INTERVAL_MS),
            wake: wake.clone(),
        });
        let weak = Arc::downgrade(&session);
        thread::spawn(move || worker(weak, 1, wake));
        drop(live);
        session
    }

    pub(super) fn advance_one(session: &SharedSession) -> Value {
        let completed = {
            let mut live = session.lock().unwrap();
            let plan = live.construction.plan.as_mut().unwrap();
            assert_eq!(plan.status, Status::Running);
            plan.next_due = Instant::now();
            plan.wake.notify();
            plan.completed + 1
        };
        until(session, |value| value["completed_steps"] == completed)
    }

    fn assert_worker_exits_without_another_step(session: &SharedSession, revision: u64) {
        let wake = {
            let mut live = session.lock().unwrap();
            let plan = live.construction.plan.as_mut().unwrap();
            assert!(!plan.status.active());
            // An overdue deadline must not bypass cancellation or closure.
            plan.next_due = Instant::now();
            plan.wake.notify();
            plan.wake.clone()
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        // One reference belongs to the plan and one to this test. The worker
        // releases its third reference only after it has exited completely.
        while Arc::strong_count(&wake) != 2 {
            assert!(Instant::now() < deadline, "terminal worker did not exit");
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(snapshot(session)["current_revision"], revision);
    }

    #[derive(Default)]
    struct Backend {
        calls: AtomicUsize,
    }
    impl LiveModuleEvents for Backend {
        fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            Err("unexpected live action".into())
        }
        fn snapshot(&self) -> Value {
            self.calls.fetch_add(1, Ordering::Relaxed);
            json!({})
        }
        fn sync_patch(&self, _: &Patch, _: u64) {
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
        fn retire_modules(&self, _: &[String]) {
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
        fn initialize_modules(&self, _: &[String]) {
            self.calls.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn all_intermediate_graphs_preflight_without_live_history_or_backend_effects() {
        let session = shared();
        let backend = Arc::new(Backend::default());
        session
            .lock()
            .unwrap()
            .set_live_module_events(Some(backend.clone()))
            .unwrap();
        backend.calls.store(0, Ordering::Relaxed);
        let before = session.lock().unwrap().snapshot();
        let invalid = [
            json!([initial_step(), {"label":"Bad later step","edits":[{"tool":"set_parameter","arguments":{"id":"missing","parameter":"value","value":1}}]}]),
            json!([
                {"label":"Invalid exposed graph","edits":[{"tool":"set_output","arguments":{"id":"source"}}]},
                initial_step()
            ]),
        ];
        for steps in invalid {
            assert!(execute_shared(
                &session,
                "start_construction",
                json!({"expected_revision":0,"steps":steps})
            )
            .unwrap_err()
            .contains("construction step"));
            assert_eq!(session.lock().unwrap().snapshot(), before);
            assert_eq!(snapshot(&session)["status"], "idle");
            assert_eq!(backend.calls.load(Ordering::Relaxed), 0);
        }
        // A successful preflight likewise has no live effects before admission.
        let receipt =
            execute_shared(&session, "start_construction", request(&session, 2000)).unwrap();
        execute_shared(
            &session,
            "pause_construction",
            json!({"plan_id":receipt["plan_id"]}),
        )
        .unwrap();
        assert_eq!(backend.calls.load(Ordering::Relaxed), 0);
        assert_eq!(session.lock().unwrap().snapshot(), before);
    }

    #[test]
    fn each_step_commits_one_real_revision_and_individual_undo_entry() {
        let session = shared();
        let receipt =
            execute_shared(&session, "start_construction", request(&session, 100)).unwrap();
        assert_eq!(receipt["base_revision"], 0);
        assert_eq!(receipt["completed_steps"], 0);
        assert!(receipt["last_revision"].is_null());
        assert_eq!(receipt["next_label"], "Add source");
        let done = until(&session, |s| s["status"] == "completed");
        assert_eq!(done["completed_steps"], 3);
        assert_eq!(done["last_label"], "Set 2");
        assert_eq!(done["last_revision"], 3);
        assert_eq!(done["expected_revision"], 3);
        assert!(done["next_label"].is_null());
        assert!(done["error"].is_null());
        let mut live = session.lock().unwrap();
        assert_eq!(live.revision, 3);
        assert_eq!(live.undo.len(), 3);
        assert!(live.history_group.is_none());
        live.execute("undo", json!({})).unwrap();
        assert_eq!(live.patch.nodes[0].params["value"], 1.0);
        live.execute("undo", json!({})).unwrap();
        assert_eq!(live.patch.nodes[0].params["value"], 0.0);
        live.execute("undo", json!({})).unwrap();
        assert!(live.patch.nodes.is_empty());
        assert_eq!(live.revision, 6);
        assert_eq!(live.construction_snapshot()["status"], "completed");
    }

    #[test]
    fn pause_and_resume_serialize_and_schedule_a_fresh_interval() {
        let session = controlled_worker();
        let paused = execute_shared(&session, "pause_construction", json!({"plan_id":1})).unwrap();
        assert_eq!(paused["status"], "paused");
        assert_eq!(paused["completed_steps"], 0);
        {
            let mut live = session.lock().unwrap();
            // Resume must replace an expired paused deadline with a full new
            // interval. Check the actual deadline under the session lock, so
            // scheduler delays cannot turn this into a sub-100ms race.
            let plan = live.construction.plan.as_mut().unwrap();
            plan.next_due = Instant::now() - Duration::from_secs(1);
            plan.interval_ms = 100;
            let before = Instant::now();
            let resumed = live
                .execute(
                    "resume_construction",
                    json!({"plan_id":1,"expected_revision":0}),
                )
                .unwrap();
            let after = Instant::now();
            assert_eq!(resumed["status"], "running");
            assert_eq!(resumed["completed_steps"], 0);
            let plan = live.construction.plan.as_mut().unwrap();
            assert!(plan.next_due >= before + Duration::from_millis(100));
            assert!(plan.next_due <= after + Duration::from_millis(100));
            plan.interval_ms = CONTROLLED_INTERVAL_MS;
            plan.next_due = Instant::now() + Duration::from_millis(CONTROLLED_INTERVAL_MS);
        }
        advance_one(&session);
        advance_one(&session);
        assert_eq!(advance_one(&session)["status"], "completed");
    }

    #[test]
    fn cancel_retains_a_committed_prefix_and_stops_later_steps() {
        let session = controlled_worker();
        assert_eq!(advance_one(&session)["last_revision"], 1);
        let cancelled =
            execute_shared(&session, "cancel_construction", json!({"plan_id":1})).unwrap();
        assert_eq!(cancelled["status"], "cancelled");
        assert_eq!(cancelled["completed_steps"], 1);
        assert_worker_exits_without_another_step(&session, 1);
        let mut live = session.lock().unwrap();
        assert_eq!(live.patch.nodes.len(), 1);
        live.execute("undo", json!({})).unwrap();
        assert!(live.patch.nodes.is_empty());
    }

    #[test]
    fn external_graph_edits_undo_redo_and_layout_conflict_without_rebase() {
        for action in ["edit", "undo", "redo", "layout"] {
            let session = shared();
            session
                .lock()
                .unwrap()
                .execute("apply_edits", json!({"edits":initial_step()["edits"]}))
                .unwrap();
            if action == "redo" {
                session.lock().unwrap().execute("undo", json!({})).unwrap();
            }
            let base = session.lock().unwrap().revision;
            let steps = if action == "redo" {
                json!([initial_step(), parameter_step(1)])
            } else {
                json!([parameter_step(1), parameter_step(2)])
            };
            let receipt = execute_shared(
                &session,
                "start_construction",
                json!({"expected_revision":base,"steps":steps,"interval_ms":100}),
            )
            .unwrap();
            execute_shared(
                &session,
                "pause_construction",
                json!({"plan_id":receipt["plan_id"]}),
            )
            .unwrap();
            {
                let mut live = session.lock().unwrap();
                match action {
                    "edit" => {
                        live.execute(
                            "set_parameter",
                            json!({"id":"source","parameter":"value","value":7}),
                        )
                        .unwrap();
                    }
                    "undo" | "redo" => {
                        live.execute(action, json!({})).unwrap();
                    }
                    "layout" => {
                        live.record_layout_checkpoint(base).unwrap();
                    }
                    _ => unreachable!(),
                }
                assert_eq!(
                    live.construction_snapshot()["status"],
                    "conflicted",
                    "{action}"
                );
            }
            let current = session.lock().unwrap().revision;
            assert!(execute_shared(
                &session,
                "resume_construction",
                json!({"plan_id":receipt["plan_id"],"expected_revision":current})
            )
            .unwrap_err()
            .contains("newly reviewed"));
            thread::sleep(Duration::from_millis(130));
            assert_eq!(snapshot(&session)["completed_steps"], 0);
            assert_eq!(snapshot(&session)["current_revision"], current);
        }
        // The same conflict protection applies while running before its first step.
        let session = shared();
        execute_shared(&session, "start_construction", request(&session, 100)).unwrap();
        session.lock().unwrap().record_layout_checkpoint(0).unwrap();
        until(&session, |s| s["status"] == "conflicted");
        assert_eq!(snapshot(&session)["completed_steps"], 0);
    }

    #[test]
    fn stale_resume_conflicts_even_when_the_graph_has_not_changed() {
        let session = shared();
        let receipt =
            execute_shared(&session, "start_construction", request(&session, 100)).unwrap();
        execute_shared(
            &session,
            "pause_construction",
            json!({"plan_id":receipt["plan_id"]}),
        )
        .unwrap();
        assert!(execute_shared(
            &session,
            "resume_construction",
            json!({"plan_id":receipt["plan_id"],"expected_revision":1})
        )
        .unwrap_err()
        .contains("newly reviewed"));
        assert_eq!(snapshot(&session)["status"], "conflicted");
        assert_eq!(snapshot(&session)["current_revision"], 0);
        let next = execute_shared(&session, "start_construction", request(&session, 2000)).unwrap();
        assert!(next["plan_id"].as_u64().unwrap() > receipt["plan_id"].as_u64().unwrap());
    }

    #[test]
    fn concurrent_starts_admit_only_one_plan() {
        let session = shared();
        let args = request(&session, 2000);
        let barrier = Arc::new(Barrier::new(3));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let session = session.clone();
            let args = args.clone();
            let barrier = barrier.clone();
            handles.push(thread::spawn(move || {
                barrier.wait();
                execute_shared(&session, "start_construction", args)
            }));
        }
        barrier.wait();
        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        assert_eq!(snapshot(&session)["plan_id"], 1);
        assert_eq!(snapshot(&session)["current_revision"], 0);
    }

    #[test]
    fn close_stops_future_steps_and_weak_worker_does_not_retain_session() {
        let session = controlled_worker();
        assert_eq!(advance_one(&session)["last_revision"], 1);
        session.lock().unwrap().seal_for_close(1).unwrap();
        assert_eq!(snapshot(&session)["status"], "cancelled");
        assert_worker_exits_without_another_step(&session, 1);
        let session = controlled_worker();
        execute_shared(&session, "pause_construction", json!({"plan_id":1})).unwrap();
        let wake = session
            .lock()
            .unwrap()
            .construction
            .plan
            .as_ref()
            .unwrap()
            .wake
            .clone();
        let weak = Arc::downgrade(&session);
        drop(session);
        let deadline = Instant::now() + Duration::from_secs(1);
        while weak.upgrade().is_some() || Arc::strong_count(&wake) != 1 {
            assert!(
                Instant::now() < deadline,
                "worker retained a dropped session or missed close wakeup"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn clones_and_detached_snapshots_exclude_construction_state() {
        let session = shared();
        let receipt =
            execute_shared(&session, "start_construction", request(&session, 2000)).unwrap();
        execute_shared(
            &session,
            "pause_construction",
            json!({"plan_id":receipt["plan_id"]}),
        )
        .unwrap();
        let live = session.lock().unwrap();
        assert_eq!(live.clone().construction_snapshot()["status"], "idle");
        let detached = live.detached_snapshot();
        assert_eq!(detached.construction_snapshot()["status"], "idle");
        assert!(detached.live_module_events.is_none());
        assert!(detached.undo.is_empty());
    }

    #[test]
    fn bounds_and_non_graph_operations_are_rejected_without_registration() {
        let session = shared();
        let default = request(&session, 100);
        let mut cases = Vec::new();
        for (key, value) in [
            ("expected_revision", json!(-1)),
            ("interval_ms", json!(99)),
            ("interval_ms", json!(2001)),
            ("title", json!("é".repeat(65))),
            ("steps", json!([])),
            ("steps", json!(vec![initial_step(); 65])),
            (
                "steps",
                json!([{"label":"é".repeat(81),"edits":initial_step()["edits"]}]),
            ),
            (
                "steps",
                json!([{"label":"Too many","edits":vec![json!({"tool":"clear_cables","arguments":{}});9]}]),
            ),
            (
                "steps",
                json!(vec![
                    json!({"label":"Too many total","edits":vec![json!({"tool":"clear_cables","arguments":{}});8]});
                    33
                ]),
            ),
        ] {
            let mut args = default.clone();
            args[key] = value;
            cases.push(args);
        }
        let mut missing = default.clone();
        missing.as_object_mut().unwrap().remove("expected_revision");
        cases.push(missing);
        for tool in [
            "apply_edits",
            "start_construction",
            "trigger_module",
            "save_patch",
            "render_audio",
            "load_patch",
        ] {
            let mut args = default.clone();
            args["steps"] = json!([{"label":"Disallowed","edits":[{"tool":tool,"arguments":{}}]}]);
            cases.push(args);
        }
        let mut nested_revision = default.clone();
        nested_revision["steps"][0]["edits"][0]["arguments"]["expected_revision"] = json!(0);
        cases.push(nested_revision);
        for args in cases {
            assert!(
                execute_shared(&session, "start_construction", args.clone()).is_err(),
                "accepted {args}"
            );
            assert_eq!(snapshot(&session)["status"], "idle");
            assert_eq!(session.lock().unwrap().revision, 0);
        }
        assert!(execute_shared(&session, "inspect_construction", json!({"plan_id":1})).is_err());
    }
}
