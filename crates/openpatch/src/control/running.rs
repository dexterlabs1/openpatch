//! Typed module state follows history independently of initial parameters.
use super::*;

#[derive(Default)]
pub(super) struct EditEffects {
    pub parameters: super::parameters::ParameterEffects,
    pub replacement_document: Option<ProjectDocument>,
    pub playback_mode: Option<PlaybackMode>,
    pub source_copies: BTreeMap<String, String>,
    pub cleared_source_routes: bool,
    pub learn_reconciliation: bool,
    pub lifecycle_captured: bool,
    pub forward_lifecycle: HostLifecycleDelta,
    pub reverse_lifecycle: HostLifecycleDelta,
    pub retired: Vec<String>,
    pub retired_cables: Vec<String>,
    pub initialized: BTreeSet<String>,
    // Presentation provenance only; independent of DSP cable retirement.
    pub cable_moves: BTreeMap<String, BTreeSet<String>>,
    discontinuous_cables: BTreeSet<String>,
    pub release_controls: BTreeSet<String>,
    pub before_saved: BTreeMap<String, SavedModuleState>,
    pub after_saved: BTreeMap<String, SavedModuleState>,
    pub load_saved: BTreeMap<String, SavedModuleState>,
    working_saved: BTreeMap<String, SavedModuleState>,
    full_midi_restores: BTreeSet<String>,
    replaced: BTreeSet<String>,
    // Host presentation only: requested rack rows for modules added here.
    pub placements: BTreeMap<String, u32>,
    pub(super) live_snapshot: Option<Value>,
    pub(super) live_frame: Option<u64>,
}

#[cfg(test)]
pub(super) fn initial_running(node: &Node) -> bool {
    node.params.get("running").copied().unwrap_or(1.0) == 1.0
}

