//! Control-thread operations, shared by transports. Never called by the DSP loop.
mod cable_edits;
mod capture;
mod construction;
mod layout_history;
mod preset_tools;
mod project_capture;
mod running;
pub use project_capture::{ProjectPatchSnapshot, ProjectRunningSnapshot};
use running::EditEffects;
#[cfg(test)]
mod adsr_eg_tests;
#[cfg(test)]
mod cable_color_tests;
#[cfg(test)]
mod cable_edit_tests;
#[cfg(test)]
mod cable_order_tests;
#[cfg(test)]
mod duplicate_tests;
#[cfg(test)]
mod parameter_action_tests;
#[cfg(test)]
mod preset_tests;
#[cfg(test)]
mod push_tests;
#[cfg(test)]
mod score_tests;
#[cfg(test)]
mod sequence_tests;
#[cfg(test)]
mod stack_tests;
use openpatch_core::{
    catalog, render_audio_with_performance_events, validate_midi_events, validate_module_events,
    write_wav_interleaved, Cable, Engine, ModuleAction, ModuleHandle, ModuleParameterActions, Node,
    Patch, PortFrame, SavedModuleState, TimedMidiEvent, TimedModuleEvent, BLOCK_SIZE, MAX_CHANNELS,
    MODULE_ACTION_NAMES,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Instant,
};

/// Control-thread bridge to the bounded live audio event queue. A successful
/// trigger returns a queue receipt; callback application is reported separately.
pub trait LiveModuleEvents: Send + Sync {
    fn trigger(&self, id: &str, action: ModuleAction) -> Result<Value, String>;
    fn snapshot(&self) -> Value;
    /// Read one zero-based voice from a live rack Scope; never render a substitute.
    fn inspect_scope(&self, _id: &str, _channel: usize) -> Result<Value, String> {
        Err("live Scope inspection is unsupported by this backend".into())
    }
    fn sync_patch(&self, _patch: &Patch, _revision: u64) {}
    /// Retire explicit removals, including remove-and-readd in one transaction.
    fn retire_modules(&self, _ids: &[String]) {}
    /// Retire explicit cable removals, including reuse of an ID in one batch.
    fn retire_cables(&self, _ids: &[String]) {}
    /// Initialize final, synchronized module incarnations after a successful edit.
    /// This hook is not replayed by undo/redo.
    fn initialize_modules(&self, _ids: &[String]) {}
    /// Assign saved SEQ3 Run after adoption, including equal-value loads/history.
    fn load_module_running(&self, _states: &BTreeMap<String, bool>) {}
    /// Assign saved PUSH HOLD after adoption, including equal-value loads/history.
    fn load_module_holding(&self, _states: &BTreeMap<String, bool>) {}
    /// Assign typed persistent state after adoption, including equal-value loads.
    fn load_module_saved_states(&self, states: &BTreeMap<String, SavedModuleState>) {
        let mut running = BTreeMap::new();
        let mut holding = BTreeMap::new();
        for (id, state) in states {
            match state {
                SavedModuleState::Seq3Run(value) => {
                    running.insert(id.clone(), *value);
                }
                SavedModuleState::PushHold(value) => {
                    holding.insert(id.clone(), *value);
                }
                SavedModuleState::RandomValues { .. } | SavedModuleState::ShiftRegister { .. } => {}
            }
        }
        if !running.is_empty() {
            self.load_module_running(&running);
        }
        if !holding.is_empty() {
            self.load_module_holding(&holding);
        }
    }
    /// Release transient manual buttons without resetting DSP histories.
    fn release_module_controls(&self, _ids: &[String]) {}
}

pub struct Session {
    patch: Patch,
    revision: u64,
    state_id: u64,
    workspace: PathBuf,
    undo: VecDeque<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    history_group: Option<(String, String, String)>,
    closing: bool,
    live_module_events: Option<Arc<dyn LiveModuleEvents>>,
    construction: construction::Construction,
}

