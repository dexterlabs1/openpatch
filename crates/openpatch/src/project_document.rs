//! Shared persisted document. Unsupported source content belongs here, never in
//! hidden DSP parameters. A Core Patch is a derived, explicitly qualified view.
use base64::{engine::general_purpose::STANDARD, Engine as _};
use openpatch_core::{
    Cable, Node, Patch, WaveAssets, MAX_CABLES, MAX_NODES, MAX_PATCH_ASSETS, MAX_PATCH_ASSET_BYTES,
    MAX_PATCH_FILE_BYTES,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

mod automation;
pub mod hosted;
pub use automation::{AutomationOmissionReason, OmittedAutomation};

pub const DOCUMENT_VERSION: u32 = 3;
const MAX_SOURCE_BLOB_BYTES: usize = 16 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 32 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct SourceBlob(Arc<Vec<u8>>);

impl SourceBlob {
    pub fn new(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > MAX_SOURCE_BLOB_BYTES {
            return Err("source asset exceeds 16 MiB".into());
        }
        Ok(Self(Arc::new(bytes)))
    }
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }
    pub fn id(&self) -> String {
        format!("{:x}", Sha256::digest(self.bytes()))
    }
}
impl Serialize for SourceBlob {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(self.bytes()))
    }
}
impl<'de> Deserialize<'de> for SourceBlob {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() > MAX_SOURCE_BLOB_BYTES.div_ceil(3) * 4 {
            return Err(serde::de::Error::custom("source asset exceeds 16 MiB"));
        }
        let bytes = STANDARD.decode(encoded).map_err(serde::de::Error::custom)?;
        Self::new(bytes).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcePackage {
    /// Complete original JSON, including unknown global/module/cable state.
    pub patch: Arc<Value>,
    /// Original relative archive names address immutable content by digest.
    pub assets: BTreeMap<String, String>,
    #[serde(default)]
    pub missing_assets: BTreeSet<String>,
}
impl SourcePackage {
    pub fn id(&self) -> Result<String, String> {
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceModule {
    pub package: String,
    /// Index into the immutable package's original modules array. A duplicate
    /// gets a fresh native ID and retains this original provenance.
    pub index: usize,
    pub mapping: Option<SourceMapping>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMapping {
    pub id: String,
    pub version: u32,
    pub fidelity: MappingFidelity,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MappingFidelity {
    SourceBehavior,
    MusicalRole,
    UserSubstitution,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum PortDirection {
    Input,
    Output,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedPort {
    pub direction: PortDirection,
    pub index: u32,
}
impl ObservedPort {
    pub fn key(&self) -> String {
        let direction = match self.direction {
            PortDirection::Input => "in",
            PortDirection::Output => "out",
        };
        format!("vcv_{direction}_{}", self.index)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DocumentNode {
    Native {
        node: Node,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<SourceModule>,
    },
    Unresolved {
        id: String,
        source: SourceModule,
        /// Only observed incident ports; this is not an invented plugin manifest.
        ports: Vec<ObservedPort>,
    },
    Hosted {
        id: String,
        module: hosted::HostedModule,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<SourceModule>,
    },
}
impl DocumentNode {
    pub fn id(&self) -> &str {
        match self {
            Self::Native { node, .. } => &node.id,
            Self::Unresolved { id, .. } | Self::Hosted { id, .. } => id,
        }
    }
    pub fn source(&self) -> Option<&SourceModule> {
        match self {
            Self::Native { source, .. } | Self::Hosted { source, .. } => source.as_ref(),
            Self::Unresolved { source, .. } => Some(source),
        }
    }
    pub fn is_native(&self) -> bool {
        matches!(self, Self::Native { .. })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentCable {
    pub route: Cable,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceCable>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCable {
    pub package: String,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectLayout {
    pub version: u32,
    pub zoom: f32,
    pub pan: [f32; 2],
    pub positions: BTreeMap<String, [f32; 2]>,
}
impl Default for ProjectLayout {
    fn default() -> Self {
        Self {
            version: 1,
            zoom: 1.0,
            pan: [0.0; 2],
            positions: BTreeMap::new(),
        }
    }
}

impl ProjectLayout {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let layout = self;
        if layout.version != 1
            || !layout.zoom.is_finite()
            || !(0.25..=4.0).contains(&layout.zoom)
            || layout.positions.len() > MAX_NODES
            || layout
                .pan
                .iter()
                .any(|p| !p.is_finite() || p.abs() > 1_000_000.0)
        {
            return Err("invalid document layout".into());
        }
        for (id, position) in &layout.positions {
            // Legacy layout accepted any bounded nonempty ID, including stale
            // positions. Do not silently tighten that migration contract.
            if id.is_empty()
                || id.len() > 64
                || position
                    .iter()
                    .any(|p| !p.is_finite() || p.abs() > 100_000.0)
            {
                return Err("invalid document module position".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentGraph {
    /// Core scheduling version, independent of document and mapping versions.
    pub version: u32,
    pub sample_rate: u32,
    #[serde(default)]
    pub assets: WaveAssets,
    pub nodes: Vec<DocumentNode>,
    pub cables: Vec<DocumentCable>,
    pub output: Option<String>,
    pub output_port: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDocument {
    pub format: String,
    pub version: u32,
    pub graph: DocumentGraph,
    pub layout: ProjectLayout,
    #[serde(default)]
    pub sources: BTreeMap<String, Arc<SourcePackage>>,
    #[serde(default)]
    pub source_blobs: BTreeMap<String, SourceBlob>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackMode {
    Exact,
    /// An explicit caller choice, never the default for missing dependencies.
    Partial,
}

/// Permission accompanies a prepared graph; speaker mute is independent.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ExecutionDisposition {
    #[default]
    RunnableExact = 0,
    RunnablePartial = 1,
    SuspendedUnresolved = 2,
}
impl ExecutionDisposition {
    pub fn is_runnable(self) -> bool {
        self != Self::SuspendedUnresolved
    }
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct OmittedRoute {
    pub cable_id: String,
    pub from: String,
    pub to: String,
}
/// Validated editable native state. This carries no permission to execute a
/// document whose omitted source modules may change musical behavior.
#[derive(Clone, Debug)]
pub struct NativeStateCarrier {
    pub patch: Patch,
    pub unresolved_modules: Vec<String>,
    pub omitted_routes: Vec<OmittedRoute>,
    pub omitted_output: Option<String>,
    pub omitted_automation: Vec<OmittedAutomation>,
}
#[derive(Clone, Debug)]
pub struct RuntimeProjection {
    pub patch: Patch,
    pub unresolved_modules: Vec<String>,
    pub omitted_routes: Vec<OmittedRoute>,
    pub omitted_output: Option<String>,
    pub omitted_automation: Vec<OmittedAutomation>,
}

// Anchor opaque entries before the next surviving native entry. Native order
// remains exactly the executable candidate's order (including engine-v1
// scheduling); a state capture does not regroup imported nodes or cables.
fn overlay_opaque_order<T: Clone>(
    original: &[T],
    native: Vec<T>,
    key: impl Fn(&T) -> String,
    opaque: impl Fn(&T) -> bool,
) -> Vec<T> {
    let ids: BTreeSet<_> = native.iter().map(&key).collect();
    let mut before = BTreeMap::new();
    let mut pending = Vec::new();
    for entry in original {
        if opaque(entry) {
            pending.push(entry.clone());
        } else if ids.contains(&key(entry)) {
            before.insert(key(entry), std::mem::take(&mut pending));
        }
    }
    let mut result = Vec::with_capacity(native.len() + original.len());
    for entry in native {
        if let Some(opaque) = before.remove(&key(&entry)) {
            result.extend(opaque);
        }
        result.push(entry);
    }
    result.extend(pending);
    result
}

impl ProjectDocument {
    pub fn from_patch(mut patch: Patch, layout: ProjectLayout) -> Result<Self, String> {
        patch.assign_missing_cable_ids()?;
        let document = Self {
            format: "openpatch-project".into(),
            version: DOCUMENT_VERSION,
            graph: DocumentGraph {
                version: patch.version,
                sample_rate: patch.sample_rate,
                assets: patch.assets,
                nodes: patch
                    .nodes
                    .into_iter()
                    .map(|node| DocumentNode::Native { node, source: None })
                    .collect(),
                cables: patch
                    .cables
                    .into_iter()
                    .map(|route| DocumentCable {
                        route,
                        source: None,
                    })
                    .collect(),
                output: patch.output,
                output_port: patch.output_port,
            },
            layout,
            sources: BTreeMap::new(),
            source_blobs: BTreeMap::new(),
        };
        document.validate_structure()?;
        Ok(document)
    }

    /// Migrate an existing envelope or native patch without changing scheduling.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_PATCH_FILE_BYTES {
            return Err("project exceeds 64 MiB".into());
        }
        let value: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let document = if value.get("format").is_some() {
            if value["format"] != "openpatch-project" {
                return Err("unknown project format".into());
            }
            match value["version"].as_u64() {
                Some(1) => {
                    #[derive(Deserialize)]
                    #[serde(deny_unknown_fields)]
                    struct Legacy {
                        format: String,
                        version: u32,
                        patch: Patch,
                        layout: ProjectLayout,
                    }
                    let old: Legacy = serde_json::from_value(value).map_err(|e| e.to_string())?;
                    debug_assert_eq!((old.format.as_str(), old.version), ("openpatch-project", 1));
                    Self::from_patch(old.patch, old.layout)?
                }
                Some(2) => {
                    let mut old: Self = serde_json::from_value(value).map_err(|e| e.to_string())?;
                    if old
                        .graph
                        .nodes
                        .iter()
                        .any(|node| matches!(node, DocumentNode::Hosted { .. }))
                    {
                        return Err("hosted modules require project document version 3".into());
                    }
                    old.version = DOCUMENT_VERSION;
                    old
                }
                Some(3) => serde_json::from_value(value).map_err(|e| e.to_string())?,
                _ => return Err("unsupported project document version".into()),
            }
        } else {
            Self::from_patch(
                serde_json::from_value(value).map_err(|e| e.to_string())?,
                ProjectLayout::default(),
            )?
        };
        document.validate_structure()?;
        Ok(document)
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.validate_structure()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_PATCH_FILE_BYTES {
            return Err("project exceeds 64 MiB".into());
        }
        Ok(bytes)
    }

    /// Legacy patch APIs cannot silently strip provenance or opaque source data.
    /// Runtime callers use project(); persistence callers save the full document.
    pub fn native_patch_for_legacy_api(&self) -> Result<Patch, String> {
        if !self.sources.is_empty() || !self.source_blobs.is_empty() {
            return Err(
                "native patch export would discard source content; save the full project document"
                    .into(),
            );
        }
        Ok(self.project(PlaybackMode::Exact)?.patch)
    }

    pub fn source_module(&self, source: &SourceModule) -> Result<&Value, String> {
        self.sources
            .get(&source.package)
            .and_then(|package| package.patch["modules"].as_array())
            .and_then(|modules| modules.get(source.index))
            .filter(|module| module.is_object())
            .ok_or_else(|| "missing original source module".into())
    }

    /// Inspection deliberately excludes opaque source JSON and asset payloads.
    /// Its distinct format cannot be mistaken for a self-contained saved file.
    pub fn reference_view(&self) -> Value {
        let assets: BTreeMap<_, _> = self
            .graph
            .assets
            .iter()
            .map(|(id, asset)| (id, asset.descriptor()))
            .collect();
        let sources: BTreeMap<_, _> = self
            .sources
            .iter()
            .map(|(id, package)| {
                (
                    id,
                    serde_json::json!({
                        "rack_version":package.patch["version"],
                        "modules":package.patch["modules"].as_array().map_or(0, Vec::len),
                        "assets":package.assets,"missing_assets":package.missing_assets
                    }),
                )
            })
            .collect();
        let blobs: BTreeMap<_, _> = self
            .source_blobs
            .iter()
            .map(|(id, blob)| (id, serde_json::json!({"bytes":blob.bytes().len()})))
            .collect();
        serde_json::json!({
            "format":"openpatch-project-reference","version":self.version,
            "graph":{"version":self.graph.version,"sample_rate":self.graph.sample_rate,
                "nodes":self.graph.nodes.iter().map(|node|match node {
                    DocumentNode::Hosted{id,module,source}=>json!({"type":"hosted","id":id,"module":{
                        "schema_version":module.schema_version,"identity":module.identity,"ports":module.ports,
                        "state_reference":{"sha256":module.state.id(),"bytes":module.state.encoded_len()}
                    },"source":source}),
                    _=>json!(node),
                }).collect::<Vec<_>>(),"cables":self.graph.cables,
                "output":self.graph.output,"output_port":self.graph.output_port,"assets":assets},
            "layout":self.layout,"sources":sources,"source_blobs":blobs
        })
    }

    /// Validate recoverable document structure without claiming DSP support.
    pub fn validate_structure(&self) -> Result<(), String> {
        if self.format != "openpatch-project" || self.version != DOCUMENT_VERSION {
            return Err("unsupported project document version or format".into());
        }
        if !matches!(self.graph.version, 1 | 2)
            || !(8000..=192000).contains(&self.graph.sample_rate)
        {
            return Err("invalid graph scheduling version or sample rate".into());
        }
        if self.graph.nodes.len() > MAX_NODES || self.graph.cables.len() > MAX_CABLES {
            return Err("document exceeds module or cable limit".into());
        }
        self.validate_layout()?;
        self.validate_sources()?;
        let mut native_asset_bytes = 0usize;
        if self.graph.assets.len() > MAX_PATCH_ASSETS
            || (self.graph.version != 2 && !self.graph.assets.is_empty())
        {
            return Err("invalid native asset count or graph version".into());
        }
        for (id, asset) in &self.graph.assets {
            native_asset_bytes = native_asset_bytes
                .checked_add(asset.decoded_bytes())
                .ok_or("native asset byte count overflow")?;
            if id != asset.id() || native_asset_bytes > MAX_PATCH_ASSET_BYTES {
                return Err("invalid native asset identity or byte budget".into());
            }
        }
        let mut nodes = BTreeMap::new();
        let mut hosted_bytes = 0usize;
        for node in &self.graph.nodes {
            validate_id(node.id())?;
            if nodes.insert(node.id(), node).is_some() {
                return Err("duplicate document module ID".into());
            }
            if let Some(source) = node.source() {
                self.source_module(source)?;
                if matches!(
                    node,
                    DocumentNode::Native { .. } | DocumentNode::Hosted { .. }
                ) && source.mapping.is_none()
                {
                    return Err(if node.is_native() {
                        "native source module needs an explicit mapping"
                    } else {
                        "hosted source module needs an explicit mapping"
                    }
                    .into());
                }
                if let Some(mapping) = &source.mapping {
                    if mapping.id.is_empty() || mapping.version == 0 {
                        return Err("invalid source mapping identity".into());
                    }
                }
            }
            if let DocumentNode::Unresolved { source, ports, .. } = node {
                if source.mapping.is_some() {
                    return Err("unresolved module cannot claim a native mapping".into());
                }
                let unique: BTreeSet<_> = ports.iter().map(|p| (p.direction, p.index)).collect();
                if unique.len() != ports.len() || ports.len() > MAX_CABLES * 2 {
                    return Err("duplicate or excessive observed source ports".into());
                }
            }
            if let DocumentNode::Hosted { module, .. } = node {
                module.validate_structure()?;
                hosted_bytes = hosted_bytes
                    .checked_add(module.state.encoded_len())
                    .ok_or("hosted state byte budget overflow")?;
                if hosted_bytes > MAX_SOURCE_BYTES {
                    return Err("hosted state exceeds 32 MiB document budget".into());
                }
            }
        }
        let mut cables = BTreeSet::new();
        for cable in &self.graph.cables {
            let route = &cable.route;
            let id = route
                .id
                .as_deref()
                .ok_or("document cables need stable IDs")?;
            validate_id(id)?;
            if !cables.insert(id) {
                return Err("duplicate document cable ID".into());
            }
            for (id, port, direction) in [
                (&route.from, &route.output, PortDirection::Output),
                (&route.to, &route.input, PortDirection::Input),
            ] {
                let node = nodes
                    .get(id.as_str())
                    .ok_or("document cable references a missing module")?;
                if let DocumentNode::Unresolved { ports, .. } = node {
                    if !ports
                        .iter()
                        .any(|p| p.direction == direction && p.key() == *port)
                    {
                        return Err("cable references an unobserved source port".into());
                    }
                }
                if let DocumentNode::Hosted { module, .. } = node {
                    if !module
                        .ports
                        .iter()
                        .any(|p| p.direction == direction && p.key == *port)
                    {
                        return Err("cable references an undeclared hosted port".into());
                    }
                }
            }
            if let Some(source) = &cable.source {
                let original = self
                    .sources
                    .get(&source.package)
                    .and_then(|p| p.patch["cables"].as_array())
                    .and_then(|c| c.get(source.index));
                if !original.is_some_and(Value::is_object) {
                    return Err("missing original source cable".into());
                }
            }
        }
        if self
            .graph
            .output
            .as_ref()
            .is_some_and(|id| !nodes.contains_key(id.as_str()))
        {
            return Err("document output references a missing module".into());
        }
        self.automation_omissions()?;
        Ok(())
    }

    fn validate_layout(&self) -> Result<(), String> {
        self.layout.validate()
    }

    fn validate_sources(&self) -> Result<(), String> {
        let mut bytes = 0usize;
        if self.sources.len() > MAX_NODES || self.source_blobs.len() > 1024 {
            return Err("too many source records or assets".into());
        }
        for (id, blob) in &self.source_blobs {
            if *id != blob.id() {
                return Err("source blob digest does not match bytes".into());
            }
            bytes = bytes
                .checked_add(blob.bytes().len())
                .ok_or("source byte count overflow")?;
        }
        for (id, package) in &self.sources {
            let encoded = serde_json::to_vec(package).map_err(|e| e.to_string())?;
            if *id != format!("{:x}", Sha256::digest(&encoded))
                || !package.patch["modules"].is_array()
                || package.assets.len() > 1024
            {
                return Err("invalid source package identity or modules".into());
            }
            bytes = bytes
                .checked_add(encoded.len())
                .ok_or("source byte count overflow")?;
            for (path, blob) in &package.assets {
                validate_asset_path(path)?;
                if !self.source_blobs.contains_key(blob) {
                    return Err("source asset blob is missing".into());
                }
            }
        }
        if bytes > MAX_SOURCE_BYTES {
            return Err("retained source content exceeds 32 MiB".into());
        }
        Ok(())
    }

    /// Replace the editable native subgraph while retaining immutable source
    /// packages and all unresolved content. Copy keys are fresh native IDs and
    /// their values identify original native nodes whose provenance is cloned.
    pub fn with_native_patch(
        &self,
        candidate: Patch,
        copies: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let prior = self.state_carrier()?;
        if candidate.version != self.graph.version
            && !(self.graph.version == 1 && candidate.version == 2 && !candidate.assets.is_empty())
        {
            return Err("native edit cannot change graph scheduling version".into());
        }
        candidate.validate()?;
        let original_nodes: BTreeMap<_, _> = self
            .graph
            .nodes
            .iter()
            .map(|node| (node.id(), node))
            .collect();
        let candidate_ids: BTreeSet<_> = candidate
            .nodes
            .iter()
            .map(|node| node.id.as_str())
            .collect();
        for node in &candidate.nodes {
            if matches!(
                original_nodes.get(node.id.as_str()),
                Some(DocumentNode::Unresolved { .. } | DocumentNode::Hosted { .. })
            ) {
                return Err("native module ID collides with an unresolved module".into());
            }
        }
        for (new_id, old_id) in copies {
            if original_nodes.contains_key(new_id.as_str())
                || !candidate_ids.contains(new_id.as_str())
                || !matches!(
                    original_nodes.get(old_id.as_str()),
                    Some(DocumentNode::Native { .. })
                )
            {
                return Err(
                    "copy provenance requires a fresh native ID and an existing native source"
                        .into(),
                );
            }
        }
        let original_cables: BTreeMap<_, _> = self
            .graph
            .cables
            .iter()
            .map(|cable| (cable.route.id.as_deref().expect("validated"), cable))
            .collect();
        let unresolved: BTreeSet<_> = prior
            .unresolved_modules
            .iter()
            .map(String::as_str)
            .collect();
        let mut native_cables = Vec::with_capacity(candidate.cables.len());
        for route in &candidate.cables {
            let id = route
                .id
                .as_deref()
                .ok_or("native edit cables need stable IDs")?;
            let original = original_cables.get(id);
            if original.is_some_and(|cable| {
                unresolved.contains(cable.route.from.as_str())
                    || unresolved.contains(cable.route.to.as_str())
            }) {
                return Err("native cable ID collides with an unresolved route".into());
            }
            native_cables.push(DocumentCable {
                route: route.clone(),
                source: original.and_then(|cable| cable.source.clone()),
            });
        }
        let mut document = self.clone();
        let native_nodes = candidate
            .nodes
            .iter()
            .map(|node| {
                let source_id = copies
                    .get(&node.id)
                    .map_or(node.id.as_str(), String::as_str);
                let source = original_nodes
                    .get(source_id)
                    .and_then(|node| node.source())
                    .cloned();
                DocumentNode::Native {
                    node: node.clone(),
                    source,
                }
            })
            .collect();
        document.graph.nodes = overlay_opaque_order(
            &self.graph.nodes,
            native_nodes,
            |node| node.id().to_owned(),
            |node| !node.is_native(),
        );
        // Retain opaque routes only while both endpoints survive. Keep their
        // position among native routes without changing candidate DSP ordering.
        document.graph.cables = overlay_opaque_order(
            &self.graph.cables,
            native_cables,
            |cable| cable.route.id.clone().expect("validated"),
            |cable| {
                let route = &cable.route;
                (unresolved.contains(route.from.as_str()) || unresolved.contains(route.to.as_str()))
                    && [route.from.as_str(), route.to.as_str()]
                        .iter()
                        .all(|id| unresolved.contains(id) || candidate_ids.contains(id))
            },
        );
        let preserve_output = prior.omitted_output.is_some()
            && candidate.output == prior.patch.output
            && candidate.output_port == prior.patch.output_port;
        if !preserve_output {
            document.graph.output = candidate.output;
            document.graph.output_port = candidate.output_port;
        }
        document.graph.version = candidate.version;
        document.graph.sample_rate = candidate.sample_rate;
        document.graph.assets = candidate.assets;
        document.layout.positions.retain(|id, _| {
            !matches!(
                original_nodes.get(id.as_str()),
                Some(DocumentNode::Native { .. })
            ) || candidate_ids.contains(id.as_str())
        });
        document.state_carrier()?;
        Ok(document)
    }

    /// Derive editable native state and report every omitted source dependency.
    /// Validation here does not authorize execution of an incomplete document.
    pub fn state_carrier(&self) -> Result<NativeStateCarrier, String> {
        self.validate_structure()?;
        let unresolved: BTreeSet<_> = self
            .graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                DocumentNode::Unresolved { id, .. } | DocumentNode::Hosted { id, .. } => {
                    Some(id.clone())
                }
                _ => None,
            })
            .collect();
        let mut omitted_routes = Vec::new();
        let cables = self
            .graph
            .cables
            .iter()
            .filter_map(|cable| {
                let route = &cable.route;
                if unresolved.contains(&route.from) || unresolved.contains(&route.to) {
                    omitted_routes.push(OmittedRoute {
                        cable_id: route.id.clone().expect("validated"),
                        from: route.from.clone(),
                        to: route.to.clone(),
                    });
                    None
                } else {
                    Some(route.clone())
                }
            })
            .collect();
        let omitted_output = self
            .graph
            .output
            .as_ref()
            .filter(|id| unresolved.contains(*id))
            .cloned();
        let patch = Patch {
            version: self.graph.version,
            sample_rate: self.graph.sample_rate,
            assets: self.graph.assets.clone(),
            nodes: self
                .graph
                .nodes
                .iter()
                .filter_map(|node| match node {
                    DocumentNode::Native { node, .. } => Some(node.clone()),
                    _ => None,
                })
                .collect(),
            cables,
            output: self
                .graph
                .output
                .clone()
                .filter(|_| omitted_output.is_none()),
            output_port: self.graph.output_port.clone(),
        };
        patch.validate()?;
        Ok(NativeStateCarrier {
            patch,
            unresolved_modules: unresolved.into_iter().collect(),
            omitted_routes,
            omitted_output,
            omitted_automation: self.automation_omissions()?,
        })
    }

    /// Partial playback omits unresolved nodes and their incident cables. This
    /// can change connected-port normaling; omissions must be shown to the user.
    pub fn project(&self, mode: PlaybackMode) -> Result<RuntimeProjection, String> {
        let carrier = self.state_carrier()?;
        if mode == PlaybackMode::Exact
            && (!carrier.unresolved_modules.is_empty() || !carrier.omitted_automation.is_empty())
        {
            return Err(format!(
                "{} unresolved modules and {} omitted automation lanes prevent exact playback",
                carrier.unresolved_modules.len(),
                carrier.omitted_automation.len()
            ));
        }
        Ok(RuntimeProjection {
            patch: carrier.patch,
            unresolved_modules: carrier.unresolved_modules,
            omitted_routes: carrier.omitted_routes,
            omitted_output: carrier.omitted_output,
            omitted_automation: carrier.omitted_automation,
        })
    }
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'))
    {
        return Err("invalid document identity".into());
    }
    Ok(())
}
fn validate_asset_path(path: &str) -> Result<(), String> {
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', '\0'])
        || path.as_bytes().get(1) == Some(&b':')
        || path.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        return Err("source asset needs a canonical relative archive path".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "project_document_tests.rs"]
mod tests;
