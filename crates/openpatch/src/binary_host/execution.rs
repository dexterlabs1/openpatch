//! Control-thread permission for the provisional full-document delayed host.
//!
//! A grant is neither a running companion nor an Exact/Partial playback mode.
//! The session must revoke it when an incarnation is retired; a matching hash
//! alone cannot distinguish removal followed by recreation of an identical node.
use super::graph::HostedGraphPlan;
use crate::project_document::{DocumentNode, ProjectDocument};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

// The built-in catalog is immutable for the process lifetime. Reconstructing
// every module's complete schema for each permission comparison made a single
// knob edit perform several full catalog builds on the control thread.
fn parameter_catalog() -> &'static Value {
    static CATALOG: OnceLock<Value> = OnceLock::new();
    CATALOG.get_or_init(openpatch_core::catalog)
}

/// The sole provisional profile. Device deadline qualification remains separate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedExecutionProfile {
    Offset48kQ128D384Max128,
    XFade48kQ128D384Max128,
}

impl HostedExecutionProfile {
    pub fn for_capability(capability: crate::project_document::hosted::HostedCapability) -> Self {
        match capability {
            crate::project_document::hosted::HostedCapability::Offset => {
                Self::Offset48kQ128D384Max128
            }
            crate::project_document::hosted::HostedCapability::XFade => {
                Self::XFade48kQ128D384Max128
            }
        }
    }
    pub const SAMPLE_RATE_HZ: u32 = super::SAMPLE_RATE;
    pub const QUANTUM_FRAMES: usize = super::QUANTUM;
    pub const TRANSPORT_DELAY_FRAMES: usize = super::runner::TRANSPORT_DELAY_FRAMES;
    pub const MAX_CALLBACK_FRAMES: usize = super::runner::MAX_CALLBACK_FRAMES;

    pub const fn sample_rate_hz(self) -> u32 {
        Self::SAMPLE_RATE_HZ
    }
    pub const fn quantum_frames(self) -> usize {
        Self::QUANTUM_FRAMES
    }
    pub const fn transport_delay_frames(self) -> usize {
        Self::TRANSPORT_DELAY_FRAMES
    }
    pub const fn max_callback_frames(self) -> usize {
        Self::MAX_CALLBACK_FRAMES
    }
}

/// Lowercase SHA-256 over a versioned, canonical execution projection.
/// Construction is control-thread work; ordinary knob values are not topology.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct HostedTopologyKey(String);

impl HostedTopologyKey {
    pub fn new(
        document: &ProjectDocument,
        profile: HostedExecutionProfile,
    ) -> Result<Self, String> {
        let plan = HostedGraphPlan::prepare(document)?;
        if profile != HostedExecutionProfile::for_capability(plan.module().capability()?) {
            return Err("hosted execution profile differs from document capability".into());
        }
        Self::project(document, profile)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn project(
        document: &ProjectDocument,
        profile: HostedExecutionProfile,
    ) -> Result<Self, String> {
        let catalog = parameter_catalog();
        let mut graph = serde_json::to_value(&document.graph).map_err(|e| e.to_string())?;
        // Project the typed graph so newly added opaque fields are included by
        // default. Only explicitly understood parameter/presentation fields go.
        for (entry, node) in graph["nodes"]
            .as_array_mut()
            .ok_or("missing graph nodes")?
            .iter_mut()
            .zip(&document.graph.nodes)
        {
            entry
                .as_object_mut()
                .ok_or("invalid graph node")?
                .remove("source");
            match node {
                DocumentNode::Native { node, .. } => {
                    let schema = catalog["modules"].as_array().and_then(|modules| {
                        modules.iter().find(|module| module["kind"] == node.kind)
                    });
                    let params = entry["node"]["params"]
                        .as_object_mut()
                        .ok_or("invalid native parameters")?;
                    params.retain(|key, _| {
                        structural_parameter(&node.kind, key)
                            || schema.and_then(|s| s["params"].get(key)).is_none()
                    });
                    // Omitted structural controls have their declared default.
                    if let Some(specs) = schema.and_then(|s| s["params"].as_object()) {
                        for (key, spec) in specs {
                            if structural_parameter(&node.kind, key) {
                                if let Some(default) = spec.get("default") {
                                    params.entry(key.clone()).or_insert_with(|| {
                                        default
                                            .as_f64()
                                            .map(|value| json!(value as f32))
                                            .unwrap_or_else(|| default.clone())
                                    });
                                }
                            }
                        }
                    }
                }
                DocumentNode::Hosted { module, .. } => {
                    // Keep the qualified model, parameter IDs/order/cardinality
                    // and every custom field. Only declared live values vary.
                    let parameter_count = module.capability()?.parameters().len() as u64;
                    for param in entry["module"]["state"]["params"]
                        .as_array_mut()
                        .ok_or("invalid hosted parameters")?
                    {
                        if param["id"].as_u64().is_some_and(|id| id < parameter_count) {
                            param
                                .as_object_mut()
                                .ok_or("invalid hosted parameter")?
                                .remove("value");
                        }
                    }
                }
                DocumentNode::Unresolved { .. } => {
                    return Err("unresolved modules prevent hosted execution permission".into())
                }
            }
        }
        for cable in graph["cables"]
            .as_array_mut()
            .ok_or("missing graph cables")?
        {
            let object = cable.as_object_mut().ok_or("invalid graph cable")?;
            object.remove("source");
            // Bind all route ordering metadata conservatively; color alone is
            // presentation and cannot change the granted execution topology.
            if let Some(route) = object.get_mut("route").and_then(Value::as_object_mut) {
                route.remove("color");
            }
        }
        let projection = json!({
            "contract": "openpatch:hosted-execution-topology:v1",
            "document_format": document.format,
            "document_version": document.version,
            "profile": {
                "name": profile,
                "sample_rate_hz": profile.sample_rate_hz(),
                "quantum_frames": profile.quantum_frames(),
                "transport_delay_frames": profile.transport_delay_frames(),
                "max_callback_frames": profile.max_callback_frames(),
            },
            "graph": graph,
        });
        let bytes = serde_json::to_vec(&canonical(projection)).map_err(|e| e.to_string())?;
        Ok(Self(format!("{:x}", Sha256::digest(bytes))))
    }
}

// Channel/voice configuration changes the execution boundary. Unknown params
// are retained above, even when numeric: they are not proven ordinary knobs.
fn structural_parameter(kind: &str, key: &str) -> bool {
    matches!(
        key,
        "channels" | "width" | "voices" | "voice_count" | "polyphony" | "seed" | "steps"
    ) || (matches!(kind, "midi_cv" | "midi_cc" | "midi_gate")
        && matches!(key, "channel" | "allocation"))
}

fn canonical(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = object.into_iter().collect();
            entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(k, v)| (k, canonical(v)))
                    .collect(),
            )
        }
        Value::Array(array) => Value::Array(array.into_iter().map(canonical).collect()),
        other => other,
    }
}