pub(super) fn live_saved(
    snapshot: &Value,
    id: &str,
    revision: u64,
    expected: SavedModuleState,
) -> Result<(SavedModuleState, u64), String> {
    if snapshot["coherent"].as_bool() == Some(false) {
        return Err("live project state is not coherent; retry capture".into());
    }
    let module = &snapshot["modules"][id];
    if module["available"] != true || module["revision"].as_u64() != Some(revision) {
        return Err(format!("live state unavailable for module '{id}'; wait for revision {revision} to be published"));
    }
    if module["kind"]
        .as_str()
        .is_some_and(|kind| kind != expected.kind())
    {
        return Err(format!("live state kind mismatch for module '{id}'"));
    }
    let state = match expected {
        SavedModuleState::MidiGateMappings(_) => {
            let rows = module["state"]["assignments"]
                .as_array()
                .filter(|rows| rows.len() == 16)
                .ok_or_else(|| {
                    format!("live MIDI Gate assignments unavailable for module '{id}'")
                })?;
            let mut maps = [-1_i16; 16];
            let mut seen = BTreeSet::new();
            for (slot, value) in maps.iter_mut().zip(rows) {
                *slot = value
                    .as_i64()
                    .filter(|value| (-1..=127).contains(value))
                    .ok_or_else(|| format!("invalid MIDI Gate assignment for module '{id}'"))?
                    as i16;
                if *slot >= 0 && !seen.insert(*slot) {
                    return Err(format!(
                        "colliding live MIDI Gate assignments for module '{id}'"
                    ));
                }
            }
            SavedModuleState::MidiGateMappings(maps)
        }
        SavedModuleState::MidiCc { .. } | SavedModuleState::MidiCcMappings(_) => {
            let state = &module["state"];
            let mappings = state["assignments"]
                .as_array()
                .filter(|a| a.len() == 16)
                .ok_or_else(|| format!("live MIDI CC assignments unavailable for module '{id}'"))?;
            let controllers = state["values"]
                .as_array()
                .filter(|a| a.len() == 128)
                .ok_or_else(|| format!("live MIDI CC values unavailable for module '{id}'"))?;
            let mut assignments = [-1_i16; 16];
            let mut values = [0_u8; 128];
            for (slot, value) in assignments.iter_mut().zip(mappings) {
                *slot = value
                    .as_i64()
                    .filter(|v| (-1..=127).contains(v))
                    .ok_or_else(|| format!("invalid MIDI CC assignment for module '{id}'"))?
                    as i16;
            }
            let mut seen = BTreeSet::new();
            if assignments.iter().any(|cc| *cc >= 0 && !seen.insert(*cc)) {
                return Err(format!(
                    "colliding live MIDI CC assignments for module '{id}'"
                ));
            }
            for (slot, value) in values.iter_mut().zip(controllers) {
                *slot = value
                    .as_u64()
                    .filter(|v| *v <= 127)
                    .ok_or_else(|| format!("invalid MIDI CC value for module '{id}'"))?
                    as u8;
            }
            SavedModuleState::MidiCc {
                assignments,
                values,
            }
        }
        SavedModuleState::StockQuantizerNotes(_) => {
            if module["kind"].as_str() != Some("stock_quantizer") {
                return Err(format!(
                    "live Stock Quantizer kind unavailable for module '{id}'"
                ));
            }
            let mask = module["state"]["scale_mask"]
                .as_u64()
                .filter(|mask| *mask <= 4095)
                .ok_or_else(|| format!("invalid live Stock Quantizer mask for module '{id}'"))?
                as u16;
            let notes = module["state"]["enabled_notes"]
                .as_array()
                .filter(|notes| notes.len() == 12)
                .ok_or_else(|| {
                    format!("live Stock Quantizer notes unavailable for module '{id}'")
                })?;
            for (index, note) in notes.iter().enumerate() {
                if note.as_bool() != Some(mask & (1 << index) != 0) {
                    return Err(format!(
                        "incoherent live Stock Quantizer notes for module '{id}'"
                    ));
                }
            }
            SavedModuleState::StockQuantizerNotes(mask)
        }
        SavedModuleState::StockSeq3 { .. } => {
            let state = &module["state"];
            let running = state["running"]
                .as_bool()
                .ok_or_else(|| format!("live Stock SEQ3 Run unavailable for module '{id}'"))?;
            let passthrough = state["passthrough"].as_bool().ok_or_else(|| {
                format!("live Stock SEQ3 passthrough unavailable for module '{id}'")
            })?;
            let flags = state["gates"]
                .as_array()
                .filter(|flags| flags.len() == 8)
                .ok_or_else(|| format!("live Stock SEQ3 gates unavailable for module '{id}'"))?;
            let mut gates = 0_u8;
            for (index, flag) in flags.iter().enumerate() {
                if flag
                    .as_bool()
                    .ok_or_else(|| format!("invalid live Stock SEQ3 gate for module '{id}'"))?
                {
                    gates |= 1 << index;
                }
            }
            SavedModuleState::StockSeq3 {
                running,
                gates,
                passthrough,
            }
        }
        SavedModuleState::ShiftRegister { .. } => {
            let rows = module["state"]["values"]
                .as_array()
                .filter(|rows| rows.len() == 8)
                .ok_or_else(|| {
                    format!("live Shift Register state unavailable for module '{id}'")
                })?;
            let mut values = [0_u32; 8];
            for (slot, value) in values.iter_mut().zip(rows) {
                let value = value
                    .as_f64()
                    .filter(|value| value.is_finite() && (-1e6..=1e6).contains(value))
                    .ok_or_else(|| format!("invalid Shift Register voltage for module '{id}'"))?
                    as f32;
                *slot = value.to_bits();
            }
            let rng = module["state"]["rng_state"]
                .as_u64()
                .and_then(|rng| u32::try_from(rng).ok())
                .filter(|rng| *rng != 0)
                .ok_or_else(|| format!("invalid Shift Register RNG for module '{id}'"))?;
            SavedModuleState::ShiftRegister { values, rng }
        }
        SavedModuleState::RandomValues { .. } => {
            let rows = module["state"]["values"]
                .as_array()
                .filter(|rows| rows.len() == 7)
                .ok_or_else(|| format!("live Random Values state unavailable for module '{id}'"))?;
            let mut values = [[0_u32; 16]; 7];
            for (output, row) in rows.iter().enumerate() {
                let row = row
                    .as_array()
                    .filter(|row| row.len() == 16)
                    .ok_or_else(|| format!("invalid Random Values voices for module '{id}'"))?;
                for (channel, value) in row.iter().enumerate() {
                    let value = value
                        .as_f64()
                        .filter(|value| value.is_finite() && (-10.0..=10.0).contains(value))
                        .ok_or_else(|| format!("invalid Random Values voltage for module '{id}'"))?
                        as f32;
                    values[output][channel] = value.to_bits();
                }
            }
            let rng = module["state"]["rng_state"]
                .as_u64()
                .and_then(|rng| u32::try_from(rng).ok())
                .filter(|rng| *rng != 0)
                .ok_or_else(|| format!("invalid Random Values RNG for module '{id}'"))?;
            SavedModuleState::RandomValues { values, rng }
        }
        _ => {
            let (parameter, _) = expected
                .boolean()
                .ok_or("persistent state has no boolean field")?;
            let value = module["state"][parameter]
                .as_bool()
                .ok_or_else(|| format!("live {parameter} state unavailable for module '{id}'"))?;
            expected.with_value(value)
        }
    };
    let frame = module["frame"]
        .as_u64()
        .ok_or_else(|| format!("coherent live frame unavailable for module '{id}'"))?;
    Ok((state, frame))
}

pub(super) fn load_saved(
    backend: &dyn LiveModuleEvents,
    states: &BTreeMap<String, SavedModuleState>,
) {
    backend.load_module_saved_states(states);
}