// A cloned control snapshot never adopts another session's plan or worker.
impl Clone for Session {
    fn clone(&self) -> Self {
        Self {
            patch: self.patch.clone(),
            revision: self.revision,
            state_id: self.state_id,
            workspace: self.workspace.clone(),
            undo: self.undo.clone(),
            redo: self.redo.clone(),
            history_group: self.history_group.clone(),
            closing: self.closing,
            live_module_events: self.live_module_events.clone(),
            construction: construction::Construction::default(),
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.construction.wake();
    }
}

#[derive(Clone)]
struct HistoryEntry {
    patch: Patch,
    state_id: u64,
    // Reset metadata belongs to the transition, even if both patches serialize
    // identically. Replay it whenever undo/redo crosses this boundary.
    retired_modules: Vec<String>,
    retired_cables: Vec<String>,
    restore_saved: BTreeMap<String, SavedModuleState>,
    return_saved: BTreeMap<String, SavedModuleState>,
    release_controls: Vec<String>,
}

const HISTORY_LIMIT: usize = 128;
const MAX_EDITS: usize = 256;
const EDIT_TOOLS: &[&str] = &[
    "add_module",
    "duplicate_modules",
    "remove_module",
    "connect",
    "disconnect",
    "disconnect_cables",
    "move_cables",
    "promote_cables",
    "set_cable_color",
    "clear_cables",
    "set_parameter",
    "set_score",
    "set_bypass",
    "invert_mutes",
    "initialize_modules",
    "randomize_modules",
    "apply_module_preset",
    "set_output",
];

/// Shared only by control threads; never acquire this mutex in an audio callback.
pub type SharedSession = Arc<Mutex<Session>>;

/// Compatibility name for the original offline command-line application.
pub type Application = Session;

/// Execute a control request while allowing the UI to remain responsive during
/// offline rendering, benchmarks, or file I/O. Those operations use a coherent
/// snapshot; graph edits remain serialized by the live session mutex.
pub fn execute_shared(session: &SharedSession, name: &str, args: Value) -> Result<Value, String> {
    execute_shared_authorized(session, name, args, None)
}

/// Local transports carry a revocable authorization. Check it while holding
/// the same lock as edits, so bridge Stop can revoke queued requests atomically
/// with pausing construction. Stdio and native controls have no bridge token.
pub(crate) fn execute_shared_authorized(
    session: &SharedSession,
    name: &str,
    args: Value,
    authorization: Option<&std::sync::atomic::AtomicBool>,
) -> Result<Value, String> {
    if name == "start_construction" {
        return construction::start(session, args, authorization);
    }
    #[cfg(test)]
    transport_authorization_tests::pause_if_requested(
        session,
        transport_authorization_tests::Boundary::Dispatch,
    );
    let mut live = session.lock().map_err(|_| "session lock poisoned")?;
    check_transport_authorization(authorization)?;
    match name {
        "export_visualizer"
        | "save_patch"
        | "render_audio"
        | "render_arrangement"
        | "benchmark"
        | "analyze_signal"
        | "capture_signal"
        | "save_module_preset"
        | "read_module_preset"
        | "list_module_presets" => {
            let mut snapshot = live.detached_snapshot();
            drop(live);
            snapshot.execute(name, args)
        }
        _ => live.execute(name, args),
    }
}

fn check_transport_authorization(
    authorization: Option<&std::sync::atomic::AtomicBool>,
) -> Result<(), String> {
    if authorization.is_some_and(|active| !active.load(std::sync::atomic::Ordering::Acquire)) {
        return Err("local MCP bridge stopped; reconnect to a running bridge".into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod transport_authorization_tests;

fn string<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{name} must be a string"))
}

fn optional_string<'a>(args: &'a Value, name: &str, default: &'a str) -> Result<&'a str, String> {
    match args.get(name) {
        Some(_) => string(args, name),
        None => Ok(default),
    }
}

impl Session {
    pub fn new(workspace: &Path) -> Result<Self, String> {
        fs::create_dir_all(workspace).map_err(|e| e.to_string())?;
        Ok(Self {
            patch: Patch::new_v2(),
            revision: 0,
            state_id: 0,
            undo: VecDeque::new(),
            redo: Vec::new(),
            history_group: None,
            closing: false,
            live_module_events: None,
            construction: construction::Construction::default(),
            workspace: workspace.canonicalize().map_err(|e| e.to_string())?,
        })
    }

    pub fn set_live_module_events(&mut self, backend: Option<Arc<dyn LiveModuleEvents>>) {
        if let Some(backend) = &backend {
            backend.sync_patch(&self.patch, self.revision);
        }
        self.live_module_events = backend;
    }

    fn sync_live_module_events(&self) {
        if let Some(backend) = &self.live_module_events {
            backend.sync_patch(&self.patch, self.revision);
        }
    }

    pub fn patch(&self) -> &Patch {
        &self.patch
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Atomically seal an observed revision against subsequent graph edits.
    /// Call only after the final close decision while holding the shared-session
    /// mutex. A stale decision leaves the session open; repeated sealing of the
    /// same revision is harmless. Read-only work and exports remain available.
    pub fn seal_for_close(&mut self, expected_revision: u64) -> Result<(), String> {
        self.check_revision(Some(expected_revision))?;
        self.closing = true;
        self.construction.close();
        Ok(())
    }

    fn check_open(&self) -> Result<(), String> {
        if self.closing {
            Err("session is closing".into())
        } else {
            Ok(())
        }
    }

    /// Stable identity of a retained graph state; undo restores this identity
    /// while the concurrency revision always increases.
    pub fn state_id(&self) -> u64 {
        self.state_id
    }

    pub fn history_state_ids(&self) -> impl Iterator<Item = u64> + '_ {
        std::iter::once(self.state_id)
            .chain(self.undo.iter().map(|entry| entry.state_id))
            .chain(self.redo.iter().map(|entry| entry.state_id))
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub fn into_shared(self) -> SharedSession {
        Arc::new(Mutex::new(self))
    }

    pub fn snapshot(&self) -> Value {
        json!({"revision":self.revision,"state_id":self.state_id,"patch":self.patch,"history":self.history()})
    }

    /// Replace the document through the same validation and revision boundary.
    /// Modules start fresh even when IDs match the previous document.
    /// Failed validation or a stale revision leaves the live patch unchanged.
    pub fn replace_patch(
        &mut self,
        candidate: Patch,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        let retired = self.module_ids();
        let retired_cables = self.cable_ids();
        let mut effects = EditEffects::default();
        effects.retired = retired;
        effects.retired_cables = retired_cables;
        self.commit(candidate, expected_revision, None, &effects)
    }

    /// Begin a new document without resetting revision identities or the live
    /// backend. Validation and retirement must succeed before history is erased.
    pub fn new_document(
        &mut self,
        candidate: Patch,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        let revision = self.replace_patch(candidate, expected_revision)?;
        self.undo.clear();
        self.redo.clear();
        self.history_group = None;
        Ok(revision)
    }

    fn module_ids(&self) -> Vec<String> {
        self.patch
            .nodes
            .iter()
            .map(|node| node.id.clone())
            .collect()
    }

    fn cable_ids(&self) -> Vec<String> {
        self.patch
            .cables
            .iter()
            .filter_map(|cable| cable.id.clone())
            .collect()
    }

    fn commit(
        &mut self,
        mut candidate: Patch,
        expected_revision: Option<u64>,
        group: Option<(String, String, String)>,
        effects: &EditEffects,
    ) -> Result<u64, String> {
        self.check_open()?;
        self.check_revision(expected_revision)?;
        candidate.assign_missing_cable_ids()?;
        candidate.compile()?;
        let revision = self.revision.checked_add(1).ok_or("revision exhausted")?;
        if group.is_some() && group == self.history_group {
            self.patch = candidate;
            if let Some(entry) = self.undo.back_mut() {
                entry.return_saved.extend(effects.after_saved.clone());
            }
        } else {
            if self.undo.len() == HISTORY_LIMIT {
                self.undo.pop_front();
            }
            self.undo.push_back(HistoryEntry {
                patch: std::mem::replace(&mut self.patch, candidate),
                state_id: self.state_id,
                retired_modules: effects.retired.clone(),
                retired_cables: effects.retired_cables.clone(),
                restore_saved: effects.before_saved.clone(),
                return_saved: effects.after_saved.clone(),
                release_controls: effects.release_controls.iter().cloned().collect(),
            });
            self.state_id = revision;
        }
        self.history_group = group;
        self.redo.clear();
        self.revision = revision;
        self.construction.wake();
        if !effects.retired.is_empty() {
            if let Some(backend) = &self.live_module_events {
                backend.retire_modules(&effects.retired);
            }
        }
        if !effects.retired_cables.is_empty() {
            if let Some(backend) = &self.live_module_events {
                backend.retire_cables(&effects.retired_cables);
            }
        }
        self.sync_live_module_events();
        if let Some(backend) = &self.live_module_events {
            if !effects.initialized.is_empty() {
                backend
                    .initialize_modules(&effects.initialized.iter().cloned().collect::<Vec<_>>());
            }
            if !effects.release_controls.is_empty() {
                backend.release_module_controls(
                    &effects.release_controls.iter().cloned().collect::<Vec<_>>(),
                );
            }
            if !effects.load_saved.is_empty() {
                running::load_saved(backend.as_ref(), &effects.load_saved);
            }
        }
        Ok(revision)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn history(&self) -> Value {
        json!({"undo_steps":self.undo.len(),"redo_steps":self.redo.len(),"limit":HISTORY_LIMIT})
    }

    fn restore_history(&mut self, redo: bool) -> Result<Value, String> {
        self.check_open()?;
        let revision = self.revision.checked_add(1).ok_or("revision exhausted")?;
        let entry = if redo {
            self.redo.pop().ok_or("nothing to redo")?
        } else {
            self.undo.pop_back().ok_or("nothing to undo")?
        };
        let opposite = HistoryEntry {
            patch: std::mem::replace(&mut self.patch, entry.patch),
            state_id: std::mem::replace(&mut self.state_id, entry.state_id),
            retired_modules: entry.retired_modules.clone(),
            retired_cables: entry.retired_cables.clone(),
            restore_saved: entry.return_saved.clone(),
            return_saved: entry.restore_saved.clone(),
            release_controls: entry.release_controls.clone(),
        };
        if redo {
            self.undo.push_back(opposite);
        } else {
            self.redo.push(opposite);
        }
        self.revision = revision;
        self.history_group = None;
        self.construction.wake();
        if let Some(backend) = &self.live_module_events {
            if !entry.retired_modules.is_empty() {
                backend.retire_modules(&entry.retired_modules);
            }
            if !entry.retired_cables.is_empty() {
                backend.retire_cables(&entry.retired_cables);
            }
        }
        self.sync_live_module_events();
        if let Some(backend) = &self.live_module_events {
            let releases: Vec<_> = entry
                .release_controls
                .into_iter()
                .filter(|id| self.patch.nodes.iter().any(|node| node.id == *id))
                .collect();
            if !releases.is_empty() {
                backend.release_module_controls(&releases);
            }
            if !entry.restore_saved.is_empty() {
                running::load_saved(backend.as_ref(), &entry.restore_saved);
            }
        }
        Ok(self.snapshot())
    }

    fn check_revision(&self, expected: Option<u64>) -> Result<(), String> {
        if let Some(expected) = expected {
            if expected != self.revision {
                return Err(format!(
                    "revision conflict: expected {expected}, current {}",
                    self.revision
                ));
            }
        }
        Ok(())
    }

    fn file(&self, filename: &str, extension: &str) -> Result<PathBuf, String> {
        if filename.is_empty()
            || filename.len() > 120
            || !filename
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            || filename.starts_with('.')
            || !filename.ends_with(extension)
        {
            return Err(format!(
                "filename must be a simple ASCII name ending with {extension}; no directories"
            ));
        }
        let path = self.workspace.join(filename);
        // Reject symlinks for reads as well as writes. Writes additionally use create_new.
        if let Ok(meta) = fs::symlink_metadata(&path) {
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err("file must be a regular file, not a symlink".into());
            }
        }
        Ok(path)
    }

    fn analyze_signal(&self, args: &Value) -> Result<Value, String> {
        self.check_revision(expected_revision(args)?)?;
        let seconds = args
            .get("seconds")
            .and_then(Value::as_f64)
            .ok_or("seconds must be a number")?;
        if !seconds.is_finite() || !(0.001..=2.0).contains(&seconds) {
            return Err("analysis seconds must be 0.001..2".into());
        }
        let mut patch = self.patch.clone();
        patch.output = Some(string(args, "id")?.into());
        patch.output_port = optional_string(args, "port", "out")?.into();
        let mut engine = patch.compile()?;
        let frames = (seconds * patch.sample_rate as f64).round() as usize;
        let mut events = PerformanceEvents::new(args, frames, &engine)?;
        let mut block = [PortFrame::default(); BLOCK_SIZE];
        let mut counts = [0_usize; MAX_CHANNELS];
        let mut sums = [0.0_f64; MAX_CHANNELS];
        let mut squares = [0.0_f64; MAX_CHANNELS];
        let mut minimum = [f32::INFINITY; MAX_CHANNELS];
        let mut maximum = [f32::NEG_INFINITY; MAX_CHANNELS];
        let mut peak = [0.0_f32; MAX_CHANNELS];
        let mut remaining = frames;
        let mut max_channels = 0;
        let mut min_channels = MAX_CHANNELS;
        while remaining > 0 {
            let cursor = frames - remaining;
            events.apply(&mut engine, cursor);
            let until_event = events.next_frame(frames) - cursor;
            let count = remaining.min(BLOCK_SIZE).min(until_event);
            engine.process_poly_block(&mut block[..count]);
            for frame in &block[..count] {
                let channels = frame.channels as usize;
                max_channels = max_channels.max(channels);
                min_channels = min_channels.min(channels);
                for channel in 0..channels {
                    let value = frame.values[channel];
                    if !value.is_finite() {
                        return Err("analysis encountered a nonfinite signal".into());
                    }
                    counts[channel] += 1;
                    sums[channel] += value as f64;
                    squares[channel] += (value as f64).powi(2);
                    minimum[channel] = minimum[channel].min(value);
                    maximum[channel] = maximum[channel].max(value);
                    peak[channel] = peak[channel].max(value.abs());
                }
            }
            remaining -= count;
        }
        let channels: Vec<_> = (0..max_channels)
            .map(|channel| {
                let count = counts[channel];
                json!({"channel":channel + 1,"active_frames":count,
                "mean":sums[channel] / count as f64,
                "rms":(squares[channel] / count as f64).sqrt(),
                "peak":peak[channel],"min":minimum[channel],"max":maximum[channel]})
            })
            .collect();
        Ok(
            json!({"revision":self.revision,"id":patch.output,"port":patch.output_port,
            "sample_rate":patch.sample_rate,"frames":frames,"min_channels":min_channels,
            "max_channels":max_channels,"channels":channels,"solver_failures":engine.solver_failures(),
            "midi_event_count":events.midi.len(),"module_event_count":events.modules.len(),
            "state":"analyzed offline from reset","statistics":"active frames only; inactive channels excluded",
            "units":"raw module output units; consult catalog"}),
        )
    }

    pub fn execute(&mut self, name: &str, args: Value) -> Result<Value, String> {
        if EDIT_TOOLS.contains(&name)
            || matches!(
                name,
                "replace_patch"
                    | "load_patch"
                    | "apply_edits"
                    | "undo"
                    | "redo"
                    | "rotate_sequence"
                    | "trigger_module"
            )
        {
            self.check_open()?;
        }
        let schemas = tools();
        validate_arguments(&schemas, name, &args)?;
        match name {
            "start_construction" => {
                return Err(
                    "start_construction requires a shared session; use execute_shared".into(),
                )
            }
            "inspect_construction"
            | "pause_construction"
            | "resume_construction"
            | "cancel_construction" => return self.construction_command(name, &args),
            "catalog" => return Ok(catalog()),
            "capture_project_patch" => {
                let captured = self.capture_project_patch(expected_revision(&args)?)?;
                return Ok(json!({"patch":captured.patch,"revision":captured.revision,
                    "state_id":captured.state_id,"running":captured.running,"holding":captured.holding,
                    "random_values":captured.saved_states.iter().filter_map(|(id, state)| match state {
                        SavedModuleState::RandomValues { values, rng } => Some((id.clone(), json!({"values":values.map(|row| row.map(f32::from_bits)),"rng_state":rng}))),
                        _ => None,
                    }).collect::<BTreeMap<_, _>>(),
                    "shift_registers":captured.saved_states.iter().filter_map(|(id, state)| match state {
                        SavedModuleState::ShiftRegister { values, rng } => Some((id.clone(), json!({"values":values.map(f32::from_bits),"rng_state":rng}))),
                        _ => None,
                    }).collect::<BTreeMap<_, _>>(),
                    "state_source":captured.state_source,"frame":captured.frame}));
            }
            "capture_module_preset" => return self.capture_module_preset(&args),
            "save_module_preset" | "read_module_preset" | "list_module_presets" => {
                return self.preset_file_tool(name, &args)
            }
            "inspect_patch" => return Ok(self.snapshot()),
            "inspect_history" => {
                return Ok(json!({"revision":self.revision,"history":self.history()}))
            }
            "trigger_module" => {
                self.check_revision(expected_revision(&args)?)?;
                let id = string(&args, "id")?;
                let action: ModuleAction = serde_json::from_value(args["action"].clone())
                    .map_err(|error| format!("invalid action: {error}"))?;
                let node = self
                    .patch
                    .nodes
                    .iter()
                    .find(|node| node.id == id)
                    .ok_or("module not found")?;
                let supported = match node.kind.as_str() {
                    "seq3" => matches!(
                        action,
                        ModuleAction::Clock | ModuleAction::Run | ModuleAction::Reset
                    ),
                    "logic" => matches!(action, ModuleAction::BPress | ModuleAction::BRelease),
                    "process" => {
                        matches!(action, ModuleAction::GatePress | ModuleAction::GateRelease)
                    }
                    "pulses" => action.pulse_button().is_some(),
                    "gates" => matches!(
                        action,
                        ModuleAction::ResetPress | ModuleAction::ResetRelease
                    ),
                    "random_values" | "adsr_eg" => {
                        matches!(action, ModuleAction::PushPress | ModuleAction::PushRelease)
                    }
                    "sh_asr" => matches!(
                        action,
                        ModuleAction::PushPress
                            | ModuleAction::PushRelease
                            | ModuleAction::ResetPress
                            | ModuleAction::ResetRelease
                    ),
                    "push" => matches!(
                        action,
                        ModuleAction::PushPress
                            | ModuleAction::PushRelease
                            | ModuleAction::HoldPress
                            | ModuleAction::HoldRelease
                    ),
                    _ => false,
                };
                if !supported {
                    return Err("module action requires a seq3 module for clock/run/reset, logic b_press/b_release, process gate_press/gate_release, or pulses pulse1_press/pulse1_release through pulse10_press/pulse10_release, or push push_press/push_release/hold_press/hold_release, or gates reset_press/reset_release, or random_values/adsr_eg push_press/push_release, or sh_asr push_press/push_release/reset_press/reset_release".into());
                }
                if node.bypassed
                    && !matches!(
                        action,
                        ModuleAction::BRelease
                            | ModuleAction::GateRelease
                            | ModuleAction::PushRelease
                            | ModuleAction::HoldRelease
                            | ModuleAction::ResetRelease
                    )
                    && !matches!(action.pulse_button(), Some((_, false)))
                {
                    return Err("module actions are unavailable while module is bypassed".into());
                }
                let backend = self
                    .live_module_events
                    .as_ref()
                    .ok_or("live module events unsupported: no live audio backend attached")?;
                let receipt = backend.trigger(id, action)?;
                return Ok(json!({"revision":self.revision,"receipt":receipt}));
            }
            "inspect_performance" => {
                self.check_revision(expected_revision(&args)?)?;
                return Ok(
                    json!({"revision":self.revision,"supported":self.live_module_events.is_some(),
                    "performance":self.live_module_events.as_ref().map(|backend| backend.snapshot())}),
                );
            }
            "inspect_scope" => {
                self.check_revision(expected_revision(&args)?)?;
                let id = string(&args, "id")?;
                let node = self
                    .patch
                    .nodes
                    .iter()
                    .find(|node| node.id == id)
                    .ok_or("module not found")?;
                if node.kind != "scope" {
                    return Err("module is not a rack Scope".into());
                }
                let channel = match args.get("channel") {
                    None => 1,
                    Some(value) => value
                        .as_u64()
                        .filter(|v| (1..=16).contains(v))
                        .ok_or("channel must be an integer from 1 to 16")?,
                };
                let capture = self
                    .live_module_events
                    .as_ref()
                    .map(|backend| backend.inspect_scope(id, channel as usize - 1))
                    .transpose()?;
                return Ok(
                    json!({"revision":self.revision,"supported":self.live_module_events.is_some(),"scope":capture}),
                );
            }
            "analyze_signal" => return self.analyze_signal(&args),
            "capture_signal" => return self.capture_signal(&args),
            "export_visualizer" => {
                let path = self.file(string(&args, "filename")?, ".html")?;
                crate::visualizer::export(&path).map_err(|e| e.to_string())?;
                return Ok(
                    json!({"filename":args["filename"],"shader_profile":"webgl2-glsl-es-300-v1","instructions":"Open the HTML in a browser, select a local audio file, then press Play. This is a standalone visual preview, not a live native audio session."}),
                );
            }
            "save_patch" => {
                let path = self.file(string(&args, "filename")?, ".json")?;
                let captured = self.capture_project_patch(None)?;
                let bytes =
                    serde_json::to_vec_pretty(&captured.patch).map_err(|e| e.to_string())?;
                let mut f = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                f.write_all(&bytes).map_err(|e| e.to_string())?;
                return Ok(json!({"filename":args["filename"],"revision":self.revision}));
            }
            "render_arrangement" => {
                let seconds = args
                    .get("seconds")
                    .and_then(Value::as_f64)
                    .ok_or("seconds must be a number")?;
                let path = self.file(string(&args, "filename")?, ".wav")?;
                let stats = openpatch_core::render_wav_stream(&self.patch, &path, seconds)?;
                return Ok(
                    json!({"filename":args["filename"],"revision":self.revision,"state":"rendered from reset","stats":stats}),
                );
            }
            "render_audio" => {
                let seconds = args
                    .get("seconds")
                    .and_then(Value::as_f64)
                    .ok_or("seconds must be a number")?;
                if !seconds.is_finite() || !(0.001..=30.0).contains(&seconds) {
                    return Err("seconds must be 0.001..30".into());
                }
                let path = self.file(string(&args, "filename")?, ".wav")?;
                if path.exists() {
                    return Err("destination already exists".into());
                }
                let frames = (seconds * self.patch.sample_rate as f64).round() as usize;
                let events = midi_events(&args, frames)?;
                let module_events = module_events(&args, frames)?;
                let rendered = render_audio_with_performance_events(
                    &self.patch,
                    seconds,
                    &events,
                    &module_events,
                )?;
                let samples = &rendered.samples;
                let peak = samples.iter().fold(0.0_f32, |p, s| p.max(s.abs()));
                let rms = (samples.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
                    / samples.len().max(1) as f64)
                    .sqrt();
                write_wav_interleaved(&path, samples, rendered.sample_rate, rendered.channels)
                    .map_err(|e| e.to_string())?;
                return Ok(
                    json!({"filename":args["filename"],"frames":frames,"channels":rendered.channels,
                    "sample_rate":rendered.sample_rate,"peak":peak,"rms":rms,"revision":self.revision,
                    "state":"rendered from reset","midi_event_count":events.len(),"module_event_count":module_events.len(),
                    "channel_policy":if rendered.channels == 2 { "explicit stereo L/R from audio_stereo" } else { "first channel only; use poly_sum to mix all voices" }}),
                );
            }
            "benchmark" => {
                let seconds = match args.get("seconds") {
                    None => 1.0,
                    Some(v) => v.as_f64().ok_or("seconds must be a number")?,
                };
                if !seconds.is_finite() || !(0.01..=5.0).contains(&seconds) {
                    return Err("benchmark seconds must be 0.01..5".into());
                }
                let mut engine = self.patch.compile()?;
                let mut block = [0.0; 128];
                for _ in 0..16 {
                    engine.process_block(&mut block);
                }
                let frames = (seconds * self.patch.sample_rate as f64).round() as usize;
                let start = Instant::now();
                let mut remaining = frames;
                let mut checksum = 0.0_f64;
                while remaining > 0 {
                    let n = remaining.min(block.len());
                    engine.process_block(&mut block[..n]);
                    checksum += std::hint::black_box(&block[..n])
                        .iter()
                        .map(|s| *s as f64)
                        .sum::<f64>();
                    remaining -= n;
                }
                let elapsed = start.elapsed().as_secs_f64().max(1e-9);
                return Ok(
                    json!({"frames":frames,"elapsed_seconds":elapsed,"frames_per_second":frames as f64/elapsed,"realtime_factor":frames as f64/self.patch.sample_rate as f64/elapsed,"checksum":checksum,"sample_rate":self.patch.sample_rate,"block_size":128,"nodes":self.patch.nodes.len(),"arch":std::env::consts::ARCH,"os":std::env::consts::OS,"debug_build":cfg!(debug_assertions),"comparison":"OpenPatch only; not a VCV Rack comparison"}),
                );
            }
            _ => {}
        }
        let expected_revision = args
            .get("expected_revision")
            .map(|value| {
                value
                    .as_u64()
                    .ok_or("expected_revision must be a nonnegative integer")
            })
            .transpose()?;
        self.check_revision(expected_revision)?;
        // A top-level empty clear is a validated no-op. Preserve redo and an
        // active history group; transactions retain their ordinary semantics.
        if name == "clear_cables" && self.patch.cables.is_empty() {
            return Ok(self.snapshot());
        }
        if name == "randomize_modules"
            && parameter_action_ids(&self.patch, &args)?.iter().all(|id| {
                self.patch
                    .nodes
                    .iter()
                    .any(|node| node.id == *id && node.kind == "sh_asr")
            })
        {
            return Ok(self.snapshot());
        }
        let mut candidate = self.patch.clone();
        let mut effects = EditEffects::default();
        if matches!(name, "replace_patch" | "load_patch") {
            effects.retired = self.module_ids();
            effects.retired_cables = self.cable_ids();
        }
        match name {
            "rotate_sequence" => {
                self.apply_session_edit(&mut candidate, name, &args, &mut effects)?
            }
            "replace_patch" => {
                candidate =
                    serde_json::from_value(args.get("patch").ok_or("patch required")?.clone())
                        .map_err(|e| e.to_string())?;
            }
            "load_patch" => {
                let path = self.file(string(&args, "filename")?, ".json")?;
                let f = fs::File::open(path).map_err(|e| e.to_string())?;
                let mut data = Vec::new();
                f.take(1_048_577)
                    .read_to_end(&mut data)
                    .map_err(|e| e.to_string())?;
                if data.len() > 1_048_576 {
                    return Err("patch file exceeds 1 MiB".into());
                }
                candidate = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
            }
            "apply_edits" | "preview_edits" => {
                let edits = args
                    .get("edits")
                    .and_then(Value::as_array)
                    .ok_or("edits must be an array")?;
                if edits.is_empty() || edits.len() > MAX_EDITS {
                    return Err(format!("edits must contain 1..{MAX_EDITS} operations"));
                }
                for (index, edit) in edits.iter().enumerate() {
                    let result = (|| {
                        let object = edit.as_object().ok_or("edit must be an object")?;
                        if object.keys().any(|key| key != "tool" && key != "arguments") {
                            return Err("edit accepts only tool and arguments".into());
                        }
                        let tool = string(edit, "tool")?;
                        if !EDIT_TOOLS.contains(&tool) {
                            return Err(format!("tool is not a graph edit: {tool}"));
                        }
                        let arguments = edit.get("arguments").ok_or("arguments required")?;
                        validate_arguments(&schemas, tool, arguments)?;
                        if arguments.get("expected_revision").is_some()
                            || arguments.get("history_group").is_some()
                        {
                            return Err(
                                "revision and history grouping belong to the transaction, not an edit".into(),
                            );
                        }
                        self.apply_session_edit(&mut candidate, tool, arguments, &mut effects)?;
                        Ok(())
                    })();
                    result.map_err(|error: String| format!("edit {}: {error}", index + 1))?;
                }
                if name == "preview_edits" {
                    candidate.assign_missing_cable_ids()?;
                    candidate.compile()?;
                    return Ok(
                        json!({"revision":self.revision,"changes":patch_changes(&self.patch, &candidate),"patch":candidate,"committed":false,"edit_count":edits.len()}),
                    );
                }
            }
            "undo" => return self.restore_history(false),
            "redo" => return self.restore_history(true),
            name if EDIT_TOOLS.contains(&name) => {
                self.apply_session_edit(&mut candidate, name, &args, &mut effects)?
            }
            _ => return Err(format!("unknown tool: {name}")),
        }
        // Transaction boundary: failed edits never change the patch or revision.
        // Direct equal-color/same-endpoint edits are validated no-ops. Batched
        // no-ops intentionally retain the ordinary transaction history contract.
        if matches!(name, "set_cable_color" | "move_cables" | "promote_cables")
            && serde_json::to_value(&candidate).map_err(|error| error.to_string())?
                == serde_json::to_value(&self.patch).map_err(|error| error.to_string())?
        {
            candidate.compile()?;
            return Ok(self.snapshot());
        }
        let group = if name == "set_parameter" && args.get("history_group").is_some() {
            let group = string(&args, "history_group")?;
            if group.is_empty() || group.len() > 128 {
                return Err("history_group must contain 1..128 bytes".into());
            }
            Some((
                group.into(),
                string(&args, "id")?.into(),
                string(&args, "parameter")?.into(),
            ))
        } else {
            None
        };
        self.commit(candidate, expected_revision, group, &effects)?;
        Ok(self.snapshot())
    }
}

fn expected_revision(args: &Value) -> Result<Option<u64>, String> {
    args.get("expected_revision")
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| "expected_revision must be a nonnegative integer".to_owned())
        })
        .transpose()
}

fn sequence_node<'a>(patch: &'a Patch, id: &str) -> Result<&'a Node, String> {
    let node = patch
        .nodes
        .iter()
        .find(|node| node.id == id)
        .ok_or("module not found")?;
    if node.kind != "seq3" {
        return Err("module action requires a seq3 module".into());
    }
    Ok(node)
}

