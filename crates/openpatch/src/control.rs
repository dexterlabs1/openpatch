//! Control-thread operations, shared by transports. Never called by the DSP loop.
use openpatch_core::{
    catalog, render_audio_with_events, validate_midi_events, write_wav_interleaved, Cable, Node,
    Patch, PortFrame, TimedMidiEvent, BLOCK_SIZE, MAX_CHANNELS,
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

#[derive(Clone)]
pub struct Session {
    patch: Patch,
    revision: u64,
    state_id: u64,
    workspace: PathBuf,
    undo: VecDeque<HistoryEntry>,
    redo: Vec<HistoryEntry>,
    history_group: Option<(String, String, String)>,
}

#[derive(Clone)]
struct HistoryEntry {
    patch: Patch,
    state_id: u64,
}

const HISTORY_LIMIT: usize = 128;
const MAX_EDITS: usize = 256;
const EDIT_TOOLS: &[&str] = &[
    "add_module",
    "remove_module",
    "connect",
    "disconnect",
    "set_parameter",
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
    let mut live = session.lock().map_err(|_| "session lock poisoned")?;
    match name {
        "export_visualizer" | "save_patch" | "render_audio" | "benchmark" | "analyze_signal" => {
            let mut snapshot = Session {
                patch: live.patch.clone(),
                revision: live.revision,
                state_id: live.state_id,
                workspace: live.workspace.clone(),
                undo: VecDeque::new(),
                redo: Vec::new(),
                history_group: None,
            };
            drop(live);
            snapshot.execute(name, args)
        }
        _ => live.execute(name, args),
    }
}

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
            workspace: workspace.canonicalize().map_err(|e| e.to_string())?,
        })
    }

    pub fn patch(&self) -> &Patch {
        &self.patch
    }

    pub fn revision(&self) -> u64 {
        self.revision
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

    /// Commit a GUI or agent edit through the same validation and revision boundary.
    /// Failed validation or a stale revision leaves the live patch unchanged.
    pub fn replace_patch(
        &mut self,
        candidate: Patch,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        self.commit(candidate, expected_revision, None)
    }

    fn commit(
        &mut self,
        candidate: Patch,
        expected_revision: Option<u64>,
        group: Option<(String, String, String)>,
    ) -> Result<u64, String> {
        self.check_revision(expected_revision)?;
        candidate.compile()?;
        let revision = self.revision.checked_add(1).ok_or("revision exhausted")?;
        if group.is_some() && group == self.history_group {
            self.patch = candidate;
        } else {
            if self.undo.len() == HISTORY_LIMIT {
                self.undo.pop_front();
            }
            self.undo.push_back(HistoryEntry {
                patch: std::mem::replace(&mut self.patch, candidate),
                state_id: self.state_id,
            });
            self.state_id = revision;
        }
        self.history_group = group;
        self.redo.clear();
        self.revision = revision;
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
        let revision = self.revision.checked_add(1).ok_or("revision exhausted")?;
        if redo {
            let entry = self.redo.pop().ok_or("nothing to redo")?;
            self.undo.push_back(HistoryEntry {
                patch: std::mem::replace(&mut self.patch, entry.patch),
                state_id: std::mem::replace(&mut self.state_id, entry.state_id),
            });
        } else {
            let entry = self.undo.pop_back().ok_or("nothing to undo")?;
            self.redo.push(HistoryEntry {
                patch: std::mem::replace(&mut self.patch, entry.patch),
                state_id: std::mem::replace(&mut self.state_id, entry.state_id),
            });
        }
        self.revision = revision;
        self.history_group = None;
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
        let events = midi_events(args, frames)?;
        let mut event_index = 0;
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
            while event_index < events.len() && events[event_index].frame == cursor as u64 {
                engine.handle_midi(events[event_index].event);
                event_index += 1;
            }
            let until_event = events
                .get(event_index)
                .map_or(remaining, |event| event.frame as usize - cursor);
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
            "midi_event_count":events.len(),
            "state":"analyzed offline from reset","statistics":"active frames only; inactive channels excluded",
            "units":"raw module output units; consult catalog"}),
        )
    }

    pub fn execute(&mut self, name: &str, args: Value) -> Result<Value, String> {
        let schemas = tools();
        validate_arguments(&schemas, name, &args)?;
        match name {
            "catalog" => return Ok(catalog()),
            "inspect_patch" => return Ok(self.snapshot()),
            "inspect_history" => {
                return Ok(json!({"revision":self.revision,"history":self.history()}))
            }
            "analyze_signal" => return self.analyze_signal(&args),
            "export_visualizer" => {
                let path = self.file(string(&args, "filename")?, ".html")?;
                crate::visualizer::export(&path).map_err(|e| e.to_string())?;
                return Ok(
                    json!({"filename":args["filename"],"shader_profile":"webgl2-glsl-es-300-v1","instructions":"Open the HTML in a browser, select a local audio file, then press Play. This is a standalone visual preview, not a live native audio session."}),
                );
            }
            "save_patch" => {
                let path = self.file(string(&args, "filename")?, ".json")?;
                let bytes = serde_json::to_vec_pretty(&self.patch).map_err(|e| e.to_string())?;
                let mut f = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map_err(|e| e.to_string())?;
                f.write_all(&bytes).map_err(|e| e.to_string())?;
                return Ok(json!({"filename":args["filename"],"revision":self.revision}));
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
                let rendered = render_audio_with_events(&self.patch, seconds, &events)?;
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
                    "state":"rendered from reset","midi_event_count":events.len(),
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
        let mut candidate = self.patch.clone();
        match name {
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
                        apply_edit(&mut candidate, tool, arguments)
                    })();
                    result.map_err(|error: String| format!("edit {}: {error}", index + 1))?;
                }
                if name == "preview_edits" {
                    candidate.compile()?;
                    return Ok(
                        json!({"revision":self.revision,"changes":patch_changes(&self.patch, &candidate),"patch":candidate,"committed":false,"edit_count":edits.len()}),
                    );
                }
            }
            "undo" => return self.restore_history(false),
            "redo" => return self.restore_history(true),
            name if EDIT_TOOLS.contains(&name) => apply_edit(&mut candidate, name, &args)?,
            _ => return Err(format!("unknown tool: {name}")),
        }
        // Transaction boundary: failed edits never change the patch or revision.
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
        self.commit(candidate, expected_revision, group)?;
        Ok(self.snapshot())
    }
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
            (previous.kind != node.kind || previous.params != node.params)
                .then(|| json!({"id":id,"before":previous,"after":node}))
        })
        .collect();
    let endpoints = |cable: &Cable| {
        (
            cable.from.clone(),
            cable.output.clone(),
            cable.to.clone(),
            cable.input.clone(),
        )
    };
    let old_cables: BTreeSet<_> = before.cables.iter().map(endpoints).collect();
    let new_cables: BTreeSet<_> = after.cables.iter().map(endpoints).collect();
    let added_cables: Vec<_> = after
        .cables
        .iter()
        .filter(|cable| !old_cables.contains(&endpoints(cable)))
        .collect();
    let removed_cables: Vec<_> = before
        .cables
        .iter()
        .filter(|cable| !new_cables.contains(&endpoints(cable)))
        .collect();
    json!({"added_modules":added,"removed_modules":removed,"changed_modules":changed,
        "added_cables":added_cables,"removed_cables":removed_cables,
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

fn apply_edit(candidate: &mut Patch, name: &str, args: &Value) -> Result<(), String> {
    match name {
        "add_module" => {
            let params: BTreeMap<String, f32> =
                serde_json::from_value(args.get("params").cloned().unwrap_or(json!({})))
                    .map_err(|e| e.to_string())?;
            candidate.nodes.push(Node {
                id: string(args, "id")?.into(),
                kind: string(args, "kind")?.into(),
                params,
            });
        }
        "remove_module" => {
            let id = string(args, "id")?;
            if !candidate.nodes.iter().any(|n| n.id == id) {
                return Err("module not found".into());
            }
            candidate.nodes.retain(|n| n.id != id);
            candidate.cables.retain(|c| c.from != id && c.to != id);
            if candidate.output.as_deref() == Some(id) {
                candidate.output = None;
                candidate.output_port = "out".into();
            }
        }
        "connect" => {
            candidate.cables.push(Cable {
                from: string(args, "from")?.into(),
                output: optional_string(args, "output", "out")?.into(),
                to: string(args, "to")?.into(),
                input: string(args, "input")?.into(),
            });
        }
        "disconnect" => {
            let to = string(args, "to")?;
            let input = string(args, "input")?;
            let before = candidate.cables.len();
            candidate
                .cables
                .retain(|c| !(c.to == to && c.input == input));
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
        "set_output" => {
            candidate.output = Some(string(args, "id")?.into());
            candidate.output_port = optional_string(args, "port", "out")?.into();
        }
        _ => return Err(format!("unknown graph edit: {name}")),
    }
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

pub fn tools() -> Value {
    let s = json!({"type":"string"});
    let rev = json!({"type":"integer","minimum":0});
    let midi_events = midi_events_schema();
    let edits = json!({"type":"array","minItems":1,"maxItems":MAX_EDITS,"items":{
        "type":"object","properties":{"tool":{"type":"string","enum":EDIT_TOOLS},"arguments":{"type":"object"}},
        "required":["tool","arguments"],"additionalProperties":false
    }});
    json!([
        tool("inspect_history","Read shared GUI/MCP undo and redo counts. History contains up to 128 graph edits; layouts and files are not included.",json!({}),&[],true),
        tool("apply_edits","Atomically apply 1..256 graph edits, validate the final graph, and commit one revision and undo step. Any error leaves state unchanged. Each edit has tool and arguments; revisions belong on this outer transaction.",json!({"edits":edits,"expected_revision":rev}),&["edits"],false),
        tool("preview_edits","Validate graph edits and return the proposed patch without committing, changing history, or playing sound. Use the returned revision with apply_edits.",json!({"edits":edits,"expected_revision":rev}),&["edits"],true),
        tool("undo","Undo one shared GUI/MCP graph edit. Revision increases; file writes and rack layout are not undone.",json!({"expected_revision":rev}),&[],false),
        tool("redo","Redo one undone graph edit. A new graph edit clears redo history.",json!({"expected_revision":rev}),&[],false),
        tool("catalog","List built-in modules, named ports, polyphony policies, patch-version requirements, parameter defaults and ranges.",json!({}),&[],true),
        tool("inspect_patch","Read the session's patch and revision. In desktop MCP mode this is the same live patch shown in the native window.",json!({}),&[],true),
        tool("export_visualizer","Export the bundled audio-reactive WebGL2 visualizer preview to a new .html file. Includes editable GLSL. Open locally in a browser.",json!({"filename":s}),&["filename"],false),
        tool("replace_patch","Atomically replace the complete patch. Invalid patches leave state unchanged.",json!({"patch":{"type":"object"},"expected_revision":rev}),&["patch"],false),
        tool("add_module","Add a built-in module by stable ID. Consult catalog for kind and params.",json!({"id":s,"kind":s,"params":{"type":"object","additionalProperties":{"type":"number"}},"expected_revision":rev}),&["id","kind"],false),
        tool("remove_module","Remove a module and its attached cables.",json!({"id":s,"expected_revision":rev}),&["id"],false),
        tool("connect","Connect named ports; output defaults to out. Occupied inputs are rejected. Version 1 rejects cycles; version 2 permits feedback with one sample of delay on every cable.",json!({"from":s,"output":s,"to":s,"input":s,"expected_revision":rev}),&["from","to","input"],false),
        tool("disconnect","Disconnect a named destination input.",json!({"to":s,"input":s,"expected_revision":rev}),&["to","input"],false),
        tool("set_parameter","Set a finite parameter within catalog limits. An optional history_group merges consecutive changes to the same parameter into one undo step; use a new unique group for each gesture.",json!({"id":s,"parameter":s,"value":{"type":"number"},"expected_revision":rev,"history_group":{"type":"string","minLength":1,"maxLength":128}}),&["id","parameter","value"],false),
        tool("set_output","Select a module output port (default out). audio_stereo selects explicit L/R output; other outputs monitor channel 1. Add poly_sum for a mono mix of all voices.",json!({"id":s,"port":s,"expected_revision":rev}),&["id"],false),
        tool("save_patch","Save patch JSON inside the workspace. Simple filename only; never overwrites.",json!({"filename":s}),&["filename"],false),
        tool("load_patch","Load workspace patch JSON, validate, then replace the session patch.",json!({"filename":s,"expected_revision":rev}),&["filename"],false),
        tool("render_audio","Render offline from reset to 16-bit WAV in workspace, stereo only for a selected audio_stereo module. Optional sample-frame MIDI events drive midi_cv modules. Does not play speakers or change history. Never overwrites.",json!({"filename":s,"seconds":{"type":"number","minimum":0.001,"maximum":30},"midi_events":midi_events}),&["filename","seconds"],false),
        tool("analyze_signal","Measure a named output offline from reset with optional sample-frame MIDI events. Reports each channel's active frames, mean, RMS, peak and range. No speaker playback or live-state mutation. Raw module signal units; audio_out/audio_stereo convert volts to digital full scale.",json!({"id":s,"port":s,"seconds":{"type":"number","minimum":0.001,"maximum":2},"midi_events":midi_events}),&["id","seconds"],true),
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
}