/// Session-local explicit permission, bound to one document load generation.
/// Private fields and absence of Deserialize prevent restoring permission from
/// a project, history snapshot, or a caller-authored hash/profile tuple.
#[derive(Clone, Debug)]
pub struct HostedExecutionGrant {
    document_generation: u64,
    topology_key: HostedTopologyKey,
    profile: HostedExecutionProfile,
}

impl HostedExecutionGrant {
    pub fn new(document: &ProjectDocument, document_generation: u64) -> Result<Self, String> {
        let plan = HostedGraphPlan::prepare(document)?;
        let profile = HostedExecutionProfile::for_capability(plan.module().capability()?);
        let topology_key = HostedTopologyKey::new(document, profile)?;
        Ok(Self {
            document_generation,
            topology_key,
            profile,
        })
    }

    pub fn document_generation(&self) -> u64 {
        self.document_generation
    }
    pub fn topology_key(&self) -> &HostedTopologyKey {
        &self.topology_key
    }
    pub fn profile(&self) -> HostedExecutionProfile {
        self.profile
    }

    pub fn matches_document(
        &self,
        document: &ProjectDocument,
        document_generation: u64,
    ) -> Result<bool, String> {
        if document_generation != self.document_generation {
            return Ok(false);
        }
        Ok(HostedTopologyKey::new(document, self.profile)? == self.topology_key)
    }

