//! Exact-ID cable edits. All validation and routing preparation run on the
//! control thread; endpoint/color metadata never changes a cable's saved ID.
use super::*;
use openpatch_core::{promote_cable_endpoints, CableEndpoint};

fn endpoint(args: &Value) -> Result<CableEndpoint, String> {
    match string(args, "end")? {
        "input" => Ok(CableEndpoint::Input),
        "output" => Ok(CableEndpoint::Output),
        _ => Err("end must be input or output".into()),
    }
}

fn existing_plug_order(
    candidate: &Patch,
    end: CableEndpoint,
    mut indices: Vec<usize>,
) -> Vec<usize> {
    indices.sort_by_key(|&index| {
        let rank = match end {
            CableEndpoint::Input => candidate.cables[index].input_order,
            CableEndpoint::Output => candidate.cables[index].output_order,
        };
        (rank.unwrap_or(0), index)
    });
    indices
}

pub(super) fn promote(candidate: &mut Patch, args: &Value) -> Result<(), String> {
    let indices = selected_indices(candidate, args)?;
    let end = endpoint(args)?;
    let order = existing_plug_order(candidate, end, indices);
    promote_cable_endpoints(&mut candidate.cables, end, &order)?;
    Ok(())
}

pub(super) fn rgb(value: &Value) -> Result<[u8; 3], String> {
    serde_json::from_value(value.clone())
        .map_err(|_| "color must be an array of exactly 3 integers in 0..255".into())
}

fn selected_indices(candidate: &Patch, args: &Value) -> Result<Vec<usize>, String> {
    let ids = args
        .get("cable_ids")
        .and_then(Value::as_array)
        .ok_or("cable_ids must be an array")?;
    if ids.is_empty() || ids.len() > openpatch_core::MAX_CABLES {
        return Err(format!(
            "cable_ids must contain 1..{} unique existing IDs",
            openpatch_core::MAX_CABLES
        ));
    }
    let mut seen = BTreeSet::new();
    ids.iter()
        .map(|value| {
            let id = value.as_str().ok_or("each cable ID must be a string")?;
            if !seen.insert(id) {
                return Err(format!("duplicate cable ID '{id}'"));
            }
            candidate
                .cables
                .iter()
                .position(|cable| cable.id.as_deref() == Some(id))
                .ok_or_else(|| format!("cable '{id}' not found"))
        })
        .collect()
}

pub(super) fn set_color(candidate: &mut Patch, args: &Value) -> Result<(), String> {
    let color = rgb(args.get("color").ok_or("color required")?)?;
    let id = string(args, "cable_id")?;
    let cable = candidate
        .cables
        .iter_mut()
        .find(|cable| cable.id.as_deref() == Some(id))
        .ok_or("cable not found")?;
    cable.color = Some(color);
    Ok(())
}

pub(super) fn disconnect(
    candidate: &mut Patch,
    args: &Value,
    retired: &mut Vec<String>,
) -> Result<(), String> {
    let indices: BTreeSet<_> = selected_indices(candidate, args)?.into_iter().collect();
    let mut index = 0;
    candidate.cables.retain(|cable| {
        let remove = indices.contains(&index);
        index += 1;
        if remove {
            retired.extend(cable.id.clone());
        }
        !remove
    });
    Ok(())
}

fn route(cable: &Cable) -> (&str, &str, &str, &str) {
    (&cable.from, &cable.output, &cable.to, &cable.input)
}

fn endpoint_matches(cable: &Cable, end: &str, node: &str, port: &str) -> bool {
    if end == "input" {
        cable.to == node && cable.input == port
    } else {
        cable.from == node && cable.output == port
    }
}

