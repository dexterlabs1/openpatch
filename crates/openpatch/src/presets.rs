//! Native module preset values and bounded workspace storage. Control-thread only.
use openpatch_core::{catalog, Node, SavedModuleState};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        OnceLock,
    },
};

pub const EXTENSION: &str = ".opreset";
pub const MAX_BYTES: usize = 65_536;
pub const MAX_LIBRARY_FILES: usize = 1024;

fn descriptor(kind: &str) -> Result<&'static Value, String> {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(catalog)["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|module| module["kind"] == kind)
        .ok_or_else(|| format!("unknown preset module kind '{kind}'"))
}

/// Validate a complete or partial native preset without inventing omitted values.
/// Target-dependent graph/sample-rate constraints are checked when applying it.
pub fn validate(preset: &Value) -> Result<(), String> {
    let object = preset.as_object().ok_or("preset must be an object")?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "format" | "version" | "kind" | "params" | "bypassed" | "score"
        )
    }) {
        return Err(
            "preset accepts only format, version, kind, params, bypassed, and score".into(),
        );
    }
    if preset["format"] != "openpatch-module-preset" || preset["version"].as_u64() != Some(1) {
        return Err("unsupported module preset format or version".into());
    }
    let kind = preset["kind"]
        .as_str()
        .ok_or("preset kind must be a string")?;
    let module = descriptor(kind)?;
    let params = preset["params"]
        .as_object()
        .ok_or("preset params must be an object")?;
    for (name, value) in params {
        let spec = module["params"]
            .get(name)
            .ok_or_else(|| format!("unknown preset parameter '{name}'"))?;
        let value = value
            .as_f64()
            .filter(|value| value.is_finite() && (*value as f32).is_finite())
            .ok_or_else(|| format!("preset parameter '{name}' must be a finite number"))?;
        // Validate in the same f32 domain as Node/core parameter parsing.
        let value = value as f32;
        if value < spec["min"].as_f64().unwrap() as f32
            || value > spec["max"].as_f64().unwrap() as f32
        {
            return Err(format!(
                "preset parameter '{name}' is outside catalog bounds"
            ));
        }
        if spec["step"].as_f64() == Some(1.0) && value.fract() != 0.0 {
            return Err(format!("preset parameter '{name}' must be an integer"));
        }
        if let Some(values) = spec["values"].as_array() {
            if !values
                .iter()
                .any(|allowed| allowed.as_f64().map(|value| value as f32) == Some(value))
            {
                return Err(format!(
                    "preset parameter '{name}' must be a catalog enum value"
                ));
            }
        }
    }
    if kind == "random_values" {
        let node: Node =
            serde_json::from_value(json!({"id":"preset", "kind":kind,"params":params}))
                .map_err(|error| error.to_string())?;
        openpatch_core::initial_random_values_state(&node)?;
    }
    if object
        .get("bypassed")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err("preset bypassed must be a boolean".into());
    }
    if let Some(score) = preset.get("score") {
        if kind != "score_sequencer" {
            return Err("score data requires score_sequencer".into());
        }
        let mut patch = openpatch_core::Patch::new_v2();
        patch.nodes.push(
            serde_json::from_value(json!({"id":"preset", "kind":kind,"score":score}))
                .map_err(|error| error.to_string())?,
        );
        patch.validate()?;
    }
    if serde_json::to_vec(preset)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_BYTES
    {
        return Err("module preset exceeds 64 KiB".into());
    }
    Ok(())
}

/// Capture effective defaults. The caller supplies coherent current SEQ3 Run.
pub fn capture(node: &Node, running: Option<bool>) -> Result<Value, String> {
    capture_state(node, running.map(SavedModuleState::Seq3Run))
}

/// Capture current typed persistent state. Depressed controls and pulse tails are omitted.
pub fn capture_state(node: &Node, state: Option<SavedModuleState>) -> Result<Value, String> {
    let module = descriptor(&node.kind)?;
    let mut params = BTreeMap::new();
    for (name, spec) in module["params"].as_object().unwrap() {
        if spec["hidden"] == true {
            continue;
        }
        params.insert(
            name.clone(),
            node.params
                .get(name)
                .copied()
                .unwrap_or(spec["default"].as_f64().unwrap() as f32),
        );
    }
    if let Some(expected) = SavedModuleState::from_node(node) {
        let actual = state
            .filter(|state| state.kind() == expected.kind())
            .ok_or_else(|| {
                format!(
                    "{} capture requires current {}",
                    if expected.kind() == "seq3" {
                        "SEQ3"
                    } else {
                        "PUSH"
                    },
                    if expected.kind() == "seq3" {
                        "Run"
                    } else {
                        "HOLD"
                    }
                )
            })?;
        actual.write_params(&mut params);
    }
    let mut preset = json!({"format":"openpatch-module-preset","version":1,"kind":node.kind,"params":params,"bypassed":node.bypassed});
    if node.kind == "score_sequencer" {
        preset["score"] = serde_json::to_value(node.score.clone().unwrap_or_default())
            .map_err(|error| error.to_string())?;
    }
    validate(&preset)?;
    Ok(preset)
}

