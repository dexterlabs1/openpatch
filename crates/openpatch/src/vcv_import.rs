//! Recoverable, bounded Rack import. Every conversion is pinned to a source
//! identity; original JSON (including its bytes) and archive members stay immutable.
//!
//! Fundamental v2.6.4 = e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e.
//! VCA-1, ADSR, LFO, Noise, VCMixer, WTLFO.cpp and Wavetable.hpp are
//! byte-identical to the existing native source reference 10dd0160c664770910e5584b7b00498cc48d9ddd.
//! VCO and VCF use separate source-version processors; they do not alias the
//! newer Stock identities. VCF startup history and RNG ownership are disclosed.
//! Primary source: https://github.com/VCVRack/Fundamental/tree/e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e/src
//! This is source-behavior conversion, not official binary/hardware equivalence.

use crate::project_document::{
    DocumentCable, DocumentGraph, DocumentNode, MappingFidelity, ObservedPort, PortDirection,
    ProjectDocument, ProjectLayout, SourceBlob, SourceCable, SourceMapping, SourceModule,
    SourcePackage, DOCUMENT_VERSION,
};
use openpatch_core::{Cable, ModuleContent, Node, WaveAsset, WaveAssets, MAX_CABLES, MAX_NODES};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

#[derive(Debug)]
pub struct ImportResult {
    pub document: ProjectDocument,
    pub report: ImportReport,
}

#[derive(Clone, Debug, Serialize)]
pub struct ImportReport {
    pub package_id: String,
    pub rack_version: Option<String>,
    pub module_count: usize,
    pub cable_count: usize,
    pub mapped_count: usize,
    pub unresolved_count: usize,
    /// Archive assets, excluding the separately retained original patch.json.
    pub asset_count: usize,
    pub omitted_route_count: usize,
    pub modules: Vec<ModuleImport>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModuleImport {
    pub id: String,
    pub source_id: u64,
    pub plugin: Option<String>,
    pub model: Option<String>,
    pub version: Option<String>,
    pub native_kind: Option<String>,
    pub reason: Option<String>,
}

struct Route {
    id: u64,
    from: u64,
    to: u64,
    output: u32,
    input: u32,
    color: Option<[u8; 3]>,
    input_order: Option<u16>,
    output_order: Option<u16>,
}

/// Decode and validate an isolated candidate; no caller state or filesystem is mutated.
pub fn import(bytes: &[u8]) -> Result<ImportResult, String> {
    let archive = crate::vcv_archive::read(bytes)?;
    let patch: Value = serde_json::from_slice(&archive.patch_json).map_err(|e| e.to_string())?;
    let modules = patch["modules"]
        .as_array()
        .ok_or("VCV modules must be an array")?;
    let empty = Vec::new();
    let cables = match patch.get("cables") {
        Some(value) => value.as_array().ok_or("VCV cables must be an array")?,
        None => &empty,
    };
    if modules.len() > MAX_NODES || cables.len() > MAX_CABLES {
        return Err("VCV patch exceeds module or cable limit".into());
    }
    let mut module_ids = BTreeMap::new();
    for (index, module) in modules.iter().enumerate() {
        if !module.is_object() {
            return Err("VCV module must be an object".into());
        }
        let id = source_id(&module["id"], "module id")?;
        if module_ids.insert(id, index).is_some() {
            return Err(format!("duplicate VCV module id {id}"));
        }
        validate_params(module)?;
        if module
            .get("bypass")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(format!("VCV module {id} bypass must be Boolean"));
        }
    }
    let mut cable_ids = BTreeSet::new();
    let mut inputs = BTreeSet::new();
    let mut observed: BTreeMap<u64, BTreeSet<(PortDirection, u32)>> = BTreeMap::new();
    let mut routes = Vec::new();
    for cable in cables {
        if !cable.is_object() {
            return Err("VCV cable must be an object".into());
        }
        let route = Route {
            id: source_id(&cable["id"], "cable id")?,
            from: source_id(&cable["outputModuleId"], "outputModuleId")?,
            to: source_id(&cable["inputModuleId"], "inputModuleId")?,
            output: port_index(&cable["outputId"])?,
            input: port_index(&cable["inputId"])?,
            color: parse_color(cable.get("color"))?,
            input_order: plug_order(cable.get("inputPlugOrder"))?,
            output_order: plug_order(cable.get("outputPlugOrder"))?,
        };
        if !cable_ids.insert(route.id) {
            return Err(format!("duplicate VCV cable id {}", route.id));
        }
        if !inputs.insert((route.to, route.input)) {
            return Err("multiple VCV cables address the same input".into());
        }
        for (id, direction, port) in [
            (route.from, PortDirection::Output, route.output),
            (route.to, PortDirection::Input, route.input),
        ] {
            let index = module_ids
                .get(&id)
                .ok_or("VCV cable references a missing module")?;
            // For a declared source manifest even an unresolved conversion must
            // reject a nonexistent port. Unknown plugin manifests are not invented.
            if let Some((inputs, outputs)) = source_port_counts(&modules[*index]) {
                let count = if direction == PortDirection::Input {
                    inputs
                } else {
                    outputs
                };
                if port as usize >= count {
                    return Err(format!(
                        "VCV cable references invalid source port {port} on module {id}"
                    ));
                }
            }
            observed.entry(id).or_default().insert((direction, port));
        }
        routes.push(route);
    }

