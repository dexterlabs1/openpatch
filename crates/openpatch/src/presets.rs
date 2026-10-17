//! Native module preset values and bounded workspace storage. Control-thread only.
use openpatch_core::{catalog, ModuleContent, Node, SavedModuleState, WaveAssets};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
pub const MAX_BYTES: usize = 4 * 1024 * 1024;
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
    if serde_json::to_vec(preset)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_BYTES
    {
        return Err("module preset exceeds 4 MiB".into());
    }
    let object = preset.as_object().ok_or("preset must be an object")?;
    if object.keys().any(|key| {
        !matches!(
            key.as_str(),
            "format"
                | "version"
                | "kind"
                | "params"
                | "bypassed"
                | "score"
                | "rackmcp_sequence"
                | "content"
                | "assets"
        )
    }) {
        return Err(
            "preset accepts only format, version, kind, params, bypassed, score, rackmcp_sequence, content, and assets"
                .into(),
        );
    }
    if preset["format"] != "openpatch-module-preset" || preset["version"].as_u64() != Some(1) {
        return Err("unsupported module preset format or version".into());
    }
    let kind = preset["kind"]
        .as_str()
        .ok_or("preset kind must be a string")?;
    let module = descriptor(kind)?;
    if let Some(content) = preset.get("content") {
        let content: openpatch_core::ModuleContent =
            serde_json::from_value(content.clone()).map_err(|error| error.to_string())?;
        let node: Node =
            serde_json::from_value(json!({"id":"preset", "kind":kind, "content":content}))
                .map_err(|error| error.to_string())?;
        openpatch_core::validate_content(&node)?;
    }
    decode_assets(preset)?;
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
        // These native models distinguish interactive ranges from finite raw
        // storage. Retain loaded values without narrowing gain, shift or slew;
        // the Boolean response aliases still use their ordinary enum checks.
        if (kind == "stock_octave" && name == "octaves")
            || (kind == "stock_process" && name == "slew")
            || (kind == "stock_vca_mix"
                && matches!(
                    name.as_str(),
                    "mix_level" | "level1" | "level2" | "level3" | "level4"
                ))
        {
            continue;
        }
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
    if matches!(kind, "random_values" | "sh_asr" | "midi_cc" | "midi_gate") {
        let node: Node =
            serde_json::from_value(json!({"id":"preset", "kind":kind,"params":params}))
                .map_err(|error| error.to_string())?;
        if kind == "midi_gate" {
            SavedModuleState::from_node(&node).ok_or("invalid MIDI Gate preset")?;
        } else if kind == "midi_cc" {
            openpatch_core::initial_midi_cc_state(&node)?;
        } else if kind == "sh_asr" {
            openpatch_core::initial_shift_register_state(&node)?;
        } else {
            openpatch_core::initial_random_values_state(&node)?;
        }
    }
    if object
        .get("bypassed")
        .is_some_and(|value| !value.is_boolean())
    {
        return Err("preset bypassed must be a boolean".into());
    }
    if let Some(sequence) = preset.get("rackmcp_sequence") {
        if kind != "rackmcp_sequencer" {
            return Err("rackmcp_sequence data requires rackmcp_sequencer".into());
        }
        let mut patch = openpatch_core::Patch::new_v2();
        patch.nodes.push(
            serde_json::from_value(json!({"id":"preset","kind":kind,"rackmcp_sequence":sequence}))
                .map_err(|error| error.to_string())?,
        );
        patch.validate()?;
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
    Ok(())
}

fn is_wavetable(kind: &str) -> bool {
    matches!(kind, "wavetable_vco" | "wavetable_lfo")
}

/// Decode only a portable preset's referenced asset, checking both its digest
/// and interpretation. Descriptor-only inspection objects are never accepted.
fn decode_assets(preset: &Value) -> Result<WaveAssets, String> {
    let kind = preset["kind"]
        .as_str()
        .ok_or("preset kind must be a string")?;
    if !is_wavetable(kind) && preset.get("assets").is_some() {
        return Err("preset assets require a wavetable module".into());
    }
    let referenced = preset
        .get("content")
        .map(|content| serde_json::from_value::<ModuleContent>(content.clone()))
        .transpose()
        .map_err(|error| error.to_string())?;
    let shape = match referenced {
        Some(ModuleContent::Wavetable {
            asset_id,
            wave_points,
            ..
        }) => Some((asset_id, wave_points)),
        _ => None,
    };
    let encoded = match preset.get("assets") {
        Some(value) => Some(value.as_object().ok_or("preset assets must be an object")?),
        None => None,
    };
    let count = encoded.map_or(0, |assets| assets.len());
    if count != usize::from(shape.is_some()) {
        return Err("preset assets must contain exactly the referenced wavetable asset".into());
    }
    let Some((id, points)) = shape else {
        return Ok(WaveAssets::new());
    };
    let payload = encoded
        .and_then(|assets| assets.get(&id))
        .ok_or("preset is missing its referenced wavetable asset")?;
    let asset: openpatch_core::WaveAsset = serde_json::from_value(payload.clone())
        .map_err(|error| format!("invalid preset wavetable asset: {error}"))?;
    if asset.id() != id {
        return Err("preset wavetable asset digest does not match sample bytes".into());
    }
    if points < 2 || points as usize > asset.sample_count() {
        return Err("preset wave length must fit at least one complete wave".into());
    }
    Ok([(id, std::sync::Arc::new(asset))].into_iter().collect())
}

