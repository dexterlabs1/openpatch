//! Authored parameter assignment intent and effective-value undo checkpoints.
//! Automation never writes these maps or creates a Session transaction.
use super::*;
use openpatch_core::{runtime_parameter_descriptor, runtime_parameter_names};

pub(super) type ParameterValues = BTreeMap<String, BTreeMap<String, f32>>;
pub(super) type Targets = BTreeMap<String, BTreeSet<String>>;

#[derive(Clone, Default)]
pub(super) struct ParameterEffects {
    pub before: ParameterValues,
    pub after: ParameterValues,
    working: ParameterValues,
}

fn authored(node: &Node, key: &str) -> f32 {
    node.params.get(key).copied().unwrap_or_else(|| {
        runtime_parameter_descriptor(&node.kind, key)
            .expect("native capability")
            .default
    })
}

fn targets(patch: &Patch, name: &str, args: &Value) -> Result<Targets, String> {
    let ids = match name {
        "set_parameter" | "apply_module_preset" | "remove_module" => {
            vec![string(args, "id")?.to_owned()]
        }
        "initialize_modules" | "randomize_modules" => {
            parameter_action_ids(patch, args)?.into_iter().collect()
        }
        _ => return Ok(Targets::new()),
    };
    let mut targets = Targets::new();
    for id in ids {
        let Some(node) = patch.nodes.iter().find(|node| node.id == id) else {
            continue;
        };
        let keys = runtime_parameter_names(&node.kind)
            .iter()
            .copied()
            .filter(|key| match name {
                "set_parameter" => args["parameter"].as_str() == Some(*key),
                "apply_module_preset" => args["preset"]["params"].get(*key).is_some(),
                _ => true,
            })
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        if !keys.is_empty() {
            targets.insert(id, keys);
        }
    }
    Ok(targets)
}

fn observe(
    session: &Session,
    node: &Node,
    key: &str,
    snapshot: Option<&Value>,
    frame: &mut Option<u64>,
) -> Result<f32, String> {
    let Some(snapshot) = snapshot else {
        return Ok(authored(node, key));
    };
    let module = &snapshot["modules"][&node.id];
    if snapshot["coherent"].as_bool() == Some(false)
        || module["available"] != true
        || module["revision"].as_u64() != Some(session.revision)
        || module["kind"].as_str() != Some(node.kind.as_str())
    {
        return Err(format!(
            "live parameter history unavailable for '{}'; wait for revision {}",
            node.id, session.revision
        ));
    }
    let seen = module["frame"]
        .as_u64()
        .ok_or("live parameter history frame unavailable")?;
    if frame.is_some_and(|old| old != seen) {
        return Err("live parameter history has mixed callback frames; retry edit".into());
    }
    *frame = Some(seen);
    let value = module["parameters"][key]
        .as_f64()
        .ok_or_else(|| format!("live parameter '{key}' unavailable for '{}'", node.id))?
        as f32;
    let descriptor = runtime_parameter_descriptor(&node.kind, key).expect("native capability");
    if !value.is_finite()
        || value < descriptor.min
        || value > descriptor.max
        || (descriptor.snap && value.fract() != 0.0)
    {
        return Err(format!("invalid live parameter '{key}' for '{}'", node.id));
    }
    Ok(value)
}

fn needs_live(session: &Session) -> bool {
    session.execution_disposition().is_runnable()
        && session
            .live_module_events
            .as_ref()
            .is_some_and(|backend| backend.supports_live_parameters())
}

pub(super) fn before_edit(
    session: &Session,
    candidate: &Patch,
    name: &str,
    args: &Value,
    effects: &mut EditEffects,
) -> Result<Targets, String> {
    let targets = targets(candidate, name, args)?;
    for (id, keys) in &targets {
        let node = candidate
            .nodes
            .iter()
            .find(|node| node.id == *id)
            .expect("selected node");
        let original = session
            .patch
            .nodes
            .iter()
            .find(|old| old.id == *id && old.kind == node.kind);
        let original = original.filter(|_| !effects.retired.contains(id));
        for key in keys {
            if effects
                .parameters
                .working
                .get(id)
                .and_then(|values| values.get(key))
                .is_some()
            {
                continue;
            }
            let value = if let Some(original) = original {
                if needs_live(session) && effects.live_snapshot.is_none() {
                    effects.live_snapshot =
                        Some(session.live_module_events.as_ref().unwrap().snapshot());
                }
                let value = observe(
                    session,
                    original,
                    key,
                    effects
                        .live_snapshot
                        .as_ref()
                        .filter(|_| needs_live(session)),
                    &mut effects.live_frame,
                )?;
                effects
                    .parameters
                    .before
                    .entry(id.clone())
                    .or_default()
                    .entry(key.clone())
                    .or_insert(value);
                value
            } else {
                authored(node, key)
            };
            effects
                .parameters
                .working
                .entry(id.clone())
                .or_default()
                .insert(key.clone(), value);
        }
    }
    Ok(targets)
}

