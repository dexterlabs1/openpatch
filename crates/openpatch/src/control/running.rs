//! Saved SEQ3 custom state follows history independently of initial params.
use super::*;

#[derive(Default)]
pub(super) struct EditEffects {
    pub retired: Vec<String>,
    pub retired_cables: Vec<String>,
    pub initialized: BTreeSet<String>,
    pub release_controls: BTreeSet<String>,
    pub before_running: BTreeMap<String, bool>,
    pub after_running: BTreeMap<String, bool>,
    pub load_running: BTreeMap<String, bool>,
    working_running: BTreeMap<String, bool>,
    replaced: BTreeSet<String>,
    live_snapshot: Option<Value>,
}

pub(super) fn initial_running(node: &Node) -> bool {
    node.params.get("running").copied().unwrap_or(1.0) == 1.0
}

pub(super) fn live_running(
    snapshot: &Value,
    id: &str,
    revision: u64,
) -> Result<(bool, u64), String> {
    let module = &snapshot["modules"][id];
    if module["available"] != true || module["revision"].as_u64() != Some(revision) {
        return Err(format!("live state unavailable for module '{id}'; wait for revision {revision} to be published"));
    }
    let running = module["state"]["running"]
        .as_bool()
        .ok_or_else(|| format!("live Run state unavailable for module '{id}'"))?;
    let frame = module["frame"]
        .as_u64()
        .ok_or_else(|| format!("coherent live frame unavailable for module '{id}'"))?;
    Ok((running, frame))
}

impl Session {
    fn run_for_edit(
        &self,
        candidate: &Patch,
        id: &str,
        effects: &mut EditEffects,
        history: bool,
    ) -> Result<Option<bool>, String> {
        let node = candidate
            .nodes
            .iter()
            .find(|node| node.id == id)
            .ok_or("module not found")?;
        if node.kind != "seq3" {
            return Ok(None);
        }
        let value = if let Some(value) = effects.working_running.get(id) {
            *value
        } else if effects.replaced.contains(id) {
            initial_running(node)
        } else if let Some(backend) = &self.live_module_events {
            let snapshot = effects
                .live_snapshot
                .get_or_insert_with(|| backend.snapshot());
            live_running(snapshot, id, self.revision)?.0
        } else {
            initial_running(node)
        };
        effects.working_running.insert(id.to_owned(), value);
        if history && !effects.replaced.contains(id) {
            effects.before_running.entry(id.to_owned()).or_insert(value);
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
            "set_parameter" if args["parameter"] == "running" => {
                vec![string(args, "id")?.to_owned()]
            }
            _ => Vec::new(),
        };
        for id in &targets {
            self.run_for_edit(candidate, id, effects, true)?;
        }
        // Intermediate duplicate IDs are legal until final graph validation.
        // If a later removal deletes both copies, undo must still restore the
        // original instance's actual custom state, not its initial params.
        if name == "add_module" {
            let id = string(args, "id")?;
            if candidate.nodes.iter().any(|node| node.id == id) {
                self.run_for_edit(candidate, id, effects, true)?;
            }
        }
        let mut copies = Vec::new();
        if name == "duplicate_modules" {
            let mappings = args["modules"]
                .as_array()
                .ok_or("modules must be an array")?;
            for mapping in mappings {
                let id = string(mapping, "id")?;
                if let Some(value) = self.run_for_edit(candidate, id, effects, false)? {
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
            effects.working_running.remove(id);
            effects.after_running.remove(id);
            effects.load_running.remove(id);
            effects.release_controls.remove(id);
        } else {
            for id in targets {
                if let Some(value) = effects.working_running.get(&id) {
                    effects.after_running.insert(id.clone(), *value);
                }
                let explicit = if name == "apply_module_preset" {
                    args["preset"]["params"]
                        .get("running")
                        .and_then(Value::as_f64)
                } else if name == "set_parameter" && args["parameter"] == "running" {
                    args.get("value").and_then(Value::as_f64)
                } else {
                    None
                };
                if let Some(value) = explicit.filter(|_| {
                    candidate
                        .nodes
                        .iter()
                        .any(|node| node.id == id && node.kind == "seq3")
                }) {
                    let value = value as f32 == 1.0;
                    effects.working_running.insert(id.clone(), value);
                    effects.after_running.insert(id.clone(), value);
                    effects.load_running.insert(id.clone(), value);
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
            if node.kind == "seq3" {
                copies.push((id.to_owned(), initial_running(node)));
            }
        }
        for (id, value) in copies {
            let node = candidate
                .nodes
                .iter_mut()
                .find(|node| node.id == id)
                .unwrap();
            if name == "duplicate_modules" {
                node.params
                    .insert("running".into(), if value { 1.0 } else { 0.0 });
            }
            effects.replaced.insert(id.clone());
            effects.working_running.insert(id.clone(), value);
            effects.after_running.insert(id.clone(), value);
            // Fresh instances already receive their initial Run via params.
            // The snapshot is retained so redo restores it after later actions.
        }
        Ok(())
    }
}