pub fn apply(node: &mut Node, preset: &Value) -> Result<Option<bool>, String> {
    validate(preset)?;
    if preset["kind"] != node.kind {
        return Err("module preset kind does not match target module".into());
    }
    if let Some(score) = preset.get("score") {
        node.score =
            Some(serde_json::from_value(score.clone()).map_err(|error| error.to_string())?);
    }
    for (name, value) in preset["params"].as_object().unwrap() {
        node.params
            .insert(name.clone(), value.as_f64().unwrap() as f32);
    }
    if let Some(bypassed) = preset.get("bypassed") {
        node.bypassed = bypassed.as_bool().unwrap();
    }
    Ok((node.kind == "seq3")
        .then(|| {
            preset["params"]["running"]
                .as_f64()
                .map(|value| value as f32 == 1.0)
        })
        .flatten())
}

pub fn read(path: &Path) -> Result<Value, String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| error.to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("preset must be a regular file, not a symlink".into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("preset must not be a reparse point".into());
        }
    }
    if !metadata.is_file() {
        return Err("preset must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("module preset exceeds 64 KiB".into());
    }
    let preset = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid module preset JSON: {error}"))?;
    validate(&preset)?;
    Ok(preset)
}

struct Staged(PathBuf);
impl Drop for Staged {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Publish by an atomic, same-filesystem hard link; an existing target is never
/// replaced. Only this function's exclusively-created staging file is removed.
pub fn save_new(path: &Path, preset: &Value) -> Result<(), String> {
    validate(preset)?;
    let bytes = serde_json::to_vec_pretty(preset).map_err(|error| error.to_string())?;
    if bytes.len() > MAX_BYTES {
        return Err("module preset exceeds 64 KiB".into());
    }
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let directory = path.parent().ok_or("preset path has no parent")?;
    for _ in 0..32 {
        let temporary = directory.join(format!(
            ".openpatch-preset-{}-{}.tmp",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        };
        let staged = Staged(temporary);
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        fs::hard_link(&staged.0, path).map_err(|error| error.to_string())?;
        return Ok(());
    }
    Err("could not create an exclusive preset staging file".into())
}

pub fn list(
    workspace: &Path,
    kind: Option<&str>,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    if let Some(kind) = kind {
        descriptor(kind)?;
    }
    let mut files = Vec::new();
    for entry in fs::read_dir(workspace).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.starts_with('.')
            || name.len() > 120
            || !name.ends_with(EXTENSION)
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        {
            continue;
        }
        files.push((name, entry.path()));
        if files.len() > MAX_LIBRARY_FILES {
            return Err("preset library exceeds 1024 files".into());
        }
    }
    files.retain(|(name, _)| {
        name.to_ascii_lowercase()
            .contains(&query.to_ascii_lowercase())
    });
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut entries = Vec::new();
    for (filename, path) in files {
        match read(&path) {
            Ok(preset) => {
                if kind.is_none_or(|kind| preset["kind"] == kind) {
                    entries.push(json!({"filename":filename,"kind":preset["kind"],"valid":true,"error":null}));
                }
            }
            Err(error) => {
                entries.push(json!({"filename":filename,"kind":null,"valid":false,"error":error}))
            }
        }
    }
    let total = entries.len();
    let presets: Vec<_> = entries.into_iter().skip(offset).take(limit).collect();
    Ok(json!({"presets":presets,"offset":offset,"limit":limit,"total":total}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_kind_and_float_boundary_captures_and_roundtrips() {
        for module in catalog()["modules"].as_array().unwrap() {
            let kind = module["kind"].as_str().unwrap();
            let base = Node {
                score: None,
                id: "test".into(),
                kind: kind.into(),
                params: BTreeMap::new(),
                bypassed: false,
            };
            let defaults = capture_state(&base, SavedModuleState::from_node(&base)).unwrap();
            assert_eq!(
                defaults["params"].as_object().unwrap().len(),
                module["params"].as_object().unwrap().len()
            );
            for (name, spec) in module["params"].as_object().unwrap() {
                for boundary in ["min", "max"] {
                    let mut original = base.clone();
                    // Reserved state fields are valid only within a complete
                    // snapshot; never invent their catalog defaults individually.
                    if spec["hidden"] == true {
                        if let Some(state) = SavedModuleState::from_node(&base) {
                            state.write_params(&mut original.params);
                        }
                    }
                    let value = spec[boundary].as_f64().unwrap() as f32;
                    original.params.insert(name.clone(), value);
                    let preset = capture_state(&original, SavedModuleState::from_node(&original))
                        .unwrap_or_else(|error| panic!("{kind}.{name} {boundary}: {error}"));
                    let bytes = serde_json::to_vec(&preset).unwrap();
                    let decoded: Value = serde_json::from_slice(&bytes).unwrap();
                    validate(&decoded).unwrap();
                    let mut restored = base.clone();
                    apply(&mut restored, &decoded).unwrap();
                    assert_eq!(restored.params[name], value, "{kind}.{name} {boundary}");
                }
            }
        }
    }
}