impl Session {
    fn state_for_edit(
        &self,
        candidate: &Patch,
        id: &str,
        effects: &mut EditEffects,
        history: bool,
    ) -> Result<Option<SavedModuleState>, String> {
        self.check_midi_cc_capture()?;
        let node = candidate
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or("module not found")?;
        let Some(initial) = SavedModuleState::from_node(node) else {
            return Ok(None);
        };
        let value = if let Some(value) = effects.working_saved.get(id) {
            *value
        } else if effects.replaced.contains(id) || !self.execution_disposition().is_runnable() {
            initial
        } else if let Some(backend) = &self.live_module_events {
            let snapshot = effects
                .live_snapshot
                .get_or_insert_with(|| backend.snapshot());
            let (state, frame) = live_saved(snapshot, id, self.revision, initial)?;
            self.check_midi_cc_capture()?;
            if effects.live_frame.is_some_and(|prior| prior != frame) {
                return Err("live edit state has mixed callback frames; retry edit".into());
            }
            effects.live_frame = Some(frame);
            state
        } else {
            initial
        };
        // A preceding mapping edit in this transaction changes ownership only.
        // Overlay that candidate map on the captured live controller bank.
        let value = match (value, initial) {
            (SavedModuleState::MidiGateMappings(_), SavedModuleState::MidiGateMappings(maps)) => {
                SavedModuleState::MidiGateMappings(maps)
            }
            (
                SavedModuleState::MidiCc { values, .. },
                SavedModuleState::MidiCc { assignments, .. },
            ) => SavedModuleState::MidiCc {
                assignments,
                values,
            },
            _ => value,
        };
        effects.working_saved.insert(id.to_owned(), value);
        if history && !effects.replaced.contains(id) {
            if let (
                Some(SavedModuleState::MidiCcMappings(assignments)),
                SavedModuleState::MidiCc { values, .. },
            ) = (effects.before_saved.get(id).copied(), value)
            {
                effects.before_saved.insert(
                    id.to_owned(),
                    SavedModuleState::MidiCc {
                        assignments,
                        values,
                    },
                );
            } else {
                effects.before_saved.entry(id.to_owned()).or_insert(value);
            }
        }
        Ok(Some(value))
    }

