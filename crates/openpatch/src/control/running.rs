//! Typed module state follows history independently of initial parameters.
use super::*;

#[derive(Default)]
pub(super) struct EditEffects {
    pub retired: Vec<String>,
    pub retired_cables: Vec<String>,
    pub initialized: BTreeSet<String>,
    pub release_controls: BTreeSet<String>,
    pub before_saved: BTreeMap<String, SavedModuleState>,
    pub after_saved: BTreeMap<String, SavedModuleState>,
    pub load_saved: BTreeMap<String, SavedModuleState>,
    working_saved: BTreeMap<String, SavedModuleState>,
    replaced: BTreeSet<String>,
    live_snapshot: Option<Value>,
    live_frame: Option<u64>,
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
        } else if effects.replaced.contains(id) {
            initial
        } else if let Some(backend) = &self.live_module_events {
            let snapshot = effects
                .live_snapshot
                .get_or_insert_with(|| backend.snapshot());
            let (state, frame) = live_saved(snapshot, id, self.revision, initial)?;
            if effects.live_frame.is_some_and(|prior| prior != frame) {
                return Err("live edit state has mixed callback frames; retry edit".into());
            }
            effects.live_frame = Some(frame);
            state
        } else {
            initial
        };
        effects.working_saved.insert(id.to_owned(), value);
        if history && !effects.replaced.contains(id) {
            effects.before_saved.entry(id.to_owned()).or_insert(value);
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
        let prior_seed = candidate
            .nodes
            .iter()
            .find(|node| node.id == args["id"].as_str().unwrap_or(""))
            .and_then(|node| node.params.get("seed"))
            .copied()
            .unwrap_or(1.0);
        let targets: Vec<String> = match name {
            "initialize_modules" | "randomize_modules" => {
                parameter_action_ids(candidate, args)?.into_iter().collect()
            }
            "apply_module_preset" | "remove_module" => vec![string(args, "id")?.to_owned()],
            "set_parameter"
                if candidate.nodes.iter().any(|node| {
                    node.id == args["id"].as_str().unwrap_or("") && node.kind == "random_values"
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
        for id in &targets {
            self.state_for_edit(candidate, id, effects, true)?;
        }
        // Intermediate duplicate IDs are legal until final graph validation.
        // If a later removal deletes both copies, undo must still restore the
        // original instance's actual custom state, not its initial params.
        if name == "add_module" {
            let id = string(args, "id")?;
            if candidate.nodes.iter().any(|node| node.id == id) {
                self.state_for_edit(candidate, id, effects, true)?;
            }
        }
        let mut copies = Vec::new();
        if name == "duplicate_modules" {
            let mappings = args["modules"]
                .as_array()
                .ok_or("modules must be an array")?;
            for mapping in mappings {
                let id = string(mapping, "id")?;
                if let Some(value) = self.state_for_edit(candidate, id, effects, false)? {
                    copies.push((string(mapping, "new_id")?.to_owned(), value));
                }
            }
        }
        if name == "rotate_sequence" {
            rotate_sequence(candidate, args)?;
        } else {
            apply_edit(
                candidate,
                name,
                args,
                &mut effects.retired_cables,
                &mut effects.initialized,
            )?;
        }
        if name == "remove_module" {
            let id = string(args, "id")?;
            effects.retired.push(id.to_owned());
            effects.replaced.insert(id.to_owned());
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
                    let explicit = if current.kind() == "random_values" {
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
                        if value.kind() == "random_values" {
                            let node = candidate
                                .nodes
                                .iter_mut()
                                .find(|node| node.id == id)
                                .unwrap();
                            value.write_params(&mut node.params);
                        }
                        effects.working_saved.insert(id.clone(), value);
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