/// Return validated, self-contained samples to merge into the target patch
/// before compiling a candidate. Applying the node alone does not install them.
pub fn assets_from_preset(preset: &Value) -> Result<WaveAssets, String> {
    validate(preset)?;
    decode_assets(preset)
}

/// Compact review metadata, deliberately missing the portable format marker.
/// This cannot be passed back as a valid preset with its samples omitted.
pub fn summary(preset: &Value) -> Result<Value, String> {
    let assets = assets_from_preset(preset)?;
    let mut view = preset.clone();
    let object = view.as_object_mut().ok_or("preset must be an object")?;
    object.remove("format");
    object.remove("version");
    object.remove("assets");
    object.insert("portable_preset".into(), json!(false));
    object.insert("asset_data_included".into(), json!(false));
    if !assets.is_empty() {
        object.insert(
            "assets".into(),
            Value::Object(
                assets
                    .iter()
                    .map(|(id, asset)| (id.clone(), asset.descriptor()))
                    .collect(),
            ),
        );
    }
    Ok(view)
}

/// Capture effective defaults. The caller supplies coherent current SEQ3 Run.
pub fn capture(node: &Node, running: Option<bool>) -> Result<Value, String> {
    capture_state(node, running.map(SavedModuleState::Seq3Run))
}

/// Capture current typed persistent state. Depressed controls and pulse tails are omitted.
pub fn capture_state(node: &Node, state: Option<SavedModuleState>) -> Result<Value, String> {
    capture_state_with_assets(node, state, &WaveAssets::new())
}

/// Capture a portable single-module preset with only its referenced samples.
pub fn capture_state_with_assets(
    node: &Node,
    state: Option<SavedModuleState>,
    assets: &WaveAssets,
) -> Result<Value, String> {
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
            .filter(|state| {
                state.kind() == expected.kind()
                    && !matches!(state, SavedModuleState::StockSeq3Fields { .. })
            })
            .ok_or_else(|| {
                format!(
                    "{} capture requires current persistent state",
                    expected.kind()
                )
            })?;
        actual.write_params(&mut params);
    }
    let mut preset = json!({"format":"openpatch-module-preset","version":1,"kind":node.kind,"params":params,"bypassed":node.bypassed});
    if node.kind == "rackmcp_sequencer" {
        preset["rackmcp_sequence"] =
            serde_json::to_value(node.rackmcp_sequence.clone().unwrap_or_default())
                .map_err(|error| error.to_string())?;
    }
    if node.kind == "score_sequencer" {
        preset["score"] = serde_json::to_value(node.score.clone().unwrap_or_default())
            .map_err(|error| error.to_string())?;
    }
    if let Some(content) = &node.content {
        preset["content"] = serde_json::to_value(content).map_err(|error| error.to_string())?;
    } else if let Some(text) = node.effective_notes_text() {
        preset["content"] = json!({"type":"notes","text":text});
    } else if let Some(width_hp) = node.effective_blank_width_hp() {
        preset["content"] = json!({"type":"blank","width_hp":width_hp});
    }
    if let Some(ModuleContent::Wavetable { asset_id, .. }) = &node.content {
        let asset = assets
            .get(asset_id)
            .ok_or("custom wavetable capture requires its referenced asset samples")?;
        if asset.id() != asset_id {
            return Err("capture wavetable asset digest does not match sample bytes".into());
        }
        preset["assets"] =
            serde_json::to_value([(asset_id, asset)].into_iter().collect::<BTreeMap<_, _>>())
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
    if let Some(content) = preset.get("content") {
        node.content =
            Some(serde_json::from_value(content.clone()).map_err(|error| error.to_string())?);
    } else if is_wavetable(&node.kind) {
        // A wavetable preset without custom content selects the production
        // default bank. Legacy annotation presets retain their partial merge.
        node.content = None;
    }
    if let Some(sequence) = preset.get("rackmcp_sequence") {
        node.rackmcp_sequence =
            Some(serde_json::from_value(sequence.clone()).map_err(|error| error.to_string())?);
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
    Ok(matches!(node.kind.as_str(), "seq3" | "stock_seq3")
        .then(|| {
            preset["params"]["running"]
                .as_f64()
                .map(|value| value as f32 == 1.0)
        })
        .flatten())
}

pub fn read(path: &Path) -> Result<Value, String> {
    read_with_digest(path).map(|(preset, _)| preset)
}

/// Hash the exact bounded bytes reviewed by the caller so applying from a file
/// can reject a subsequent replacement instead of applying unseen contents.
pub fn read_with_digest(path: &Path) -> Result<(Value, String), String> {
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
        return Err("module preset exceeds 4 MiB".into());
    }
    let preset = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid module preset JSON: {error}"))?;
    validate(&preset)?;
    Ok((preset, format!("{:x}", Sha256::digest(&bytes))))
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
        return Err("module preset exceeds 4 MiB".into());
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
                content: None,
                rackmcp_sequence: None,
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