    pub fn prepare(
        &self,
        document: &ProjectDocument,
        document_generation: u64,
    ) -> Result<HostedGraphPlan, String> {
        if document_generation != self.document_generation {
            return Err(
                "hosted execution permission belongs to another document generation".into(),
            );
        }
        let plan = HostedGraphPlan::prepare(document)?;
        if HostedTopologyKey::project(document, self.profile)? != self.topology_key {
            return Err(
                "hosted execution permission no longer matches the document topology".into(),
            );
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_document::hosted::HostedState;

    #[test]
    fn knobs_and_layout_keep_permission_but_a_fresh_load_does_not() {
        let mut document = super::super::graph::tests::document();
        let grant = HostedExecutionGrant::new(&document, 7).unwrap();
        if let DocumentNode::Native { node, .. } = &mut document.graph.nodes[0] {
            node.params.insert("value".into(), 0.75);
        }
        if let DocumentNode::Hosted { module, .. } = &mut document.graph.nodes[2] {
            *module = module.with_parameter("offset", 0.25).unwrap();
        }
        document.layout.zoom = 2.0;
        document
            .layout
            .positions
            .insert("offset".into(), [200.0, 50.0]);
        document.graph.cables[0].route.color = Some([1, 2, 3]);
        assert!(grant.matches_document(&document, 7).unwrap());
        assert!(!grant.matches_document(&document, 8).unwrap());
        assert!(grant.prepare(&document, 8).is_err());
        let plan = grant.prepare(&document, 7).unwrap();
        assert_eq!(plan.native_patch().nodes[0].params["value"], 0.75);
        assert_eq!(plan.module().parameter("offset").unwrap(), 0.25);
        assert!(document
            .project(crate::project_document::PlaybackMode::Exact)
            .is_err());
        assert_eq!(grant.topology_key().as_str().len(), 64);
    }

    #[test]
    fn topology_identity_custom_state_and_structural_controls_invalidate() {
        let original = super::super::graph::tests::document();
        let grant = HostedExecutionGrant::new(&original, 1).unwrap();
        for change in 0..9 {
            let mut document = original.clone();
            match change {
                0 => document.graph.nodes.swap(0, 1),
                1 => document.graph.output = Some("dc".into()),
                2 => document.graph.cables[1].route.from = "dc".into(),
                3 => {
                    if let DocumentNode::Native { node, .. } = &mut document.graph.nodes[0] {
                        node.bypassed = true;
                    }
                }
                4 => {
                    if let DocumentNode::Hosted { module, .. } = &mut document.graph.nodes[2] {
                        module.identity.plugin_sha256 = "a".repeat(64);
                    }
                }
                5 => {
                    if let DocumentNode::Hosted { module, .. } = &mut document.graph.nodes[2] {
                        *module = module.with_option("offset_first", false).unwrap();
                    }
                }
                6 => {
                    if let DocumentNode::Native { node, .. } = &mut document.graph.nodes[0] {
                        node.kind = "poly_constant".into();
                        node.params.clear();
                        node.params.insert("channels".into(), 2.0);
                    }
                }
                7 => document.graph.cables[0].route.input_order = Some(1),
                8 => {
                    if let DocumentNode::Hosted { module, .. } = &mut document.graph.nodes[2] {
                        module.ports.swap(0, 1);
                    }
                }
                _ => unreachable!(),
            }
            assert!(
                !grant.matches_document(&document, 1).unwrap(),
                "change {change}"
            );
            assert!(grant.prepare(&document, 1).is_err());
        }
        let mut mixer = original;
        if let DocumentNode::Native { node, .. } = &mut mixer.graph.nodes[1] {
            node.kind = "studio_mixer".into();
            node.params.clear();
        }
        mixer.graph.cables[1].route.input = "in1".into();
        mixer.graph.output_port = "left".into();
        let grant = HostedExecutionGrant::new(&mixer, 1).unwrap();
        if let DocumentNode::Native { node, .. } = &mut mixer.graph.nodes[1] {
            node.params.insert("channels".into(), 8.0);
        }
        assert!(grant.matches_document(&mixer, 1).unwrap());
        if let DocumentNode::Native { node, .. } = &mut mixer.graph.nodes[1] {
            node.params.insert("channels".into(), 4.0);
        }
        assert!(!grant.matches_document(&mixer, 1).unwrap());
    }

    #[test]
    fn unsupported_or_opaque_snapshots_and_stacked_routes_never_get_a_grant() {
        let original = super::super::graph::tests::document();
        let mut opaque = original.clone();
        if let DocumentNode::Hosted { module, .. } = &mut opaque.graph.nodes[2] {
            let mut state = module.state.as_ref().clone();
            state["opaque"] = json!({"future": 1});
            module.state = HostedState::new(state).unwrap();
        }
        assert!(HostedExecutionGrant::new(&opaque, 1).is_err());
        let mut stacked = original.clone();
        let mut cable = stacked.graph.cables[0].clone();
        cable.route.id = Some("second-input".into());
        stacked.graph.cables.push(cable);
        assert!(HostedExecutionGrant::new(&stacked, 1).is_err());
        let mut other_rate = original;
        other_rate.graph.sample_rate = 44_100;
        assert!(HostedExecutionGrant::new(&other_rate, 1).is_err());
    }

    #[test]
    fn full_automation_content_is_bound_even_when_ordinary_knobs_are_not() {
        let mut document = super::super::graph::tests::document();
        document.graph.nodes.push(DocumentNode::Native {
            node: serde_json::from_value(json!({
                "id": "score", "kind": "score_sequencer", "params": {},
                "score": {"lanes": [{"points": [{"bar": 0, "value": 0.25}]}]}
            }))
            .unwrap(),
            source: None,
        });
        let grant = HostedExecutionGrant::new(&document, 1).unwrap();
        if let DocumentNode::Native { node, .. } = &mut document.graph.nodes[3] {
            node.score.as_mut().unwrap().lanes[0].points[0].value = 0.75;
        }
        assert!(!grant.matches_document(&document, 1).unwrap());
        assert!(grant.prepare(&document, 1).is_err());
    }
}