pub(super) fn move_endpoints(
    candidate: &mut Patch,
    args: &Value,
    retired: &mut Vec<String>,
) -> Result<(), String> {
    let indices = selected_indices(candidate, args)?;
    let end = string(args, "end")?;
    let endpoint = endpoint(args)?;
    let promote = args
        .get("promote")
        .map(|value| value.as_bool().ok_or("promote must be a boolean"))
        .transpose()?
        .unwrap_or(false);
    // Explicit plug order must be a full selected-ID permutation. Duplicate
    // resolution still uses cable_ids order, independent of this visual order.
    let plug_order = if let Some(order) = args.get("plug_order") {
        let order_args = json!({"cable_ids":order});
        let ordered = selected_indices(candidate, &order_args)?;
        if ordered.iter().copied().collect::<BTreeSet<_>>()
            != indices.iter().copied().collect::<BTreeSet<_>>()
        {
            return Err("plug_order must be a permutation of cable_ids".into());
        }
        ordered
    } else {
        existing_plug_order(candidate, endpoint, indices.clone())
    };
    let mode = optional_string(args, "duplicate_mode", "reject")?;
    if !matches!(mode, "reject" | "discard" | "stack") {
        return Err("duplicate_mode must be reject, discard, or stack".into());
    }
    if mode == "stack" && candidate.version != 2 {
        return Err("stack mode requires patch version 2".into());
    }
    let node = string(args, "node")?;
    let port = string(args, "port")?;
    let target = candidate
        .nodes
        .iter()
        .find(|target| target.id == node)
        .ok_or("destination module not found")?;
    let catalog = catalog();
    let module = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == target.kind)
        .ok_or("unknown destination module kind")?;
    let ports = &module[if end == "input" { "inputs" } else { "outputs" }];
    if !ports
        .as_array()
        .is_some_and(|ports| ports.iter().any(|value| value.as_str() == Some(port)))
    {
        return Err(format!("node '{node}': unknown {end} port '{port}'"));
    }
    if end == "output" && candidate.version == 1 && port != "out" {
        return Err("version 1 supports only source output 'out'".into());
    }
    // Keep same-origin members in the survivor set, including intentional
    // identical stacks. Remove all changed origins before checking new routes.
    let changed: BTreeSet<_> = indices
        .iter()
        .copied()
        .filter(|&index| !endpoint_matches(&candidate.cables[index], end, node, port))
        .collect();
    let should_promote = promote || !changed.is_empty();
    let plug_ids: Vec<_> = plug_order
        .iter()
        .filter_map(|&index| candidate.cables[index].id.clone())
        .collect();
    let mut accepted: Vec<Cable> = candidate
        .cables
        .iter()
        .enumerate()
        .filter(|(index, _)| !changed.contains(index))
        .map(|(_, cable)| cable.clone())
        .collect();
    let mut replacements = BTreeMap::new();
    for index in indices {
        if !changed.contains(&index) {
            continue;
        }
        let mut cable = candidate.cables[index].clone();
        if end == "input" {
            cable.to = node.into();
            cable.input = port.into();
        } else {
            cable.from = node.into();
            cable.output = port.into();
        }
        let duplicate = accepted
            .iter()
            .any(|existing| route(existing) == route(&cable));
        if duplicate && mode == "reject" {
            return Err("move would create an identical cable route; use discard or stack".into());
        }
        retired.extend(cable.id.clone());
        if duplicate && mode == "discard" {
            replacements.insert(index, None);
        } else {
            accepted.push(cable.clone());
            replacements.insert(index, Some(cable));
        }
    }
    // Retain saved accumulation order for both untouched and moved members.
    let mut index = 0;
    candidate.cables.retain_mut(|cable| {
        let replacement = replacements.remove(&index);
        index += 1;
        match replacement {
            Some(Some(updated)) => {
                *cable = updated;
                true
            }
            Some(None) => false,
            None => true,
        }
    });
    if should_promote {
        let retained: Vec<_> = plug_ids
            .iter()
            .filter_map(|id| {
                candidate
                    .cables
                    .iter()
                    .position(|cable| cable.id.as_ref() == Some(id))
            })
            .collect();
        if !retained.is_empty() {
            promote_cable_endpoints(&mut candidate.cables, endpoint, &retained)?;
        }
    }
    Ok(())
}