    let mut source_blobs = BTreeMap::new();
    let mut asset_names = BTreeMap::new();
    let original = SourceBlob::new(archive.patch_json)?;
    asset_names.insert("patch.json".into(), original.id());
    source_blobs.insert(original.id(), original);
    for (name, bytes) in &archive.assets {
        let blob = SourceBlob::new(bytes.clone())?;
        asset_names.insert(name.clone(), blob.id());
        source_blobs.entry(blob.id()).or_insert(blob);
    }
    let mut missing_assets = BTreeSet::new();
    let mut assets = WaveAssets::new();
    let mut nodes = Vec::new();
    let mut summaries = Vec::new();
    let mut native_ports = BTreeMap::new();
    for (index, module) in modules.iter().enumerate() {
        let source_id = source_id(&module["id"], "module id")?;
        let id = module_id(source_id);
        let mapping = manifest(module);
        let converted = mapping
            .as_ref()
            .ok_or_else(|| unsupported_reason(module))
            .and_then(|mapping| convert(module, &id, mapping, &archive.assets));
        let (node, reason, kind) = match converted {
            Ok((node, asset)) => {
                if let Some(asset) = asset {
                    assets.insert(asset.id().into(), Arc::new(asset));
                }
                let mapping = mapping.expect("successful conversion needs mapping");
                if observed.get(&source_id).is_some_and(|ports| {
                    ports.iter().any(|(direction, index)| {
                        let names = if *direction == PortDirection::Input {
                            mapping.inputs
                        } else {
                            mapping.outputs
                        };
                        names[*index as usize].is_empty()
                    })
                }) {
                    (
                        None,
                        Some("incident removed source port has no native representation".into()),
                        None,
                    )
                } else {
                    let kind = Some(node.kind.clone());
                    native_ports.insert(source_id, mapping);
                    (Some(node), None, kind)
                }
            }
            Err(reason) => {
                if module["plugin"] == "Fundamental"
                    && module["model"] == "LFO2"
                    && module["version"] == "2.6.4"
                {
                    let path = format!("modules/{source_id}/wavetable.wav");
                    if !archive.assets.contains_key(&path) {
                        missing_assets.insert(path);
                    }
                }
                (None, Some(reason), None)
            }
        };
        summaries.push(ModuleImport {
            id: id.clone(),
            source_id,
            plugin: string(module, "plugin"),
            model: string(module, "model"),
            version: string(module, "version"),
            native_kind: kind,
            reason,
        });
        nodes.push((id, index, source_id, node));
    }
    let package = SourcePackage {
        patch: Arc::new(patch.clone()),
        assets: asset_names,
        missing_assets,
    };
    let package_id = package.id()?;
    let nodes = nodes
        .into_iter()
        .map(|(id, index, source_id, node)| {
            let source = SourceModule {
                package: package_id.clone(),
                index,
                mapping: node.as_ref().map(|_| SourceMapping {
                    id: format!("vcv/Fundamental/{}/2.6.4", module_model(&modules[index])),
                    version: 1,
                    fidelity: MappingFidelity::SourceBehavior,
                }),
            };
            match node {
                Some(node) => DocumentNode::Native {
                    node,
                    source: Some(source),
                },
                None => DocumentNode::Unresolved {
                    id,
                    source,
                    ports: observed
                        .remove(&source_id)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|(direction, index)| ObservedPort { direction, index })
                        .collect(),
                },
            }
        })
        .collect();
    let cables = routes
        .into_iter()
        .enumerate()
        .map(|(index, route)| DocumentCable {
            route: Cable {
                id: Some(format!("vcv_c_{}", route.id)),
                from: module_id(route.from),
                to: module_id(route.to),
                output: route_port(
                    native_ports.get(&route.from),
                    PortDirection::Output,
                    route.output,
                ),
                input: route_port(
                    native_ports.get(&route.to),
                    PortDirection::Input,
                    route.input,
                ),
                color: route.color,
                input_order: route.input_order,
                output_order: route.output_order,
            },
            source: Some(SourceCable {
                package: package_id.clone(),
                index,
            }),
        })
        .collect();
    let mut warnings = vec!["Source-behavior mappings use OpenPatch scheduling, lifecycle and random streams; no official binary or hardware equivalence is claimed.".into()];
    if summaries
        .iter()
        .any(|module| module.native_kind.as_deref() == Some("rack264_vcf"))
    {
        warnings.push("Fundamental 2.6.4 VCF starts every previous-input history at zero because its source constructor leaves that history undefined; undefined cold-start equivalence is unclaimed. Its deterministic module-local noise RNG intentionally differs from Rack's global random stream.".into());
    }
    let layout = layout(&patch, &mut warnings)?;
    let document = ProjectDocument {
        format: "openpatch-project".into(),
        version: DOCUMENT_VERSION,
        graph: DocumentGraph {
            version: 2,
            sample_rate: 48000,
            assets,
            nodes,
            cables,
            output: None,
            output_port: "out".into(),
        },
        layout,
        sources: BTreeMap::from([(package_id.clone(), Arc::new(package))]),
        source_blobs,
    };
    document.validate_structure()?;
    let mapped_count = summaries
        .iter()
        .filter(|entry| entry.native_kind.is_some())
        .count();
    let omitted_route_count = document
        .graph
        .cables
        .iter()
        .filter(|cable| {
            [cable.route.from.as_str(), cable.route.to.as_str()]
                .iter()
                .any(|id| {
                    summaries
                        .iter()
                        .any(|entry| entry.id == *id && entry.native_kind.is_none())
                })
        })
        .count();
    let report = ImportReport {
        package_id,
        rack_version: string(&patch, "version"),
        module_count: modules.len(),
        cable_count: document.graph.cables.len(),
        mapped_count,
        unresolved_count: modules.len() - mapped_count,
        asset_count: archive.assets.len(),
        omitted_route_count,
        modules: summaries,
        warnings,
    };
    Ok(ImportResult { document, report })
}