pub(super) fn after_edit(
    session: &Session,
    candidate: &Patch,
    targets: &Targets,
    effects: &mut EditEffects,
) {
    for (id, keys) in targets {
        if let Some(node) = candidate.nodes.iter().find(|node| node.id == *id) {
            for key in keys {
                let value = authored(node, key);
                effects
                    .parameters
                    .after
                    .entry(id.clone())
                    .or_default()
                    .insert(key.clone(), value);
                effects
                    .parameters
                    .working
                    .entry(id.clone())
                    .or_default()
                    .insert(key.clone(), value);
            }
        } else {
            effects.parameters.after.remove(id);
            effects.parameters.working.remove(id);
        }
    }
    // Creation/recreation explicitly assigns its own compiled defaults. A
    // reused ID cannot inherit effective values from the deleted incarnation.
    for node in &candidate.nodes {
        if !session
            .patch
            .nodes
            .iter()
            .any(|old| old.id == node.id && old.kind == node.kind)
            || effects.retired.contains(&node.id)
        {
            for &key in runtime_parameter_names(&node.kind) {
                let value = authored(node, key);
                effects
                    .parameters
                    .after
                    .entry(node.id.clone())
                    .or_default()
                    .insert(key.to_owned(), value);
                effects
                    .parameters
                    .working
                    .entry(node.id.clone())
                    .or_default()
                    .insert(key.to_owned(), value);
            }
        }
    }
}

/// Full loads and imported-document replacement use the ordinary commit path,
/// outside apply_session_edit. Complete their explicit fresh-state coverage.
pub(super) fn for_commit(
    session: &Session,
    candidate: &mut Patch,
    effects: &EditEffects,
) -> Result<ParameterEffects, String> {
    let mut result = effects.parameters.clone();
    let mut captured = None;
    let mut frame = effects.live_frame;
    for node in &session.patch.nodes {
        if !effects.retired.contains(&node.id) {
            continue;
        }
        for &key in runtime_parameter_names(&node.kind) {
            if result
                .before
                .get(&node.id)
                .and_then(|values| values.get(key))
                .is_some()
            {
                continue;
            }
            if needs_live(session) && effects.live_snapshot.is_none() && captured.is_none() {
                captured = Some(session.live_module_events.as_ref().unwrap().snapshot());
            }
            let snapshot = effects
                .live_snapshot
                .as_ref()
                .or(captured.as_ref())
                .filter(|_| needs_live(session));
            let value = observe(session, node, key, snapshot, &mut frame)?;
            result
                .before
                .entry(node.id.clone())
                .or_default()
                .insert(key.to_owned(), value);
        }
    }
    for node in &mut candidate.nodes {
        let fresh = effects.retired.contains(&node.id)
            || !session
                .patch
                .nodes
                .iter()
                .any(|old| old.id == node.id && old.kind == node.kind);
        if fresh {
            for &key in runtime_parameter_names(&node.kind) {
                let value = authored(node, key);
                result
                    .after
                    .entry(node.id.clone())
                    .or_default()
                    .insert(key.to_owned(), value);
            }
        }
        if let Some(values) = result.after.get(&node.id) {
            node.params
                .extend(values.iter().map(|(k, v)| (k.clone(), *v)));
        }
    }
    result.after.retain(|id, values| {
        let Some(node) = candidate.nodes.iter().find(|node| node.id == *id) else {
            return false;
        };
        values.retain(|key, _| runtime_parameter_descriptor(&node.kind, key).is_some());
        !values.is_empty()
    });
    Ok(result)
}

pub(super) fn overlay(patch: &mut Patch, values: &ParameterValues) {
    for node in &mut patch.nodes {
        if let Some(values) = values.get(&node.id) {
            for (key, value) in values {
                if runtime_parameter_descriptor(&node.kind, key).is_some() {
                    node.params.insert(key.clone(), *value);
                }
            }
        }
    }
}

pub(super) fn same_coverage(a: &ParameterValues, b: &ParameterValues) -> bool {
    a.len() == b.len()
        && a.iter().all(|(id, values)| {
            b.get(id)
                .is_some_and(|other| values.keys().eq(other.keys()))
        })
}

#[cfg(test)]
mod tests;