fn rotate_sequence(patch: &mut Patch, args: &Value) -> Result<(), String> {
    let id = string(args, "id")?;
    sequence_node(patch, id)?;
    let offset = match string(args, "direction")? {
        "left" => 1,
        "right" => 7,
        _ => return Err("direction must be left or right".into()),
    };
    let node = patch.nodes.iter_mut().find(|node| node.id == id).unwrap();
    for row in 0..4 {
        let keys: Vec<_> = (1..=8)
            .map(|step| {
                if row == 0 {
                    format!("gate{step}")
                } else {
                    format!("row{row}_{step}")
                }
            })
            .collect();
        let default = if row == 0 { 1.0 } else { 0.0 };
        let values: Vec<_> = keys
            .iter()
            .map(|key| node.params.get(key).copied().unwrap_or(default))
            .collect();
        for (step, key) in keys.into_iter().enumerate() {
            node.params.insert(key, values[(step + offset) % 8]);
        }
    }
    Ok(())
}

fn module_events(args: &Value, frames: usize) -> Result<Vec<TimedModuleEvent>, String> {
    let events = match args.get("module_events") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid module_events: {error}"))?,
        None => Vec::new(),
    };
    validate_module_events(&events, frames)?;
    Ok(events)
}

/// Apply both traces before their sample, preserving each array's equal-frame
/// order and applying MIDI before module actions when the two traces tie.
struct PerformanceEvents {
    midi: Vec<TimedMidiEvent>,
    modules: Vec<TimedModuleEvent>,
    handles: Vec<ModuleHandle>,
    midi_index: usize,
    module_index: usize,
}
impl PerformanceEvents {
    fn new(args: &Value, frames: usize, engine: &Engine) -> Result<Self, String> {
        let midi = midi_events(args, frames)?;
        let modules = module_events(args, frames)?;
        let handles = modules
            .iter()
            .map(|event| engine.resolve_module_action(&event.id, event.action))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            midi,
            modules,
            handles,
            midi_index: 0,
            module_index: 0,
        })
    }
    fn apply(&mut self, engine: &mut Engine, frame: usize) {
        while self
            .midi
            .get(self.midi_index)
            .is_some_and(|event| event.frame == frame as u64)
        {
            engine.handle_midi(self.midi[self.midi_index].event);
            self.midi_index += 1;
        }
        while self
            .modules
            .get(self.module_index)
            .is_some_and(|event| event.frame == frame as u64)
        {
            let applied = engine.handle_module_event(
                self.handles[self.module_index],
                self.modules[self.module_index].action,
            );
            debug_assert!(applied, "validated offline module event was rejected");
            self.module_index += 1;
        }
    }
    fn next_frame(&self, frames: usize) -> usize {
        self.midi
            .get(self.midi_index)
            .map_or(frames, |event| event.frame as usize)
            .min(
                self.modules
                    .get(self.module_index)
                    .map_or(frames, |event| event.frame as usize),
            )
    }
}

