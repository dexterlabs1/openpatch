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
    let value = module["state"][expected.parameter()]
        .as_bool()
        .ok_or_else(|| {
            format!(
                "live {} state unavailable for module '{id}'",
                if expected.kind() == "seq3" {
                    "Run"
                } else {
                    "HOLD"
                }
            )
        })?;
    let frame = module["frame"]
        .as_u64()
        .ok_or_else(|| format!("coherent live frame unavailable for module '{id}'"))?;
    Ok((expected.with_value(value), frame))
}

pub(super) fn load_saved(
    backend: &dyn LiveModuleEvents,
    states: &BTreeMap<String, SavedModuleState>,
) {
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
        }
    }
    if !running.is_empty() {
        backend.load_module_running(&running);
    }
    if !holding.is_empty() {
        backend.load_module_holding(&holding);
    }
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
        let targets: Vec<String> = match name {
            "initialize_modules" | "randomize_modules" => {
                parameter_action_ids(candidate, args)?.into_iter().collect()
            }
            "apply_module_preset" | "remove_module" => vec![string(args, "id")?.to_owned()],
            "set_parameter" if args["parameter"] == "running" || args["parameter"] == "holding" => {
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
                    let explicit = if name == "apply_module_preset" {
                        args["preset"]["params"]
                            .get(current.parameter())
                            .and_then(Value::as_f64)
                    } else if name == "set_parameter" && args["parameter"] == current.parameter() {
                        args.get("value").and_then(Value::as_f64)
                    } else if name == "initialize_modules" && current.kind() == "push" {
                        Some(0.0)
                    } else {
                        None
                    };
                    if let Some(value) = explicit {
                        let value = current.with_value(value as f32 == 1.0);
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
                node.params.insert(
                    value.parameter().into(),
                    if value.value() { 1.0 } else { 0.0 },
                );
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