fn module_id(id: u64) -> String {
    format!("vcv_m_{id}")
}
fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn module_model(value: &Value) -> &str {
    value["model"].as_str().unwrap_or("")
}
fn source_id(value: &Value, field: &str) -> Result<u64, String> {
    value
        .as_u64()
        .filter(|id| *id <= i64::MAX as u64)
        .ok_or_else(|| {
            format!("VCV {field} must be a nonnegative integer within signed 64-bit range")
        })
}
fn port_index(value: &Value) -> Result<u32, String> {
    value
        .as_u64()
        .and_then(|id| u32::try_from(id).ok())
        .ok_or_else(|| "VCV port index must be an unsigned 32-bit integer".into())
}
fn plug_order(value: Option<&Value>) -> Result<Option<u16>, String> {
    value
        .map(|value| {
            value
                .as_u64()
                .and_then(|v| u16::try_from(v).ok())
                .ok_or_else(|| "VCV plug order must be an unsigned 16-bit integer".into())
        })
        .transpose()
}
fn parse_color(value: Option<&Value>) -> Result<Option<[u8; 3]>, String> {
    value
        .map(|value| {
            let text = value.as_str().ok_or("VCV cable color must be #RRGGBB")?;
            if text.len() != 7
                || !text.starts_with('#')
                || !text[1..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("VCV cable color must be #RRGGBB".into());
            }
            let color = u32::from_str_radix(&text[1..], 16).map_err(|e| e.to_string())?;
            Ok([(color >> 16) as u8, (color >> 8) as u8, color as u8])
        })
        .transpose()
}
fn validate_params(module: &Value) -> Result<(), String> {
    if let Some(params) = module.get("params") {
        let params = params
            .as_array()
            .ok_or("VCV module params must be an array")?;
        if params.len() > 4096 {
            return Err("VCV module exceeds parameter count limit".into());
        }
        let mut ids = BTreeSet::new();
        for param in params {
            let id = port_index(&param["id"])?;
            if !param.is_object() || !ids.insert(id) {
                return Err("invalid or duplicate VCV parameter id".into());
            }
            let number = param["value"]
                .as_f64()
                .ok_or("VCV parameter value must be finite numeric")?;
            if !(number as f32).is_finite() {
                return Err("VCV parameter value exceeds float32 range".into());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Manifest {
    kind: &'static str,
    params: &'static [&'static str],
    inputs: &'static [&'static str],
    outputs: &'static [&'static str],
}
fn manifest(module: &Value) -> Option<Manifest> {
    if module["plugin"] != "Fundamental" || module["version"] != "2.6.4" {
        return None;
    }
    Some(match module_model(module) {
        // Removed MODE/FINE slots retain their source indices and no semantics.
        "VCO" => Manifest {
            kind: "rack264_vco",
            params: &[
                "",
                "soft_sync",
                "frequency",
                "",
                "fm_amount",
                "pulse_width",
                "pwm_amount",
                "linear_fm",
            ],
            inputs: &["pitch", "fm", "sync", "pwm"],
            outputs: &["sine", "triangle", "saw", "square"],
        },
        "VCF" => Manifest {
            kind: "rack264_vcf",
            params: &[
                "frequency",
                "",
                "resonance",
                "cutoff_cv_amount",
                "drive",
                "resonance_cv_amount",
                "drive_cv_amount",
            ],
            inputs: &["cutoff_cv", "resonance_cv", "drive_cv", "signal"],
            outputs: &["lowpass", "highpass"],
        },
        "VCA-1" => Manifest {
            kind: "stock_vca",
            params: &["level", "response"],
            inputs: &["cv", "signal"],
            outputs: &["out"],
        },
        "ADSR" => Manifest {
            kind: "adsr_eg",
            params: &[
                "attack",
                "decay",
                "sustain",
                "release",
                "attack_amount",
                "decay_amount",
                "sustain_amount",
                "release_amount",
                "",
            ],
            inputs: &[
                "attack_cv",
                "decay_cv",
                "sustain_cv",
                "release_cv",
                "gate",
                "retrigger",
            ],
            outputs: &["out"],
        },
        // Source index 4 and input index 1 are removed, retained unused slots.
        "LFO" => Manifest {
            kind: "stock_lfo",
            params: &[
                "offset",
                "invert",
                "rate",
                "fm_amount",
                "",
                "pulse_width",
                "pwm_amount",
            ],
            inputs: &["fm", "", "reset", "pwm", "clock"],
            outputs: &["sine", "triangle", "saw", "square"],
        },
        "Noise" => Manifest {
            kind: "stock_noise",
            params: &[],
            inputs: &[],
            outputs: &["white", "pink", "red", "violet", "blue", "gray", "black"],
        },
        "VCMixer" => Manifest {
            kind: "stock_vca_mix",
            params: &["mix_level", "level1", "level2", "level3", "level4"],
            inputs: &[
                "mix_cv", "in1", "in2", "in3", "in4", "cv1", "cv2", "cv3", "cv4",
            ],
            outputs: &["mix", "out1", "out2", "out3", "out4"],
        },
        "LFO2" => Manifest {
            kind: "wavetable_lfo",
            params: &[
                "offset",
                "invert",
                "rate",
                "position",
                "fm_amount",
                "position_cv_amount",
            ],
            inputs: &["fm", "reset", "position_cv", "clock"],
            outputs: &["out"],
        },
        _ => return None,
    })
}
fn unsupported_reason(_module: &Value) -> String {
    "no mapping for this exact plugin/model/version identity".into()
}
fn source_port_counts(module: &Value) -> Option<(usize, usize)> {
    manifest(module).map(|mapping| (mapping.inputs.len(), mapping.outputs.len()))
}
fn route_port(mapping: Option<&Manifest>, direction: PortDirection, index: u32) -> String {
    match mapping {
        Some(mapping) => (if direction == PortDirection::Input {
            mapping.inputs
        } else {
            mapping.outputs
        })[index as usize]
            .into(),
        None => ObservedPort { direction, index }.key(),
    }
}

fn convert(
    module: &Value,
    id: &str,
    mapping: &Manifest,
    source_assets: &BTreeMap<String, Vec<u8>>,
) -> Result<(Node, Option<WaveAsset>), String> {
    let mut params = BTreeMap::new();
    if matches!(mapping.kind, "rack264_vco" | "rack264_vcf")
        && module.get("data").is_some_and(|data| {
            !data.is_null() && !data.as_object().is_some_and(|data| data.is_empty())
        })
    {
        return Err(
            "unrecognized source module data is outside the pinned voice mapping contract".into(),
        );
    }
    if let Some(values) = module["params"].as_array() {
        if mapping.kind == "rack264_vcf" {
            // Genuine Module::fromJson calls paramsFromJson only when params
            // exists. The 2.6.4 override supplies 1 before saved entries apply.
            // An entirely absent params property retains constructor defaults0.
            params.insert("resonance_cv_amount".into(), 1.0);
            params.insert("drive_cv_amount".into(), 1.0);
        }
        for value in values {
            let index = port_index(&value["id"])? as usize;
            let name = *mapping
                .params
                .get(index)
                .ok_or("unrecognized source parameter index")?;
            let value = value["value"].as_f64().ok_or("invalid parameter value")? as f32;
            if name.is_empty() {
                // ADSR PUSH is a runtime hold, never silently turn a saved hold off.
                if mapping.kind == "adsr_eg" && value != 0.0 {
                    return Err("saved ADSR PUSH hold cannot be represented durably".into());
                }
                continue;
            }
            let valid = match (mapping.kind, name) {
                ("rack264_vco", "frequency") => (-54.0..=54.0).contains(&value),
                ("rack264_vco", "linear_fm" | "soft_sync") => value == 0.0 || value == 1.0,
                ("rack264_vcf", "frequency") => {
                    let c4: f32 = 261.6256;
                    let min = ((c4 / 8000.0).log2() + 5.0) / 10.0;
                    let max = ((8000.0 / c4).log2() + 5.0) / 10.0;
                    (min..=max).contains(&value)
                }
                ("rack264_vcf", "drive") => (-1.0..=1.0).contains(&value),
                (_, name) => match name {
                    "mix_level" | "level1" | "level2" | "level3" | "level4" => true, // source saved raw gains
                    "response" | "offset" | "invert" => value == 0.0 || value == 1.0,
                    "rate" => (-8.0..=10.0).contains(&value),
                    "pulse_width" => (0.01..=0.99).contains(&value),
                    name if name.ends_with("amount") => (-1.0..=1.0).contains(&value),
                    _ => (0.0..=1.0).contains(&value),
                },
            };
            if !valid {
                return Err(format!(
                    "source parameter {name} is outside the native supported contract"
                ));
            }
            let value = if mapping.kind == "rack264_vco" && name == "soft_sync" {
                // Source switch is 1=hard, 0=soft; native flag is 1=soft.
                1.0 - value
            } else {
                value
            };
            params.insert(name.into(), value);
        }
    }
    if mapping.kind == "stock_vca_mix" {
        for (key, name) in [("chExp", "channel_exp"), ("mixExp", "mix_exp")] {
            params.insert(
                name.into(),
                if module["data"][key] == Value::Bool(true) {
                    1.0
                } else {
                    0.0
                },
            );
        }
    }
    let mut node = Node {
        id: id.into(),
        kind: mapping.kind.into(),
        params,
        content: None,
        score: None,
        bypassed: module["bypass"].as_bool().unwrap_or(false),
    };
    let mut asset = None;
    if mapping.kind == "wavetable_lfo" {
        // Source paramsFromJson explicitly gives this missing v2 attenuverter 1.
        node.params
            .entry("position_cv_amount".into())
            .or_insert(1.0);
        let source_id = source_id(&module["id"], "module id")?;
        let path = format!("modules/{source_id}/wavetable.wav");
        let bytes = source_assets
            .get(&path)
            .ok_or_else(|| format!("required source wavetable asset is missing: {path}"))?;
        let decoded = openpatch_core::wavetable_file::decode_wavetable(bytes, "wav")?;
        // Rack loads patch storage after module data; power-of-two WAV rate wins.
        let points = match decoded.suggested_wave_length {
            Some(points) => points,
            None => match module["data"].get("waveLen") {
                Some(value) => value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or("invalid source waveLen")?,
                None => 1024,
            },
        };
        if points < 2 || points > decoded.samples.len() {
            return Err("source wavetable has unsupported geometry".into());
        }
        let name = match module["data"].get("filename") {
            Some(value) => value.as_str().ok_or("invalid source wavetable filename")?,
            None => "Basic.wav",
        };
        if name.len() > 1024 || name.contains('\0') {
            return Err("source wavetable filename exceeds native bounds".into());
        }
        let wave = WaveAsset::new(&decoded.samples)?;
        node.content = Some(ModuleContent::Wavetable {
            asset_id: wave.id().into(),
            wave_points: u32::try_from(points).map_err(|_| "waveLen exceeds native bounds")?,
            name: name.into(),
        });
        asset = Some(wave);
    }
    Ok((node, asset))
}

fn layout(patch: &Value, warnings: &mut Vec<String>) -> Result<ProjectLayout, String> {
    let mut layout = ProjectLayout::default();
    if let Some(zoom) = patch.get("zoom") {
        let zoom = zoom.as_f64().ok_or("invalid VCV zoom")? as f32;
        if !zoom.is_finite() || zoom <= 0.0 {
            return Err("invalid VCV zoom".into());
        }
        layout.zoom = zoom.clamp(0.25, 4.0);
        if layout.zoom != zoom {
            warnings.push("Viewport zoom constrained to native 0.25..4; original zoom retained in source package.".into());
        }
    }
    // Rack v2 persists grid coordinates; native panels use the same 15px HP
    // width, with 420px row spacing. Native pan is opposite the scroll offset.
    if let Some(offset) = patch.get("gridOffset") {
        let offset = pair(offset)?;
        layout.pan = [
            -offset[0] * 15.0 * layout.zoom,
            -offset[1] * 420.0 * layout.zoom,
        ];
    }
    for module in patch["modules"].as_array().ok_or("missing modules")? {
        if let Some(pos) = module.get("pos") {
            let pos = pair(pos)?;
            layout.positions.insert(
                module_id(source_id(&module["id"], "module id")?),
                [pos[0] * 15.0, pos[1] * 420.0],
            );
        }
    }
    Ok(layout)
}
fn pair(value: &Value) -> Result<[f32; 2], String> {
    let values = value
        .as_array()
        .filter(|values| values.len() == 2)
        .ok_or("VCV position/offset must contain two numbers")?;
    let mut result = [0.0; 2];
    for (to, from) in result.iter_mut().zip(values) {
        *to = from.as_f64().ok_or("VCV position/offset must be numeric")? as f32;
        if !to.is_finite() {
            return Err("VCV position/offset exceeds float32 range".into());
        }
    }
    Ok(result)
}

#[cfg(test)]
#[path = "vcv_import_tests.rs"]
mod tests;