fn module_events_schema() -> Value {
    json!({"type":"array","maxItems":65536,
        "description":"Offline seq3, logic, process, pulses, push, and gates actions sorted by frame; equal-frame array order is preserved. MIDI runs first at ties. At most 64 actions per module per frame. Each frame must be inside the render.",
        "items":{"type":"object","properties":{"frame":{"type":"integer","minimum":0},"id":{"type":"string"},"action":{"type":"string","enum":MODULE_ACTION_NAMES}},"required":["frame","id","action"],"additionalProperties":false}})
}

fn midi_events(args: &Value, frames: usize) -> Result<Vec<TimedMidiEvent>, String> {
    let events = match args.get("midi_events") {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid midi_events: {error}"))?,
        None => Vec::new(),
    };
    validate_midi_events(&events, frames)?;
    Ok(events)
}

fn midi_events_schema() -> Value {
    let mut variants = Vec::new();
    for (name, fields) in [
        (
            "note_on",
            vec![("channel", 15), ("note", 127), ("velocity", 127)],
        ),
        (
            "note_off",
            vec![("channel", 15), ("note", 127), ("velocity", 127)],
        ),
        (
            "control_change",
            vec![("channel", 15), ("controller", 127), ("value", 127)],
        ),
        ("pitch_bend", vec![("channel", 15), ("value", 16383)]),
        ("channel_pressure", vec![("channel", 15), ("pressure", 127)]),
        (
            "poly_pressure",
            vec![("channel", 15), ("note", 127), ("pressure", 127)],
        ),
        ("clock", vec![]),
        ("start", vec![]),
        ("continue", vec![]),
        ("stop", vec![]),
        ("panic", vec![]),
    ] {
        let mut properties = serde_json::Map::new();
        properties.insert("type".into(), json!({"const":name}));
        let mut required = vec!["type"];
        for (field, maximum) in fields {
            properties.insert(
                field.into(),
                json!({"type":"integer","minimum":0,"maximum":maximum}),
            );
            required.push(field);
        }
        variants.push(json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}));
    }
    json!({"type":"array","maxItems":65536,
        "description":"Sample-frame MIDI trace, sorted by frame; equal-frame events preserve array order. Channels are zero-based. Offline only; no live playback or history edits.",
        "items":{"type":"object","properties":{"frame":{"type":"integer","minimum":0},"event":{"oneOf":variants}},"required":["frame","event"],"additionalProperties":false}})
}

fn patch_changes(before: &Patch, after: &Patch) -> Value {
    let old: BTreeMap<_, _> = before.nodes.iter().map(|node| (&node.id, node)).collect();
    let new: BTreeMap<_, _> = after.nodes.iter().map(|node| (&node.id, node)).collect();
    let added: Vec<_> = new
        .iter()
        .filter(|(id, _)| !old.contains_key(*id))
        .map(|(_, node)| node)
        .collect();
    let removed: Vec<_> = old
        .iter()
        .filter(|(id, _)| !new.contains_key(*id))
        .map(|(_, node)| node)
        .collect();
    let changed: Vec<_> = new
        .iter()
        .filter_map(|(id, node)| {
            let previous = old.get(id)?;
            (previous.kind != node.kind
                || previous.params != node.params
                || previous.score != node.score
                || previous.bypassed != node.bypassed)
                .then(|| json!({"id":id,"before":previous,"after":node}))
        })
        .collect();
    let signature = |cable: &Cable| {
        (
            cable.id.clone(),
            cable.from.clone(),
            cable.output.clone(),
            cable.to.clone(),
            cable.input.clone(),
        )
    };
    let difference = |left: &'_ [Cable], right: &[Cable]| {
        let mut counts = BTreeMap::new();
        for cable in right {
            *counts.entry(signature(cable)).or_insert(0_usize) += 1;
        }
        let mut changed = Vec::new();
        for cable in left {
            let count = counts.entry(signature(cable)).or_default();
            if *count > 0 {
                *count -= 1;
            } else {
                changed.push(cable.clone());
            }
        }
        changed
    };
    let added_cables = difference(&after.cables, &before.cables);
    let removed_cables = difference(&before.cables, &after.cables);
    let changed_cables: Vec<_> = after
        .cables
        .iter()
        .filter_map(|cable| {
            let id = cable.id.as_ref()?;
            let previous = before
                .cables
                .iter()
                .find(|old| old.id.as_ref() == Some(id))?;
            (signature(previous) == signature(cable) && previous.color != cable.color)
                .then(|| json!({"id":id,"before":previous,"after":cable}))
        })
        .collect();
    let changed_cable_order: Vec<_> = after.cables.iter().filter_map(|cable| {
        let id = cable.id.as_ref()?;
        let previous = before.cables.iter().find(|old| old.id.as_ref() == Some(id))?;
        (previous.input_order != cable.input_order || previous.output_order != cable.output_order)
            .then(|| json!({"id":id,"before":{"input_order":previous.input_order,"output_order":previous.output_order},"after":{"input_order":cable.input_order,"output_order":cable.output_order}}))
    }).collect();
    // Add/remove/endpoint changes already appear above. This flag specifically
    // identifies a pure routing-order change of the same cable multiset.
    let cable_order_changed = added_cables.is_empty()
        && removed_cables.is_empty()
        && !before
            .cables
            .iter()
            .map(&signature)
            .eq(after.cables.iter().map(&signature));
    json!({"added_modules":added,"removed_modules":removed,"changed_modules":changed,
        "added_cables":added_cables,"removed_cables":removed_cables,"changed_cables":changed_cables,"changed_cable_order":changed_cable_order,"cable_order_changed":cable_order_changed,
        "output":{"before":{"node":before.output,"port":before.output_port},"after":{"node":after.output,"port":after.output_port}}})
}

fn validate_arguments(schemas: &Value, name: &str, args: &Value) -> Result<(), String> {
    if !args.is_object() {
        return Err("arguments must be an object".into());
    }
    let schema = schemas
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == name)
        .ok_or_else(|| format!("unknown tool: {name}"))?;
    let allowed = schema["inputSchema"]["properties"].as_object().unwrap();
    for key in args.as_object().unwrap().keys() {
        if !allowed.contains_key(key) {
            return Err(format!("unknown argument: {key}"));
        }
    }
    for field in schema["inputSchema"]["required"].as_array().unwrap() {
        let field = field.as_str().unwrap();
        if args.get(field).is_none() {
            return Err(format!("missing argument: {field}"));
        }
    }
    Ok(())
}