    pub(super) fn apply_session_edit(
        &self,
        candidate: &mut Patch,
        name: &str,
        args: &Value,
        effects: &mut EditEffects,
    ) -> Result<(), String> {
        if name == "apply_module_preset" {
            if let Some(filename) = args.get("filename") {
                if args.get("preset").is_some() {
                    return Err("Supply either preset or filename, not both".into());
                }
                let filename = filename.as_str().ok_or("filename must be a string")?;
                let expected = string(args, "expected_file_sha256")?;
                let (preset, digest) = crate::presets::read_with_digest(
                    &self.file(filename, crate::presets::EXTENSION)?,
                )?;
                if expected != digest {
                    return Err("Preset file changed since review; read it again".into());
                }
                let mut normalized = args.clone();
                normalized.as_object_mut().unwrap().remove("filename");
                normalized
                    .as_object_mut()
                    .unwrap()
                    .remove("expected_file_sha256");
                normalized["preset"] = preset;
                return self.apply_session_edit(candidate, name, &normalized, effects);
            }
            if args.get("preset").is_none() || args.get("expected_file_sha256").is_some() {
                return Err("Supply preset, or filename with expected_file_sha256".into());
            }
        }
        self.check_source_duplicate_routes(candidate, name, args)?;
        // Observe successful topology operations, not arbitrary before/after
        // differences. A removed/recreated ID never regains continuity within
        // this transaction, even when a later explicit move succeeds.
        let topology = matches!(
            name,
            "connect"
                | "disconnect"
                | "disconnect_cables"
                | "move_cables"
                | "remove_module"
                | "clear_cables"
                | "duplicate_modules"
        );
        let before: BTreeMap<_, _> = if topology {
            candidate
                .cables
                .iter()
                .filter_map(|cable| {
                    Some((cable.id.clone()?, construction::CableRoute::from(cable)))
                })
                .collect()
        } else {
            BTreeMap::new()
        };
        let lifecycle_before = host_lifecycle::may_change_topology(name)
            .then(|| host_lifecycle::Topology::capture(candidate));
        let retired_before = effects.retired.len();
        let parameter_targets =
            super::parameters::before_edit(self, candidate, name, args, effects)?;
        self.apply_session_edit_inner(candidate, name, args, effects)?;
        super::parameters::after_edit(self, candidate, &parameter_targets, effects);
        effects.lifecycle_captured = true;
        if let Some(before) = lifecycle_before {
            let after = host_lifecycle::Topology::capture(candidate);
            let retired = &effects.retired[retired_before..];
            effects
                .forward_lifecycle
                .append(&before.transition(&after, retired));
            effects.reverse_lifecycle = after
                .transition(&before, retired)
                .then(std::mem::take(&mut effects.reverse_lifecycle));
        }
        if topology {
            let after: BTreeMap<_, _> = candidate
                .cables
                .iter()
                .filter_map(|cable| {
                    Some((cable.id.clone()?, construction::CableRoute::from(cable)))
                })
                .collect();
            for id in before.keys().chain(after.keys()) {
                match (before.get(id), after.get(id)) {
                    (Some(old), Some(new)) if old != new && name == "move_cables" => {
                        if !effects.discontinuous_cables.contains(id) {
                            let nodes = effects.cable_moves.entry(id.clone()).or_default();
                            nodes.extend([
                                old.from.clone(),
                                old.to.clone(),
                                new.from.clone(),
                                new.to.clone(),
                            ]);
                        }
                    }
                    (Some(old), Some(new)) if old == new => {}
                    _ => {
                        effects.discontinuous_cables.insert(id.clone());
                        effects.cable_moves.remove(id);
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_session_edit_inner(
        &self,
        candidate: &mut Patch,
        name: &str,
        args: &Value,
        effects: &mut EditEffects,
    ) -> Result<(), String> {
        if name == "set_parameter"
            && candidate.nodes.iter().any(|node| {
                node.id == args["id"].as_str().unwrap_or("") && node.kind == "midi_gate"
            })
        {
            let id = string(args, "id")?.to_owned();
            let parameter = string(args, "parameter")?;
            let before = SavedModuleState::from_node(
                candidate.nodes.iter().find(|node| node.id == id).unwrap(),
            )
            .ok_or("invalid MIDI Gate state")?;
            apply_edit(
                candidate,
                name,
                args,
                &mut effects.retired_cables,
                &mut effects.initialized,
            )?;
            let node = candidate
                .nodes
                .iter_mut()
                .find(|node| node.id == id)
                .unwrap();
            let after = SavedModuleState::from_node(node).ok_or("invalid MIDI Gate state")?;
            if midi_cc::mapping_cell_for("midi_gate", parameter).is_some() {
                after.write_params(&mut node.params);
                effects.before_saved.entry(id.clone()).or_insert(before);
                effects.after_saved.insert(id.clone(), after);
                effects.working_saved.insert(id.clone(), after);
                effects.load_saved.insert(id, after);
            }
            return Ok(());
        }
        let midi_id = args["id"].as_str().filter(|id| {
            candidate
                .nodes
                .iter()
                .any(|node| node.id == *id && node.kind == "midi_cc")
        });
        if let Some(id) = midi_id.filter(|_| name == "set_parameter") {
            let id = id.to_owned();
            let parameter = string(args, "parameter")?;
            let node = candidate.nodes.iter().find(|node| node.id == id).unwrap();
            let initial = SavedModuleState::from_node(node).ok_or("invalid MIDI CC state")?;
            let full = parameter.starts_with("cc_value");
            let previous = if full {
                self.state_for_edit(candidate, &id, effects, true)?.unwrap()
            } else {
                initial
            };
            apply_edit(
                candidate,
                name,
                args,
                &mut effects.retired_cables,
                &mut effects.initialized,
            )?;
            let node = candidate
                .nodes
                .iter_mut()
                .find(|node| node.id == id)
                .unwrap();
            let current = SavedModuleState::from_node(node).ok_or("invalid MIDI CC state")?;
            if full {
                let SavedModuleState::MidiCc {
                    assignments,
                    mut values,
                } = previous
                else {
                    return Err("invalid MIDI CC state".into());
                };
                let index = parameter
                    .strip_prefix("cc_value")
                    .and_then(|s| s.parse::<usize>().ok())
                    .filter(|i| *i < 128)
                    .ok_or("invalid MIDI CC value parameter")?;
                let value = args["value"]
                    .as_f64()
                    .filter(|v| v.is_finite() && v.fract() == 0.0 && (0.0..=127.0).contains(v))
                    .ok_or("invalid MIDI CC controller value")? as u8;
                values[index] = value;
                let state = SavedModuleState::MidiCc {
                    assignments,
                    values,
                };
                state.write_params(&mut node.params);
                effects.full_midi_restores.insert(id.clone());
                effects.working_saved.insert(id.clone(), state);
                effects.after_saved.insert(id.clone(), state);
                effects.load_saved.insert(id, state);
            } else if midi_cc::mapping_cell(parameter).is_some() {
                let SavedModuleState::MidiCc {
                    assignments: before,
                    ..
                } = previous
                else {
                    return Err("invalid MIDI CC state".into());
                };
                let SavedModuleState::MidiCc {
                    assignments: after, ..
                } = current
                else {
                    return Err("invalid MIDI CC state".into());
                };
                effects
                    .before_saved
                    .entry(id.clone())
                    .or_insert(SavedModuleState::MidiCcMappings(before));
                let state = if let Some(SavedModuleState::MidiCc { values, .. }) =
                    effects.working_saved.get(&id).copied()
                {
                    let state = SavedModuleState::MidiCc {
                        assignments: after,
                        values,
                    };
                    effects.working_saved.insert(id.clone(), state);
                    state
                } else {
                    SavedModuleState::MidiCcMappings(after)
                };
                let state = if effects.full_midi_restores.contains(&id) {
                    state
                } else {
                    SavedModuleState::MidiCcMappings(after)
                };
                effects.after_saved.insert(id.clone(), state);
                // A mapping edit itself must never restore controller values.
                if matches!(
                    effects.load_saved.get(&id),
                    Some(SavedModuleState::MidiCc { .. })
                ) {
                    effects.load_saved.insert(id, state);
                } else {
                    effects
                        .load_saved
                        .insert(id, SavedModuleState::MidiCcMappings(after));
                }
            }
            return Ok(());
        }
        let prior_seed = candidate
            .nodes
            .iter()
            .find(|node| node.id == args["id"].as_str().unwrap_or(""))
            .and_then(|node| node.params.get("seed"))
            .copied()
            .unwrap_or(1.0);
        let targets: Vec<String> = match name {
            "initialize_modules" | "randomize_modules" => parameter_action_ids(candidate, args)?
                .into_iter()
                .filter(|id| {
                    name != "randomize_modules"
                        || !candidate.nodes.iter().any(|node| {
                            node.id == *id
                                && matches!(node.kind.as_str(), "sh_asr" | "midi_cc" | "midi_gate")
                        })
                })
                .collect(),
            "apply_module_preset" | "remove_module" | "rotate_scale" => {
                vec![string(args, "id")?.to_owned()]
            }
            "rotate_sequence"
                if candidate.nodes.iter().any(|node| {
                    node.id == args["id"].as_str().unwrap_or("") && node.kind == "stock_seq3"
                }) =>
            {
                vec![string(args, "id")?.to_owned()]
            }
            "set_parameter"
                if candidate.nodes.iter().any(|node| {
                    node.id == args["id"].as_str().unwrap_or("")
                        && (matches!(node.kind.as_str(), "random_values" | "sh_asr")
                            || (node.kind == "stock_seq3"
                                && args["parameter"]
                                    .as_str()
                                    .is_some_and(stock_seq3_saved_parameter))
                            || (node.kind == "stock_quantizer"
                                && args["parameter"] == "scale_mask"))
                }) || args["parameter"] == "running"
                    || args["parameter"] == "holding"
                    || args["parameter"] == "seed"
                    || args["parameter"]
                        .as_str()
                        .is_some_and(|p| p.starts_with("value") || p.starts_with("rng_")) =>
            {
                vec![string(args, "id")?.to_owned()]
            }
            _ => Vec::new(),
        };
        // A continuing keyboard paint gesture owns this complete twelve-bit
        // bank. Its last accepted assignment is already in history even when
        // the callback has not published that revision. Reuse only that exact
        // acknowledged bank; new gestures, partial edits and rotations still
        // require the ordinary coherent live capture.
        if name == "set_parameter" && args["parameter"] == "scale_mask" {
            if let Some((group, id, parameter)) = &self.history_group {
                if parameter == "scale_mask"
                    && args["id"] == id.as_str()
                    && args["history_group"] == group.as_str()
                {
                    if let Some(state @ SavedModuleState::StockQuantizerNotes(_)) = self
                        .undo
                        .back()
                        .and_then(|entry| entry.return_saved.get(id))
                        .copied()
                    {
                        if candidate
                            .nodes
                            .iter()
                            .find(|node| node.id == *id)
                            .and_then(SavedModuleState::from_node)
                            == Some(state)
                        {
                            effects.working_saved.entry(id.clone()).or_insert(state);
                        }
                    }
                }
            }
        }
        for id in &targets {
            let state = self.state_for_edit(candidate, id, effects, true)?;
            // Project one coherent bank before editing a Stock SEQ3 durable
            // field. Rotation, partial presets and randomization must not use
            // stale gate/Run/passthrough parameters from the original graph.
            if let Some(
                state @ (SavedModuleState::StockSeq3 { .. }
                | SavedModuleState::StockQuantizerNotes(_)),
            ) = state
            {
                let node = candidate
                    .nodes
                    .iter_mut()
                    .find(|node| node.id == *id)
                    .unwrap();
                state.write_params(&mut node.params);
            }
        }
        // Intermediate duplicate IDs are legal until final graph validation.
        // If a later removal deletes both copies, undo must still restore the
        // original instance's actual custom state, not its initial params.
        if name == "add_module" {
            let id = string(args, "id")?;
            if candidate.nodes.iter().any(|node| node.id == id) {
                self.state_for_edit(candidate, id, effects, true)?;
            }
            effects.placements.insert(
                id.to_owned(),
                layout_history::requested_row(args)?.unwrap_or(0),
            );
        }
        let mut copies = Vec::new();
        if name == "duplicate_modules" {
            let mappings = args["modules"]
                .as_array()
                .ok_or("modules must be an array")?;
            for mapping in mappings {
                let id = string(mapping, "id")?;
                let original = effects.source_copies.get(id).cloned().or_else(|| {
                    self.patch
                        .nodes
                        .iter()
                        .any(|node| node.id == id)
                        .then(|| id.to_owned())
                        .filter(|id| !effects.retired.contains(id))
                });
                if let Some(original) = original {
                    effects
                        .source_copies
                        .insert(string(mapping, "new_id")?.to_owned(), original);
                }
                if let Some(value) = self.state_for_edit(candidate, id, effects, false)? {
                    copies.push((string(mapping, "new_id")?.to_owned(), value));
                }
            }
        }
        if name == "rotate_sequence" {
            rotate_sequence(candidate, args)?;
        } else if name == "rotate_scale" {
            rotate_scale(candidate, args)?;
        } else {
            apply_edit(
                candidate,
                name,
                args,
                &mut effects.retired_cables,
                &mut effects.initialized,
            )?;
        }
        if name == "clear_cables" {
            effects.cleared_source_routes = true;
        }
        self.assign_source_safe_cable_ids(candidate, name, args)?;
        if name == "remove_module" {
            let id = string(args, "id")?;
            effects.retired.push(id.to_owned());
            effects.replaced.insert(id.to_owned());
            effects.placements.remove(id);
            effects.working_saved.remove(id);
            effects.after_saved.remove(id);
            effects.load_saved.remove(id);
            effects.release_controls.remove(id);
        } else {
            for id in targets {
                if let Some(value) = effects.working_saved.get(&id) {
                    effects.after_saved.insert(id.clone(), *value);
                }
                let current = candidate
                    .nodes
                    .iter()
                    .find(|node| node.id == id)
                    .and_then(SavedModuleState::from_node);
                if let Some(current) = current {
                    if let SavedModuleState::StockSeq3 {
                        mut running,
                        gates,
                        passthrough,
                    } = current
                    {
                        if name == "initialize_modules" {
                            let Some(SavedModuleState::StockSeq3 {
                                running: previous, ..
                            }) = effects.working_saved.get(&id)
                            else {
                                return Err("Stock SEQ3 state kind mismatch".into());
                            };
                            running = *previous;
                        }
                        let full = SavedModuleState::StockSeq3 {
                            running,
                            gates,
                            passthrough,
                        };
                        full.write_params(
                            &mut candidate
                                .nodes
                                .iter_mut()
                                .find(|node| node.id == id)
                                .unwrap()
                                .params,
                        );
                        effects.working_saved.insert(id.clone(), full);
                        effects.after_saved.insert(id.clone(), full);
                        // Loading only explicitly edited flags after adoption
                        // preserves CV/button edges that arrive after capture.
                        let requested = |parameter: &str| {
                            name == "set_parameter" && args["parameter"] == parameter
                                || name == "apply_module_preset"
                                    && args["preset"]["params"].get(parameter).is_some()
                        };
                        let gate_mask = if matches!(
                            name,
                            "rotate_sequence" | "randomize_modules" | "initialize_modules"
                        ) {
                            255
                        } else {
                            (0..8).fold(0_u8, |mask, i| {
                                mask | if requested(&format!("gate{}", i + 1)) {
                                    1 << i
                                } else {
                                    0
                                }
                            })
                        };
                        let assignment = SavedModuleState::StockSeq3Fields {
                            running: requested("running").then_some(running),
                            gates,
                            gate_mask,
                            passthrough: (requested("passthrough") || name == "initialize_modules")
                                .then_some(passthrough),
                        };
                        if let SavedModuleState::StockSeq3Fields {
                            running,
                            gate_mask,
                            passthrough,
                            ..
                        } = assignment
                        {
                            if running.is_some() || gate_mask != 0 || passthrough.is_some() {
                                let combined = merge_stock_seq3_assignments(
                                    effects.load_saved.get(&id).copied(),
                                    assignment,
                                )?;
                                effects.load_saved.insert(id.clone(), combined);
                            }
                        }
                        if name == "apply_module_preset" {
                            effects.release_controls.insert(id);
                        }
                        continue;
                    }
                    let explicit = if current.kind() == "stock_quantizer" {
                        (matches!(
                            name,
                            "rotate_scale" | "randomize_modules" | "initialize_modules"
                        ) || name == "set_parameter" && args["parameter"] == "scale_mask"
                            || name == "apply_module_preset"
                                && args["preset"]["params"].get("scale_mask").is_some())
                        .then_some(current)
                    } else if current.kind() == "midi_gate" {
                        if matches!(name, "initialize_modules" | "apply_module_preset") {
                            Some(current)
                        } else {
                            None
                        }
                    } else if current.kind() == "midi_cc" {
                        if name == "initialize_modules" {
                            effects.full_midi_restores.insert(id.clone());
                            Some(current)
                        } else if name == "apply_module_preset" {
                            let full = args["preset"]["params"].as_object().is_some_and(|params| {
                                params.keys().any(|key| key.starts_with("cc_value"))
                            });
                            if full {
                                let SavedModuleState::MidiCc { assignments, .. } = current else {
                                    return Err("invalid MIDI CC state".into());
                                };
                                let Some(SavedModuleState::MidiCc { mut values, .. }) =
                                    effects.working_saved.get(&id).copied()
                                else {
                                    return Err("invalid MIDI CC state".into());
                                };
                                for (index, value) in values.iter_mut().enumerate() {
                                    if let Some(saved) =
                                        args["preset"]["params"].get(format!("cc_value{index}"))
                                    {
                                        *value = saved
                                            .as_f64()
                                            .ok_or("invalid MIDI CC controller value")?
                                            as u8;
                                    }
                                }
                                effects.full_midi_restores.insert(id.clone());
                                Some(SavedModuleState::MidiCc {
                                    assignments,
                                    values,
                                })
                            } else if effects.full_midi_restores.contains(&id) {
                                let SavedModuleState::MidiCc { assignments, .. } = current else {
                                    return Err("invalid MIDI CC state".into());
                                };
                                let Some(SavedModuleState::MidiCc { values, .. }) =
                                    effects.working_saved.get(&id).copied()
                                else {
                                    return Err("invalid MIDI CC state".into());
                                };
                                Some(SavedModuleState::MidiCc {
                                    assignments,
                                    values,
                                })
                            } else {
                                let SavedModuleState::MidiCc { assignments, .. } = current else {
                                    return Err("invalid MIDI CC state".into());
                                };
                                if let Some(SavedModuleState::MidiCc {
                                    assignments: before,
                                    ..
                                }) = effects.before_saved.get(&id).copied()
                                {
                                    effects.before_saved.insert(
                                        id.clone(),
                                        SavedModuleState::MidiCcMappings(before),
                                    );
                                }
                                Some(SavedModuleState::MidiCcMappings(assignments))
                            }
                        } else {
                            None
                        }
                    } else if current.kind() == "sh_asr" {
                        let previous = effects.working_saved.get(&id).copied().unwrap_or(current);
                        let snapshot = (name == "apply_module_preset"
                            && args["preset"]["params"].get("rng_hi").is_some())
                            || (name == "set_parameter"
                                && args["parameter"].as_str().is_some_and(|name| {
                                    name.starts_with("value") || name.starts_with("rng_")
                                }));
                        if name == "initialize_modules" {
                            let mut fresh = candidate
                                .nodes
                                .iter()
                                .find(|node| node.id == id)
                                .unwrap()
                                .clone();
                            fresh.params.retain(|name, _| {
                                !name.starts_with("value") && !name.starts_with("rng_")
                            });
                            Some(openpatch_core::initial_shift_register_state(&fresh)?)
                        } else if snapshot {
                            Some(current)
                        } else if (name == "set_parameter" && args["parameter"] == "seed")
                            || (name == "apply_module_preset"
                                && args["preset"]["params"].get("seed").is_some())
                        {
                            let node = candidate.nodes.iter().find(|node| node.id == id).unwrap();
                            if node.params.get("seed").copied().unwrap_or(1.0) != prior_seed {
                                let mut fresh = node.clone();
                                fresh.params.retain(|name, _| {
                                    !name.starts_with("value") && !name.starts_with("rng_")
                                });
                                let SavedModuleState::ShiftRegister { rng, .. } =
                                    openpatch_core::initial_shift_register_state(&fresh)?
                                else {
                                    return Err("Shift Register state kind mismatch".into());
                                };
                                let SavedModuleState::ShiftRegister { values, .. } = previous
                                else {
                                    return Err("Shift Register state kind mismatch".into());
                                };
                                Some(SavedModuleState::ShiftRegister { values, rng })
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else if current.kind() == "random_values" {
                        let previous = effects.working_saved.get(&id).copied().unwrap_or(current);
                        if name == "initialize_modules" {
                            Some(previous)
                        } else if name == "randomize_modules" {
                            let node = candidate.nodes.iter().find(|node| node.id == id).unwrap();
                            let seed = args["seed"]
                                .as_u64()
                                .and_then(|seed| u32::try_from(seed).ok())
                                .ok_or("seed must be a u32")?;
                            let SavedModuleState::RandomValues { values, .. } = previous else {
                                return Err("Random Values state kind mismatch".into());
                            };
                            let seeded = SavedModuleState::RandomValues {
                                values,
                                rng: module_random_seed(&id, seed).max(1),
                            };
                            Some(openpatch_core::randomize_random_values_state(node, seeded)?)
                        } else if (name == "apply_module_preset"
                            && args["preset"]["params"].get("rng_hi").is_some())
                            || (name == "set_parameter"
                                && args["parameter"].as_str().is_some_and(|name| {
                                    name.starts_with("value") || name.starts_with("rng_")
                                }))
                        {
                            Some(current)
                        } else if name == "set_parameter" && args["parameter"] == "seed" {
                            let node = candidate.nodes.iter().find(|node| node.id == id).unwrap();
                            if node.params.get("seed").copied().unwrap_or(1.0) != prior_seed {
                                let mut fresh = node.clone();
                                fresh.params.retain(|name, _| {
                                    !name.starts_with("value") && !name.starts_with("rng_")
                                });
                                Some(openpatch_core::initial_random_values_state(&fresh)?)
                            } else {
                                None
                            }
                        } else if name == "apply_module_preset"
                            && args["preset"]["params"].get("seed").is_some()
                        {
                            let node = candidate.nodes.iter().find(|node| node.id == id).unwrap();
                            if node.params.get("seed").copied().unwrap_or(1.0) != prior_seed {
                                let mut fresh = node.clone();
                                fresh.params.retain(|name, _| {
                                    !name.starts_with("value") && !name.starts_with("rng_")
                                });
                                Some(openpatch_core::initial_random_values_state(&fresh)?)
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        let explicit = if name == "apply_module_preset" {
                            args["preset"]["params"]
                                .get(current.parameter())
                                .and_then(Value::as_f64)
                        } else if name == "set_parameter"
                            && args["parameter"] == current.parameter()
                        {
                            args.get("value").and_then(Value::as_f64)
                        } else if name == "initialize_modules" && current.kind() == "push" {
                            Some(0.0)
                        } else {
                            None
                        };
                        explicit.map(|value| current.with_value(value as f32 == 1.0))
                    };
                    if let Some(value) = explicit {
                        if matches!(
                            value.kind(),
                            "random_values"
                                | "sh_asr"
                                | "midi_cc"
                                | "midi_gate"
                                | "stock_seq3"
                                | "stock_quantizer"
                        ) {
                            let node = candidate
                                .nodes
                                .iter_mut()
                                .find(|node| node.id == id)
                                .unwrap();
                            value.write_params(&mut node.params);
                        }
                        let working = if let SavedModuleState::MidiCcMappings(assignments) = value {
                            if let Some(SavedModuleState::MidiCc { values, .. }) =
                                effects.working_saved.get(&id).copied()
                            {
                                SavedModuleState::MidiCc {
                                    assignments,
                                    values,
                                }
                            } else {
                                value
                            }
                        } else {
                            value
                        };
                        effects.working_saved.insert(id.clone(), working);
                        effects.after_saved.insert(id.clone(), value);
                        effects.load_saved.insert(id.clone(), value);
                    }
                }
                if name == "apply_module_preset" {
                    effects.release_controls.insert(id);
                }
            }
        }
        if name == "add_module" {
            let id = string(args, "id")?;
            effects.replaced.insert(id.to_owned());
            let node = candidate.nodes.iter().find(|node| node.id == id).unwrap();
            if let Some(value) = SavedModuleState::from_node(node) {
                copies.push((id.to_owned(), value));
            }
        }
        for (id, value) in copies {
            let node = candidate
                .nodes
                .iter_mut()
                .find(|node| node.id == id)
                .unwrap();
            if name == "duplicate_modules" {
                value.write_params(&mut node.params);
            }
            effects.replaced.insert(id.clone());
            effects.working_saved.insert(id.clone(), value);
            effects.after_saved.insert(id.clone(), value);
            // Fresh instances already receive their initial saved state via params.
            // The snapshot is retained so redo restores it after later actions.
        }
        Ok(())
    }
}

fn stock_seq3_saved_parameter(name: &str) -> bool {
    matches!(name, "running" | "passthrough")
        || name
            .strip_prefix("gate")
            .and_then(|value| value.parse::<u8>().ok())
            .is_some_and(|value| (1..=8).contains(&value))
}

fn merge_stock_seq3_assignments(
    previous: Option<SavedModuleState>,
    next: SavedModuleState,
) -> Result<SavedModuleState, String> {
    let SavedModuleState::StockSeq3Fields {
        running,
        gates,
        gate_mask,
        passthrough,
    } = next
    else {
        return Err("expected Stock SEQ3 field assignment".into());
    };
    let (old_run, old_gates, old_mask, old_pass) = match previous {
        None => (None, 0, 0, None),
        Some(SavedModuleState::StockSeq3Fields {
            running,
            gates,
            gate_mask,
            passthrough,
        }) => (running, gates, gate_mask, passthrough),
        Some(SavedModuleState::StockSeq3 {
            running,
            gates,
            passthrough,
        }) => (Some(running), gates, 255, Some(passthrough)),
        _ => return Err("Stock SEQ3 pending assignment kind mismatch".into()),
    };
    Ok(SavedModuleState::StockSeq3Fields {
        running: running.or(old_run),
        gates: (old_gates & !gate_mask) | (gates & gate_mask),
        gate_mask: old_mask | gate_mask,
        passthrough: passthrough.or(old_pass),
    })
}