fn apply_edit(
    candidate: &mut Patch,
    name: &str,
    args: &Value,
    retired_cables: &mut Vec<String>,
    initialized: &mut BTreeSet<String>,
) -> Result<(), String> {
    match name {
        "add_module" => {
            let params: BTreeMap<String, f32> =
                serde_json::from_value(args.get("params").cloned().unwrap_or(json!({})))
                    .map_err(|e| e.to_string())?;
            candidate.nodes.push(Node {
                score: args
                    .get("score")
                    .map(|value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()))
                    .transpose()?,
                id: string(args, "id")?.into(),
                kind: string(args, "kind")?.into(),
                params,
                bypassed: false,
            });
        }
        "set_score" => {
            let id = string(args, "id")?;
            let score =
                serde_json::from_value(args.get("score").cloned().ok_or("score is required")?)
                    .map_err(|e| e.to_string())?;
            let node = candidate
                .nodes
                .iter_mut()
                .find(|n| n.id == id)
                .ok_or("module not found")?;
            if node.kind != "score_sequencer" {
                return Err("set_score requires score_sequencer".into());
            }
            node.score = Some(score);
        }
        "duplicate_modules" => duplicate_modules(candidate, args)?,
        "set_cable_color" => cable_edits::set_color(candidate, args)?,
        "disconnect_cables" => cable_edits::disconnect(candidate, args, retired_cables)?,
        "move_cables" => cable_edits::move_endpoints(candidate, args, retired_cables)?,
        "promote_cables" => cable_edits::promote(candidate, args)?,
        "remove_module" => {
            let id = string(args, "id")?;
            if !candidate.nodes.iter().any(|n| n.id == id) {
                return Err("module not found".into());
            }
            initialized.remove(id);
            candidate.nodes.retain(|n| n.id != id);
            candidate.cables.retain(|c| {
                let retained = c.from != id && c.to != id;
                if !retained {
                    retired_cables.extend(c.id.clone());
                }
                retained
            });
            if candidate.output.as_deref() == Some(id) {
                candidate.output = None;
                candidate.output_port = "out".into();
            }
        }
        "connect" => {
            // Parse strict RGB bytes before changing the candidate or assigning
            // IDs. A present null is not an RGB color; omission uses the palette.
            let color = args.get("color").map(cable_edits::rgb).transpose()?;
            let mode = optional_string(args, "mode", "reject")?;
            if !matches!(mode, "reject" | "stack") {
                return Err("mode must be reject or stack".into());
            }
            if mode == "stack" && candidate.version != 2 {
                return Err("stack mode requires patch version 2".into());
            }
            let to = string(args, "to")?;
            let input = string(args, "input")?;
            if mode == "reject"
                && candidate
                    .cables
                    .iter()
                    .any(|cable| cable.to == to && cable.input == input)
            {
                return Err("input is already connected; use mode stack to add a cable".into());
            }
            let id = args
                .get("cable_id")
                .map(|_| string(args, "cable_id").map(str::to_owned))
                .transpose()?;
            if id.as_ref().is_some_and(|id| {
                candidate
                    .cables
                    .iter()
                    .any(|cable| cable.id.as_ref() == Some(id))
            }) {
                return Err("cable_id already exists".into());
            }
            candidate.cables.push(Cable {
                input_order: None,
                output_order: None,
                color,
                id,
                from: string(args, "from")?.into(),
                output: optional_string(args, "output", "out")?.into(),
                to: to.into(),
                input: input.into(),
            });
            let index = candidate.cables.len() - 1;
            openpatch_core::assign_new_cable_orders(&mut candidate.cables, &[index])?;
            candidate.assign_missing_cable_ids()?;
        }
        "clear_cables" => {
            retired_cables.extend(candidate.cables.iter().filter_map(|cable| cable.id.clone()));
            candidate.cables.clear();
        }
        "disconnect" => {
            let specific = args.get("cable_id").is_some();
            let destination = args.get("to").is_some() || args.get("input").is_some();
            if specific == destination {
                return Err("disconnect requires either cable_id or both to and input".into());
            }
            let (id, to, input) = if specific {
                (Some(string(args, "cable_id")?), None, None)
            } else {
                (
                    None,
                    Some(string(args, "to")?),
                    Some(string(args, "input")?),
                )
            };
            let before = candidate.cables.len();
            candidate.cables.retain(|cable| {
                let remove = id.map_or_else(
                    || cable.to == to.unwrap() && cable.input == input.unwrap(),
                    |id| cable.id.as_deref() == Some(id),
                );
                if remove {
                    retired_cables.extend(cable.id.clone());
                }
                !remove
            });
            if before == candidate.cables.len() {
                return Err("cable not found".into());
            }
        }
        "set_parameter" => {
            let id = string(args, "id")?;
            let node = candidate
                .nodes
                .iter_mut()
                .find(|n| n.id == id)
                .ok_or("module not found")?;
            let value = args
                .get("value")
                .and_then(Value::as_f64)
                .ok_or("value must be a number")? as f32;
            node.params.insert(string(args, "parameter")?.into(), value);
        }
        "set_bypass" => {
            let id = string(args, "id")?;
            let bypassed = args
                .get("bypassed")
                .and_then(Value::as_bool)
                .ok_or("bypassed must be a boolean")?;
            candidate
                .nodes
                .iter_mut()
                .find(|node| node.id == id)
                .ok_or("module not found")?
                .bypassed = bypassed;
        }
        "invert_mutes" => {
            let id = string(args, "id")?;
            let node = candidate
                .nodes
                .iter_mut()
                .find(|node| node.id == id)
                .ok_or("module not found")?;
            if node.kind != "mutes10" {
                return Err("invert_mutes requires a mutes10 module".into());
            }
            for channel in 1..=10 {
                let key = format!("mute{channel}");
                let value = node.params.get(&key).copied().unwrap_or(0.0);
                node.params.insert(key, 1.0 - value);
            }
        }
        "apply_module_preset" => {
            let id = string(args, "id")?;
            let node = candidate
                .nodes
                .iter_mut()
                .find(|node| node.id == id)
                .ok_or("module not found")?;
            crate::presets::apply(node, &args["preset"])?;
        }
        "initialize_modules" | "randomize_modules" => {
            let ids = parameter_action_ids(candidate, args)?;
            let actions = ModuleParameterActions::new(candidate.sample_rate);
            let seed = if name == "randomize_modules" {
                Some(
                    args.get("seed")
                        .and_then(Value::as_u64)
                        .and_then(|value| u32::try_from(value).ok())
                        .ok_or("seed must be an integer in 0..4294967295")?,
                )
            } else {
                None
            };
            for id in ids {
                let node = candidate
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == id)
                    .unwrap();
                if let Some(seed) = seed {
                    if node.kind == "sh_asr" {
                        continue;
                    }
                    actions.randomize(node, module_random_seed(&id, seed))?;
                } else {
                    actions.initialize(node)?;
                    initialized.insert(id);
                }
            }
        }
        "set_output" => {
            candidate.output = Some(string(args, "id")?.into());
            candidate.output_port = optional_string(args, "port", "out")?.into();
        }
        _ => return Err(format!("unknown graph edit: {name}")),
    }
    Ok(())
}

fn parameter_action_ids(candidate: &Patch, args: &Value) -> Result<BTreeSet<String>, String> {
    let requested = args
        .get("ids")
        .and_then(Value::as_array)
        .ok_or("ids must be an array")?;
    if requested.is_empty() || requested.len() > MAX_EDITS {
        return Err(format!("ids must contain 1..{MAX_EDITS} unique module IDs"));
    }
    let mut ids = BTreeSet::new();
    for value in requested {
        let id = value.as_str().ok_or("each module ID must be a string")?;
        if !ids.insert(id.to_owned()) {
            return Err(format!("duplicate module ID '{id}'"));
        }
        if !candidate.nodes.iter().any(|node| node.id == id) {
            return Err(format!("module '{id}' not found"));
        }
    }
    Ok(ids)
}

/// UTF-8 FNV-1a32, wrapping after each multiplication, XOR the request seed.
/// A module's randomization is independent of selection order and other targets.
fn module_random_seed(id: &str, seed: u32) -> u32 {
    id.bytes().fold(2_166_136_261_u32, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(16_777_619)
    }) ^ seed
}

fn duplicate_modules(candidate: &mut Patch, args: &Value) -> Result<(), String> {
    let requested = args
        .get("modules")
        .and_then(Value::as_array)
        .ok_or("modules must be an array")?;
    if requested.is_empty() || requested.len() > MAX_EDITS {
        return Err(format!("modules must contain 1..{MAX_EDITS} mappings"));
    }
    let cable_mode = optional_string(args, "cable_mode", "none")?;
    if !matches!(cable_mode, "none" | "internal" | "inputs" | "all") {
        return Err("cable_mode must be none, internal, inputs, or all".into());
    }
    let mut mapping = BTreeMap::new();
    let mut destinations = BTreeSet::new();
    let mut clones = Vec::with_capacity(requested.len());
    for item in requested {
        let object = item.as_object().ok_or("module mapping must be an object")?;
        if object.keys().any(|key| key != "id" && key != "new_id") {
            return Err("module mapping accepts only id and new_id".into());
        }
        let id = string(item, "id")?;
        let new_id = string(item, "new_id")?;
        for value in [id, new_id] {
            if value.is_empty()
                || value.len() > 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_-.".contains(&byte))
            {
                return Err(
                    "node IDs must contain 1..=64 ASCII letters, digits, '_', '-', or '.'".into(),
                );
            }
        }
        if mapping.insert(id, new_id).is_some() {
            return Err(format!("duplicate source module '{id}'"));
        }
        if !destinations.insert(new_id) {
            return Err(format!("duplicate destination module '{new_id}'"));
        }
        if candidate.nodes.iter().any(|node| node.id == new_id) {
            return Err(format!("destination module '{new_id}' already exists"));
        }
        let mut node = candidate
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or_else(|| format!("source module '{id}' not found"))?
            .clone();
        node.id = new_id.to_owned();
        clones.push(node);
    }
    // Snapshot all originals before appending. Cable limits, cycles, and other
    // graph constraints are checked on the final transaction like other edits.
    let cables: Vec<Cable> = candidate
        .cables
        .iter()
        .filter_map(|original| {
            let destination = mapping.get(original.to.as_str());
            let source = mapping.get(original.from.as_str());
            let copy = match cable_mode {
                "internal" => source.is_some() && destination.is_some(),
                "inputs" => destination.is_some(),
                "all" => source.is_some() || destination.is_some(),
                _ => false,
            };
            if !copy {
                return None;
            }
            let mut cable = original.clone();
            cable.id = None;
            cable.input_order = None;
            cable.output_order = None;
            if let Some(destination) = destination {
                cable.to = (*destination).to_owned();
            }
            if let Some(source) = source {
                cable.from = (*source).to_owned();
            }
            Some(cable)
        })
        .collect();
    candidate.nodes.extend(clones);
    let first_new = candidate.cables.len();
    candidate.cables.extend(cables);
    let fresh: Vec<_> = (first_new..candidate.cables.len()).collect();
    if !fresh.is_empty() {
        openpatch_core::assign_new_cable_orders(&mut candidate.cables, &fresh)?;
    }
    candidate.assign_missing_cable_ids()?;
    Ok(())
}

fn tool(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
    read_only: bool,
) -> Value {
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false},"annotations":{"readOnlyHint":read_only,"destructiveHint":false,"idempotentHint":read_only,"openWorldHint":false}})
}

fn disconnect_tool(string_schema: &Value, id_schema: &Value, revision_schema: &Value) -> Value {
    let mut schema = tool("disconnect", "Disconnect exactly one cable_id, or every cable on a legacy to/input pair. Selectors are mutually exclusive; surviving cables keep IDs and order.",
        json!({"cable_id":id_schema,"to":string_schema,"input":string_schema,"expected_revision":revision_schema}), &[], false);
    schema["inputSchema"]["oneOf"] = json!([
        {"required":["cable_id"],"not":{"anyOf":[{"required":["to"]},{"required":["input"]}]}},
        {"required":["to","input"],"not":{"required":["cable_id"]}}
    ]);
    schema
}

pub fn tools() -> Value {
    let s = json!({"type":"string"});
    let rev = json!({"type":"integer","minimum":0});
    let module_id =
        json!({"type":"string","minLength":1,"maxLength":64,"pattern":"^[A-Za-z0-9_.-]+$"});
    let duplicate_mappings = json!({"type":"array","minItems":1,"maxItems":MAX_EDITS,"items":{
        "type":"object","properties":{"id":module_id,"new_id":module_id},
        "required":["id","new_id"],"additionalProperties":false
    }});
    let module_ids = json!({"type":"array","minItems":1,"maxItems":MAX_EDITS,"uniqueItems":true,"items":module_id});
    let cable_ids = json!({"type":"array","minItems":1,"maxItems":openpatch_core::MAX_CABLES,"uniqueItems":true,"items":module_id});
    let color = json!({"type":"array","minItems":3,"maxItems":3,"items":{"type":"integer","minimum":0,"maximum":255}});
    let midi_events = midi_events_schema();
    let module_events = module_events_schema();
    let edits = json!({"type":"array","minItems":1,"maxItems":MAX_EDITS,"items":{
        "type":"object","properties":{"tool":{"type":"string","enum":EDIT_TOOLS},"arguments":{"type":"object"}},
        "required":["tool","arguments"],"additionalProperties":false
    }});
    let construction_edits =
        json!({"type":"array","minItems":1,"maxItems":8,"items":edits["items"]});
    let construction_steps = json!({"type":"array","minItems":1,"maxItems":64,"items":{
        "type":"object","properties":{"label":{"type":"string","maxLength":160},"edits":construction_edits},
        "required":["label","edits"],"additionalProperties":false
    }});
    let plan_id = json!({"type":"integer","minimum":1});
    json!([
        tool("start_construction","Preflight every intermediate graph on an isolated snapshot, then progressively apply a reviewed plan to this session. Accepts 1..64 labelled steps, 1..8 graph edits per step, at most 256 edits total; labels/title are bounded to 160/128 UTF-8 bytes. Each step commits one revision and ordinary undo entry after its interval. External edits, undo, redo, or layout checkpoints stop the plan as conflicted; the plan never rebases. A client disconnect does not cancel it.",json!({"expected_revision":rev,"title":{"type":"string","maxLength":128},"steps":construction_steps,"interval_ms":{"type":"integer","minimum":100,"maximum":2000,"default":500}}),&["expected_revision","steps"],false),
        tool("inspect_construction","Read the latest construction plan and current revision, or idle when no plan exists. Optional plan_id must identify the latest retained plan. Additive last_commit reports the latest successful step's exact plan/zero-based step/revisions/label and up to 128 total new_modules, new_cables, and visible effective parameter_changes. Module replacements are arrivals; cables require an ID absent before the step. Equal or hidden values and parameters on arrivals are excluded. The receipt survives pause, cancellation, and completion; a new plan clears it.",json!({"plan_id":plan_id}),&[],true),
        tool("pause_construction","Pause a running construction plan. The serialized receipt guarantees no later step commits until an explicit valid resume.",json!({"plan_id":plan_id}),&["plan_id"],false),
        tool("resume_construction","Resume only a paused plan when both its expected revision and the caller's required expected_revision match the current session. A mismatch conflicts the plan and requires a newly reviewed plan.",json!({"plan_id":plan_id,"expected_revision":rev}),&["plan_id","expected_revision"],false),
        tool("cancel_construction","Cancel the latest construction plan and retain all committed steps. The receipt guarantees no later step commits. Undo committed steps individually through normal history.",json!({"plan_id":plan_id}),&["plan_id"],false),
        tool("trigger_module","Queue a live module action: seq3 clock/run/reset, logic b_press/b_release, process gate_press/gate_release, or pulses pulse1_press/pulse1_release through pulse10_press/pulse10_release, or push push_press/push_release/hold_press/hold_release, or gates reset_press/reset_release, or random_values/adsr_eg push_press/push_release, or sh_asr push_press/push_release/reset_press/reset_release. Matching releases remain available while bypassed and use an emergency release if the queue is full. Returns a queue receipt, not confirmation of callback application. Requires a live backend; does not edit the patch or history.",json!({"id":s,"action":{"type":"string","enum":MODULE_ACTION_NAMES},"expected_revision":rev}),&["id","action"],false),
        tool("inspect_scope","Read a coherent live rack Scope capture for one voice (1..16). Returns both axes as 256 raw-voltage min/max bins, acquisition settings, channel counts, applied revision and callback frame. Empty bins are null. Includes raw minimum/maximum/peak-to-peak statistics for the selected voice and all active voices, volts/screen, and actual trigger-source channels. Statistics cover stored completed bins, which may mix successive sweeps; they are not RMS or a timestamped sample window. Publication is bounded and may lag; unavailable is distinct from silence. Requires desktop audio attachment; never performs an offline render or edits history.",json!({"id":s,"channel":{"type":"integer","minimum":1,"maximum":16,"default":1},"expected_revision":rev}),&["id"],true),
        tool("inspect_performance","Read live queue, callback, and emergency-release counters plus coherent per-module state, momentary controls, and lights. Native audio_timing reports a separately coherent completed callback with actual frames/rate, processing duration, budget utilization/peaks and budget overruns (not hardware xruns); it has its own revision, sequence and reset epoch. Each module reports availability and the actual published revision; state is unavailable until the current graph is published. Queued receipts and applied/dropped counters are distinct; headless mode reports unsupported.",json!({"expected_revision":rev}),&[],true),
        tool("rotate_sequence","Rotate all eight seq3 columns left or right, including three voltage rows and gate flags. Commits one revision and one undo step.",json!({"id":s,"direction":{"type":"string","enum":["left","right"]},"expected_revision":rev}),&["id","direction"],false),
        tool("inspect_history","Read shared GUI/MCP undo and redo counts. History contains up to 128 graph edits and native module-movement checkpoints; file writes and view pan/zoom are not included.",json!({}),&[],true),
        tool("apply_edits","Atomically apply 1..256 graph edits, validate the final graph, and commit one revision and undo step. Any error leaves state unchanged. Each edit has tool and arguments; revisions belong on this outer transaction.",json!({"edits":edits,"expected_revision":rev}),&["edits"],false),
        tool("preview_edits","Validate graph edits and return the proposed patch without committing, changing history, or playing sound. Use the returned revision with apply_edits.",json!({"edits":edits,"expected_revision":rev}),&["edits"],true),
        tool("undo","Undo one shared GUI/MCP edit or native module-movement checkpoint. Revision increases; file writes and view pan/zoom are not undone.",json!({"expected_revision":rev}),&[],false),
        tool("redo","Redo one undone graph edit. A new graph edit clears redo history.",json!({"expected_revision":rev}),&[],false),
        tool("catalog","List built-in modules, named ports, polyphony policies, patch-version requirements, parameter defaults and ranges.",json!({}),&[],true),
        tool("inspect_patch","Read the session's patch and revision. In desktop MCP mode this is the same live patch shown in the native window.",json!({}),&[],true),
        tool("capture_project_patch","Capture a project-save patch with actual SEQ3 Run, Push HOLD and all Random Values and Shift Register held voltages plus next PRNG state overlaid on a cloned graph. All live stateful modules must be available at the current revision and the same callback frame; otherwise retry. Without a backend uses initial/default state. No stateful modules means parameter-only capture without reading the backend. Returns patch, revision, state_id, running, holding, random_values and shift_registers maps, state_source, and frame. Does not change session, history, queue, or runtime.",json!({"expected_revision":rev}),&[],true),
        tool("export_visualizer","Export the bundled audio-reactive WebGL2 visualizer preview to a new .html file. Includes editable GLSL. Open locally in a browser.",json!({"filename":s}),&["filename"],false),
        tool("replace_patch","Modules start with fresh runtime state. Atomically replace the complete patch. Invalid patches leave state unchanged.",json!({"patch":{"type":"object"},"expected_revision":rev}),&["patch"],false),
        tool("add_module","Add a built-in module by stable ID. Consult catalog for kind and params.",json!({"id":s,"kind":s,"params":{"type":"object","additionalProperties":{"type":"number"}},"score":{"type":"object"},"expected_revision":rev}),&["id","kind"],false),
        tool("duplicate_modules","Atomically duplicate 1..256 modules to explicit fresh IDs, preserving kind, parameters, and bypass. cable_mode defaults to none (no cables); internal copies only cables between selected sources, inputs also copies external incoming cables, and all copies each cable touching selected modules once, including external outgoing fanout. Selected cable ends are remapped; external ends are retained. Original cable order/IDs stay intact; appended copies have fresh IDs. Selected output is preserved. Version 1 rejects outgoing stacks unless the final transaction removes the conflict. Current SEQ-3 Run, Push HOLD and all Random Values and Shift Register voltages plus PRNG are copied; other runtime state starts fresh for the new IDs. Supports preview/batches and one undo step; the final graph must validate.",json!({"modules":duplicate_mappings,"cable_mode":{"type":"string","enum":["none","internal","inputs","all"],"default":"none"},"expected_revision":rev}),&["modules"],false),
        tool("remove_module","Remove a module and its attached cables.",json!({"id":s,"expected_revision":rev}),&["id"],false),
        tool("connect","Connect named ports; output defaults to out. Default mode reject refuses occupied inputs; explicit mode stack adds another version-2 cable, including identical routes. Optional cable_id requests a fresh unique ID; otherwise IDs are assigned immediately. Optional color persists three RGB integer bytes (0..255); omission uses the rack palette. Version 1 rejects stacking/cycles; version 2 delays every cable one sample.",json!({"from":s,"output":s,"to":s,"input":s,"mode":{"type":"string","enum":["reject","stack"],"default":"reject"},"cable_id":module_id,"color":{"type":"array","minItems":3,"maxItems":3,"items":{"type":"integer","minimum":0,"maximum":255}},"expected_revision":rev}),&["from","to","input"],false),
        disconnect_tool(&s, &module_id, &rev),
        tool("set_cable_color","Set one existing cable's persisted RGB bytes without changing its ID, route or DSP state. A direct equal-color call validates revision and arguments but preserves history and redo. Supports preview and atomic batches.",json!({"cable_id":module_id,"color":color,"expected_revision":rev}),&["cable_id","color"],false),
        tool("disconnect_cables","Atomically remove 1..1024 unique existing cable IDs in one revision and undo step. Every ID must exist; invalid/stale requests change nothing. Untouched cable IDs, colors, order and runtime state are preserved.",json!({"cable_ids":cable_ids,"expected_revision":rev}),&["cable_ids"],false),
        tool("move_cables","Move the input or output endpoint of 1..1024 unique existing cable IDs to a named port of the matching direction. Keeps IDs/colors and saved array positions; collision evaluation follows supplied ID order against unaffected routes and earlier accepted moves. duplicate_mode reject fails on identical routes; discard deletes only conflicting selected members; stack permits identical routes in version 2. Moved ends promote independently of the opposite plug ranks. Optional plug_order is a selected-ID permutation controlling bottom-to-top bundle order independently of collision winners. Unchanged endpoints are retained even in identical stacks; promote true applies order promotion even on same-origin return. A direct all-unchanged request without effective promotion is a validated no-op preserving redo. Destination validation and final graph validation are atomic; version 1 still rejects occupied inputs/cycles. Changed/deleted selected cable delay frames reset; source module and untouched cable state continue.",json!({"cable_ids":cable_ids,"end":{"type":"string","enum":["input","output"]},"node":module_id,"port":s,"duplicate_mode":{"type":"string","enum":["reject","discard","stack"],"default":"reject"},"promote":{"type":"boolean","default":false},"plug_order":cable_ids,"expected_revision":rev}),&["cable_ids","end","node","port"],false),
        tool("promote_cables","Promote 1..1024 unique existing cable plugs at the input or output end, preserving their current relative plug order. Only chosen-end rank metadata changes; IDs, colors, array order, opposite ranks, and DSP state are preserved. Already-top selections are validated no-ops preserving revision/history/redo. Supports preview and batches; exact ranks restore through undo/redo.",json!({"cable_ids":cable_ids,"end":{"type":"string","enum":["input","output"]},"expected_revision":rev}),&["cable_ids","end"],false),
        tool("clear_cables","Remove every cable atomically, preserving modules, parameters, output selection, and live module state. One revision and undo step restores exact cable IDs and order. Supports preview_edits/apply_edits and the full patch cable limit. A direct call on an empty patch validates expected_revision but does not change history or revision; an empty subedit follows normal transaction history semantics.",json!({"expected_revision":rev}),&[],false),
        tool("set_score","Replace structured score patterns and automation atomically. Only score_sequencer accepts score data; consult its catalog score_schema. Changed score data resets its transport; unrelated edits preserve playback. Undoable and valid in apply_edits.",json!({"id":s,"score":{"type":"object"},"expected_revision":rev}),&["id","score"],false),
        tool("set_parameter","Set a finite parameter within catalog limits. An optional history_group merges consecutive changes to the same parameter into one undo step; use a new unique group for each gesture.",json!({"id":s,"parameter":s,"value":{"type":"number"},"expected_revision":rev,"history_group":{"type":"string","minLength":1,"maxLength":128}}),&["id","parameter","value"],false),
        tool("set_bypass","Set module bypass using a boolean. Preserves cables, parameters, and frozen runtime state; bypass routes are listed in the catalog.",json!({"id":s,"bypassed":{"type":"boolean"},"expected_revision":rev}),&["id","bypassed"],false),
        tool("invert_mutes","Invert all ten mutes10 mute flags atomically. Omitted flags default to passing (zero). Commits one revision and undo step.",json!({"id":s,"expected_revision":rev}),&["id"],false),
        tool("capture_module_preset","Capture a native module preset with effective parameter defaults and explicit bypass. Stateful modules capture actual Run, HOLD, or voltages and RNG from revision-coherent live telemetry, or deterministic initial state without a live backend. Returns exact preset contents, revision, state_source and coherent frame; transient controls, phase, cables, IDs and layout are excluded.",json!({"id":s,"expected_revision":rev}),&["id"],true),
        tool("save_module_preset","Save exactly the supplied, reviewed native preset to a new workspace .opreset file. Capture first with capture_module_preset. Never recaptures a module, overwrites a file, or changes graph/history. Files are bounded to 64 KiB; symlinks and directories are rejected.",json!({"filename":s,"preset":preset_tools::schema()}),&["filename","preset"],false),
        tool("read_module_preset","Read and validate one bounded regular workspace .opreset file. Returns immutable preset contents for review and inline apply; no graph/history change and no .vcvm compatibility.",json!({"filename":s}),&["filename"],true),
        tool("list_module_presets","Browse sorted workspace .opreset files, optionally filtering valid entries by module kind. Corrupt or unsafe entries remain visible with errors. Filename query matches case-insensitively before pagination. Paged results default to 50, maximum 100; library is bounded to 1024 files.",json!({"kind":s,"query":{"type":"string","maxLength":120},"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100,"default":50}}),&[],true),
        tool("apply_module_preset","Merge reviewed native preset params and optional bypass into a matching module kind. Omitted settings remain unchanged; no Initialize or new instance. Explicit SEQ3 running, Push holding, or a complete Random Values or Shift Register snapshot assigns actual saved state after adoption even if its configured value is unchanged. Atomic batch/preview support and one history step; undo/redo restore captured persistent state.",json!({"id":s,"preset":preset_tools::schema(),"expected_revision":rev}),&["id","preset"],false),
        tool("initialize_modules","Initialize saved parameters for 1..256 unique module IDs using catalog policy, preserving bypass, identity, cables, and output. Applies audited per-kind runtime initialization after live graph publication; this is not a fresh-instance reset. One undo step; undo/redo restore saved values and captured SEQ3 Run without replaying initialization. Supports preview and atomic batches.",json!({"ids":module_ids,"expected_revision":rev}),&["ids"],false),
        tool("randomize_modules","Deterministically randomize eligible saved parameters for 1..256 unique module IDs using catalog distributions. Preserves bypass, identity, cables, and output. Required u32 seed; each module receives UTF-8 FNV-1a32(id) XOR seed, with wrapping multiplication. Selection order does not affect results. Shift Register is entirely unchanged, including live values and RNG. Random Values rerolls all 112 voltages in the selected range using max(1, derived module seed) as its xorshift32 initial state, independently of live PRNG position, preserving range and visible seed. Preview and apply agree; undo/redo restore values and captured SEQ3 Run without rerolling.",json!({"ids":module_ids,"seed":{"type":"integer","minimum":0,"maximum":4294967295_u64},"expected_revision":rev}),&["ids","seed"],false),
        tool("set_output","Select a module output port (default out). audio_stereo selects explicit L/R output; other outputs monitor channel 1. Add poly_sum for a mono mix of all voices.",json!({"id":s,"port":s,"expected_revision":rev}),&["id"],false),
        tool("save_patch","Save a coherent patch JSON snapshot with current persistent module state inside the workspace. Live state must be available at the current revision and one callback frame. Simple filename only; never overwrites.",json!({"filename":s}),&["filename"],false),
        tool("load_patch","Modules start with fresh runtime state. Load workspace patch JSON, validate, then replace the session patch.",json!({"filename":s,"expected_revision":rev}),&["filename"],false),
        tool("render_arrangement","Stream up to 300 seconds of the current patch from reset to a new PCM16 WAV using bounded memory. For scores and autonomous patches; no MIDI/module event trace. Does not play speakers, alter the live graph, or overwrite files. Returns preclip peak/RMS and clipped sample count.",json!({"filename":s,"seconds":{"type":"number","minimum":0.001,"maximum":300}}),&["filename","seconds"],false),
        tool("render_audio","Render offline from reset to 16-bit WAV in workspace, stereo only for a selected audio_stereo module. Optional sample-frame MIDI and module events drive reproducible performances. Does not play speakers or change history. Never overwrites.",json!({"filename":s,"seconds":{"type":"number","minimum":0.001,"maximum":30},"midi_events":midi_events,"module_events":module_events}),&["filename","seconds"],false),
        tool("analyze_signal","Measure a named output offline from reset with optional sample-frame MIDI and module events. Reports each channel's active frames, mean, RMS, peak and range. No speaker playback or live-state mutation. Raw module signal units; audio_out/audio_stereo convert volts to digital full scale.",json!({"id":s,"port":s,"seconds":{"type":"number","minimum":0.001,"maximum":2},"midi_events":midi_events,"module_events":module_events,"expected_revision":rev}),&["id","seconds"],true),
        tool("capture_signal","Capture a calibrated waveform from any named output/channel offline from reset. Returns bounded nonempty min/max/mean bins preserving every active-sample peak, plus active-sample DC/RMS/peak. Raw output units come from catalog; no live audio, summing, clipping, or history changes. Optional sample-frame MIDI and module events reproduce performances; expected_revision guards the captured snapshot.",json!({"id":s,"port":s,"channel":{"type":"integer","minimum":1,"maximum":16,"default":1},"seconds":{"type":"number","minimum":0.001,"maximum":2},"points":{"type":"integer","minimum":64,"maximum":2048,"default":512},"midi_events":midi_events,"module_events":module_events,"expected_revision":rev}),&["id","seconds"],true),
        tool("benchmark","Measure this patch's offline DSP throughput, excluding compilation. Release builds recommended. No VCV Rack comparison.",json!({"seconds":{"type":"number","minimum":0.01,"maximum":5}}),&[],true)
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use openpatch_core::render;

    fn session() -> Session {
        Session::new(&std::env::temp_dir()).unwrap()
    }

    fn edits() -> Value {
        json!({"expected_revision":0,"edits":[
            {"tool":"connect","arguments":{"from":"source","to":"out","input":"signal"}},
            {"tool":"add_module","arguments":{"id":"source","kind":"constant","params":{"value":0.25}}},
            {"tool":"add_module","arguments":{"id":"out","kind":"output"}},
            {"tool":"set_output","arguments":{"id":"out"}}
        ]})
    }

    #[test]
    fn preview_and_batch_validate_final_graph_and_commit_once() {
        let mut s = session();
        let before = s.snapshot();
        let preview = s.execute("preview_edits", edits()).unwrap();
        assert_eq!(s.snapshot(), before);
        assert_eq!(preview["committed"], false);
        assert_eq!(
            preview["changes"]["added_modules"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            preview["changes"]["added_cables"].as_array().unwrap().len(),
            1
        );
        let result = s.execute("apply_edits", edits()).unwrap();
        assert_eq!(result["patch"], preview["patch"]);
        assert_eq!(result["revision"], 1);
        assert_eq!(result["history"]["undo_steps"], 1);
        assert_eq!(render(s.patch(), 0.001).unwrap()[0], 0.0);
        assert_eq!(render(s.patch(), 0.001).unwrap()[1], 0.25);
        let undone = s.execute("undo", json!({"expected_revision":1})).unwrap();
        assert_eq!(undone["patch"], before["patch"]);
        assert_eq!(undone["revision"], 2);
        assert_eq!(undone["state_id"], before["state_id"]);
        let redone = s.execute("redo", json!({"expected_revision":2})).unwrap();
        assert_eq!(redone["patch"], preview["patch"]);
        assert_eq!(redone["revision"], 3);
        assert_eq!(redone["state_id"], result["state_id"]);
    }

    #[test]
    fn failed_batch_and_stale_edits_preserve_graph_history_and_revision() {
        let mut s = session();
        s.execute("apply_edits", edits()).unwrap();
        s.execute("undo", json!({})).unwrap();
        let before = s.snapshot();
        let mut invalid = edits();
        invalid["expected_revision"] = json!(2);
        invalid["edits"][3]["arguments"]["id"] = json!("missing");
        for tool in ["apply_edits", "preview_edits"] {
            assert!(s.execute(tool, invalid.clone()).is_err());
            assert_eq!(s.snapshot(), before);
            assert!(s
                .execute(tool, edits())
                .unwrap_err()
                .contains("revision conflict"));
            assert_eq!(s.snapshot(), before);
        }
        assert!(s.execute("redo", json!({"expected_revision":0})).is_err());
        assert_eq!(s.snapshot(), before);
        s.execute("redo", json!({"expected_revision":2})).unwrap();
    }

    #[test]
    fn transaction_rejects_io_nested_transactions_and_malformed_edits() {
        let mut s = session();
        let before = s.snapshot();
        for edit in [
            json!({"tool":"save_patch","arguments":{"filename":"forbidden.json"}}),
            json!({"tool":"apply_edits","arguments":{"edits":[]}}),
            json!({"tool":"add_module","arguments":{"id":"a","kind":"constant","typo":0}}),
            json!({"tool":"add_module","arguments":{"id":"a"}}),
            json!({"tool":"add_module","arguments":{"id":"a","kind":"constant","expected_revision":0}}),
            json!({"tool":"add_module","arguments":{"id":"a","kind":"constant"},"extra":true}),
        ] {
            let error = s
                .execute("apply_edits", json!({"edits":[edit]}))
                .unwrap_err();
            assert!(error.starts_with("edit 1:"), "{error}");
            assert_eq!(s.snapshot(), before);
        }
        for edits in [vec![], vec![json!({}); MAX_EDITS + 1]] {
            assert!(s.execute("apply_edits", json!({"edits":edits})).is_err());
        }
        assert_eq!(s.snapshot(), before);
    }

    #[test]
    fn new_edit_clears_redo_and_history_is_bounded() {
        let mut s = session();
        s.execute("add_module", json!({"id":"a","kind":"constant"}))
            .unwrap();
        for i in 0..150 {
            s.execute(
                "set_parameter",
                json!({"id":"a","parameter":"value","value":i as f32 / 100.0}),
            )
            .unwrap();
        }
        assert_eq!(s.undo.len(), HISTORY_LIMIT);
        for _ in 0..HISTORY_LIMIT {
            s.execute("undo", json!({})).unwrap();
        }
        assert!(!s.can_undo());
        assert!(s.can_redo());
        let before = s.snapshot();
        assert!(s.execute("undo", json!({})).is_err());
        assert_eq!(s.snapshot(), before);
        s.execute(
            "set_parameter",
            json!({"id":"a","parameter":"value","value":-0.5}),
        )
        .unwrap();
        assert!(!s.can_redo());
        assert!(s.execute("redo", json!({})).is_err());
    }

    #[test]
    fn knob_gesture_groups_only_consecutive_changes_to_same_parameter() {
        let mut s = session();
        s.execute(
            "add_module",
            json!({"id":"a","kind":"constant","params":{"value":0.0}}),
        )
        .unwrap();
        for value in [0.2, 0.4, 0.8] {
            s.execute(
                "set_parameter",
                json!({"id":"a","parameter":"value","value":value,"history_group":"gesture1"}),
            )
            .unwrap();
        }
        assert_eq!(s.undo.len(), 2);
        assert_eq!(s.revision(), 4);
        assert_eq!(s.state_id(), 2, "one gesture retains its history identity");
        assert_eq!(s.history_state_ids().count(), 3);
        s.execute("undo", json!({})).unwrap();
        assert_eq!(s.patch.nodes[0].params["value"], 0.0);
        s.execute("redo", json!({})).unwrap();
        assert_eq!(s.patch.nodes[0].params["value"], 0.8);
        s.execute(
            "set_parameter",
            json!({"id":"a","parameter":"value","value":1.0,"history_group":"gesture1"}),
        )
        .unwrap();
        assert_eq!(s.undo.len(), 3, "undo/redo must break grouping");
        let before = s.snapshot();
        assert!(s
            .execute(
                "set_parameter",
                json!({"id":"a","parameter":"value","value":1.0,"history_group":false})
            )
            .is_err());
        assert_eq!(s.snapshot(), before);
    }

    #[test]
    fn polyphonic_analysis_named_routing_and_output_selection_share_validation() {
        let mut s = session();
        assert_eq!(s.patch.version, 2);
        s.execute("apply_edits", json!({"edits":[
            {"tool":"add_module","arguments":{"id":"bus","kind":"poly_constant","params":{"channels":2,"v1":0.25,"v2":-0.5}}},
            {"tool":"add_module","arguments":{"id":"split","kind":"poly_split"}},
            {"tool":"add_module","arguments":{"id":"out","kind":"output"}},
            {"tool":"connect","arguments":{"from":"bus","to":"split","input":"signal"}},
            {"tool":"connect","arguments":{"from":"split","output":"out2","to":"out","input":"signal"}},
            {"tool":"set_output","arguments":{"id":"split","port":"out2"}}
        ]})).unwrap();
        assert_eq!(s.patch.output_port, "out2");
        assert_eq!(s.patch.cables[1].output, "out2");
        let before = s.snapshot();
        let analysis = s
            .execute("analyze_signal", json!({"id":"bus","seconds":0.001}))
            .unwrap();
        assert_eq!(analysis["max_channels"], 2);
        assert_eq!(analysis["channels"][0]["active_frames"], 48);
        assert_eq!(analysis["channels"][0]["mean"], 0.25);
        assert_eq!(analysis["channels"][1]["mean"], -0.5);
        assert_eq!(analysis["channels"][1]["rms"], 0.5);
        let split = s
            .execute(
                "analyze_signal",
                json!({"id":"split","port":"out2","seconds":0.001}),
            )
            .unwrap();
        assert_eq!(split["max_channels"], 1);
        assert_eq!(split["channels"][0]["min"], -0.5);
        assert_eq!(split["channels"][0]["max"], 0.0);
        assert_eq!(s.snapshot(), before);
        for (tool, arguments) in [
            ("set_output", json!({"id":"split","port":"missing"})),
            (
                "analyze_signal",
                json!({"id":"bus","port":12,"seconds":0.001}),
            ),
            ("analyze_signal", json!({"id":"missing","seconds":0.001})),
            ("analyze_signal", json!({"id":"bus","seconds":20})),
        ] {
            assert!(s.execute(tool, arguments).is_err());
            assert_eq!(s.snapshot(), before);
        }
        let preview = s.execute("preview_edits", json!({"edits":[
            {"tool":"disconnect","arguments":{"to":"out","input":"signal"}},
            {"tool":"connect","arguments":{"from":"split","output":"out1","to":"out","input":"signal"}}
        ]})).unwrap();
        assert_eq!(preview["changes"]["removed_cables"][0]["output"], "out2");
        assert_eq!(preview["changes"]["added_cables"][0]["output"], "out1");
        assert_eq!(s.snapshot(), before);
        s.execute(
            "set_parameter",
            json!({"id":"bus","parameter":"channels","value":0}),
        )
        .unwrap();
        let empty = s
            .execute("analyze_signal", json!({"id":"bus","seconds":0.001}))
            .unwrap();
        assert_eq!(empty["min_channels"], 0);
        assert_eq!(empty["max_channels"], 0);
        assert_eq!(empty["channels"], json!([]));
    }
    #[test]
    fn stale_close_decision_does_not_seal_or_prevent_a_new_edit() {
        let mut s = session();
        s.execute("apply_edits", edits()).unwrap();
        let before = s.snapshot();
        assert!(s
            .seal_for_close(0)
            .unwrap_err()
            .contains("revision conflict"));
        assert!(!s.closing);
        assert_eq!(s.snapshot(), before);
        s.execute(
            "set_parameter",
            json!({"id":"source","parameter":"value","value":0.5,"expected_revision":1}),
        )
        .unwrap();
        assert_eq!(s.revision(), 2);
    }

    #[test]
    fn successful_close_seal_rejects_every_mutation_and_preserves_history() {
        let mut s = session();
        s.execute("apply_edits", edits()).unwrap();
        s.execute(
            "set_parameter",
            json!({"id":"source","parameter":"value","value":0.5}),
        )
        .unwrap();
        s.execute("undo", json!({})).unwrap();
        assert!(s.can_undo());
        assert!(s.can_redo());
        let before = s.snapshot();
        let revision = s.revision();
        s.seal_for_close(revision).unwrap();
        s.seal_for_close(revision).unwrap();
        let requests = [
            ("replace_patch", json!({"patch":Patch::new_v2()})),
            ("load_patch", json!({"filename":"missing.json"})),
            ("add_module", json!({"id":"new","kind":"constant"})),
            ("remove_module", json!({"id":"source"})),
            (
                "connect",
                json!({"from":"source","to":"out","input":"signal"}),
            ),
            ("disconnect", json!({"to":"out","input":"signal"})),
            (
                "set_parameter",
                json!({"id":"source","parameter":"value","value":1.0}),
            ),
            ("set_output", json!({"id":"source"})),
            (
                "apply_edits",
                json!({"edits":[{"tool":"remove_module","arguments":{"id":"source"}}]}),
            ),
            ("undo", json!({})),
            ("redo", json!({})),
        ];
        for (tool, args) in requests {
            assert_eq!(
                s.execute(tool, args).unwrap_err(),
                "session is closing",
                "{tool}"
            );
            assert_eq!(s.snapshot(), before);
        }
        assert_eq!(
            s.replace_patch(Patch::new_v2(), Some(revision))
                .unwrap_err(),
            "session is closing"
        );
        assert_eq!(s.snapshot(), before);
        assert_eq!(s.execute("inspect_patch", json!({})).unwrap(), before);
        assert_eq!(
            s.execute("inspect_history", json!({})).unwrap()["history"],
            before["history"]
        );
        s.execute("catalog", json!({})).unwrap();
        s.execute("preview_edits",json!({"expected_revision":revision,"edits":[{"tool":"set_parameter","arguments":{"id":"source","parameter":"value","value":1.0}}]})).unwrap();
        assert_eq!(s.snapshot(), before);
    }

    #[test]
    fn sealed_shared_session_still_supports_snapshot_exports_render_and_analysis() {
        let workspace = std::env::temp_dir().join(format!(
            "openpatch-sealed-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut s = Session::new(&workspace).unwrap();
        s.execute("apply_edits", edits()).unwrap();
        s.seal_for_close(s.revision()).unwrap();
        let before = s.snapshot();
        let shared = s.into_shared();
        for (tool, args) in [
            ("save_patch", json!({"filename":"sealed.json"})),
            ("export_visualizer", json!({"filename":"sealed.html"})),
            (
                "render_audio",
                json!({"filename":"sealed.wav","seconds":0.001}),
            ),
            ("analyze_signal", json!({"id":"source","seconds":0.001})),
            ("benchmark", json!({"seconds":0.01})),
        ] {
            execute_shared(&shared, tool, args).unwrap();
        }
        assert_eq!(shared.lock().unwrap().snapshot(), before);
        assert!(shared.lock().unwrap().closing);
        assert_eq!(
            execute_shared(&shared, "remove_module", json!({"id":"source"})).unwrap_err(),
            "session is closing"
        );
        drop(shared);
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn new_document_clears_history_only_after_validated_revision_guarded_commit() {
        #[derive(Default)]
        struct Backend {
            modules: Mutex<Vec<String>>,
            cables: Mutex<Vec<String>>,
            revisions: Mutex<Vec<u64>>,
        }
        impl LiveModuleEvents for Backend {
            fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
                unreachable!()
            }
            fn snapshot(&self) -> Value {
                json!({})
            }
            fn retire_modules(&self, ids: &[String]) {
                self.modules.lock().unwrap().extend_from_slice(ids);
            }
            fn retire_cables(&self, ids: &[String]) {
                self.cables.lock().unwrap().extend_from_slice(ids);
            }
            fn sync_patch(&self, _: &Patch, revision: u64) {
                self.revisions.lock().unwrap().push(revision);
            }
        }
        let mut s = session();
        s.execute("apply_edits", edits()).unwrap();
        s.execute(
            "set_parameter",
            json!({"id":"source","parameter":"value","value":0.5,"history_group":"gesture"}),
        )
        .unwrap();
        s.execute("undo", json!({})).unwrap();
        let backend = Arc::new(Backend::default());
        s.set_live_module_events(Some(backend.clone()));
        let before = s.snapshot();
        let revisions = backend.revisions.lock().unwrap().clone();
        let mut invalid = Patch::new_v2();
        invalid.output = Some("missing".into());
        assert!(s.new_document(invalid, Some(s.revision())).is_err());
        assert!(s.new_document(Patch::new_v2(), Some(0)).is_err());
        assert_eq!(s.snapshot(), before);
        assert!(backend.modules.lock().unwrap().is_empty());
        assert!(backend.cables.lock().unwrap().is_empty());
        assert_eq!(*backend.revisions.lock().unwrap(), revisions);
        let old_ids = s.cable_ids();
        let old_modules = s.module_ids();
        let revision = s.new_document(Patch::new_v2(), Some(s.revision())).unwrap();
        assert_eq!(revision, before["revision"].as_u64().unwrap() + 1);
        assert_eq!(s.state_id(), revision);
        assert!(!s.can_undo() && !s.can_redo());
        assert!(s.history_group.is_none());
        assert_eq!(*backend.modules.lock().unwrap(), old_modules);
        assert_eq!(*backend.cables.lock().unwrap(), old_ids);
        assert!(s.live_module_events.is_some());
        let current = s.snapshot();
        s.seal_for_close(s.revision()).unwrap();
        assert!(s
            .new_document(openpatch_core::demo_patch(), Some(s.revision()))
            .is_err());
        assert_eq!(s.snapshot(), current);
    }
}
