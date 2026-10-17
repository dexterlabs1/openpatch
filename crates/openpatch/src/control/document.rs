//! Portable document authority shared by the desktop and MCP session.
use super::*;

#[derive(Clone, Debug)]
pub struct ProjectDocumentSnapshot {
    pub document: ProjectDocument,
    pub revision: u64,
    pub state_id: u64,
    pub layout_generation: u64,
    /// Suspended documents contain authored/frozen persistent fields, not live observations.
    pub state_source: &'static str,
    pub frame: Option<u64>,
}

/// Camera motion is durable but unhistoried; automatic placement completes the
/// geometry of the current authored state without creating another undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutIntent {
    Camera,
    Placement,
}

/// Geometry supplied by a full replacement must be installed before host
/// reconciliation. Raw Rack coordinates additionally need native-width fitting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutOrigin {
    Authored,
    SourceImport,
}

pub(super) fn geometry_changed(before: &ProjectDocument, after: &ProjectDocument) -> bool {
    if before.graph.nodes.len() != after.graph.nodes.len() {
        return true;
    }
    after.graph.nodes.iter().any(|new| {
        let Some(old) = before.graph.nodes.iter().find(|old| old.id() == new.id()) else {
            return true;
        };
        match (old, new) {
            (DocumentNode::Native { node: old, .. }, DocumentNode::Native { node: new, .. }) => {
                old.kind != new.kind
                    || old.content != new.content
                    || (new.kind == "studio_mixer"
                        && old.params.get("channels") != new.params.get("channels"))
                    || ["width", "width_hp", "panel_width"]
                        .iter()
                        .any(|key| old.params.get(*key) != new.params.get(*key))
            }
            (
                DocumentNode::Unresolved { source: old, .. },
                DocumentNode::Unresolved { source: new, .. },
            ) => old.package != new.package || old.index != new.index,
            _ => true,
        }
    })
}

fn complete_host_layout(document: &ProjectDocument, layout: &ProjectLayout) -> Result<(), String> {
    layout.validate()?;
    if document
        .graph
        .nodes
        .iter()
        .any(|node| !layout.positions.contains_key(node.id()))
    {
        return Err(
            "host placement is missing document module positions; retry after rack placement"
                .into(),
        );
    }
    Ok(())
}

impl Session {
    /// Register the geometry-producing host. Initial placement must be published
    /// before capturing a complete project document.
    pub fn attach_layout_host(&mut self) -> Result<(), String> {
        self.check_open()?;
        if !self.layout_host_attached {
            let generation = self
                .layout_generation
                .checked_add(1)
                .ok_or("layout generation exhausted")?;
            self.layout_host_attached = true;
            self.layout_pending = true;
            self.layout_generation = generation;
        }
        Ok(())
    }

    pub fn layout_generation(&self) -> u64 {
        self.layout_generation
    }

    pub fn layout_pending(&self) -> bool {
        self.layout_pending
    }
    pub fn layout_origin(&self) -> Option<LayoutOrigin> {
        self.layout_origin
    }

    /// Publish host geometry under graph, history-state and layout guards. This
    /// path deliberately avoids graph compilation, source validation and audio.
    pub fn publish_host_layout(
        &mut self,
        expected_revision: u64,
        expected_state_id: u64,
        expected_layout_generation: u64,
        layout: ProjectLayout,
        intent: LayoutIntent,
    ) -> Result<Value, String> {
        self.check_open()?;
        self.check_revision(Some(expected_revision))?;
        if expected_state_id != self.state_id {
            return Err(format!(
                "stale layout state: expected {expected_state_id}, current {}",
                self.state_id
            ));
        }
        if expected_layout_generation != self.layout_generation {
            return Err(format!(
                "stale layout generation: expected {expected_layout_generation}, current {}",
                self.layout_generation
            ));
        }
        if !self.layout_host_attached {
            return Err("no layout host is attached".into());
        }
        layout.validate()?;
        match intent {
            LayoutIntent::Camera => {
                if self.layout_pending {
                    return Err("host layout placement is pending; retry camera publication after placement".into());
                }
                if layout.version != self.document.layout.version
                    || layout.positions != self.document.layout.positions
                {
                    return Err("camera publication may only change pan and zoom".into());
                }
            }
            LayoutIntent::Placement => complete_host_layout(&self.document, &layout)?,
        }
        let generation = self
            .layout_generation
            .checked_add(1)
            .ok_or("layout generation exhausted")?;
        self.document.layout = layout;
        self.layout_generation = generation;
        if intent == LayoutIntent::Placement {
            self.layout_pending = false;
            self.layout_origin = None;
            // Grouped edits can retain a shared history-state identity. Complete
            // its geometry everywhere that identity is retained.
            for entry in self.undo.iter_mut().chain(self.redo.iter_mut()) {
                if entry.state_id == self.state_id {
                    entry.document.layout = self.document.layout.clone();
                    entry.layout_origin = None;
                }
            }
        }
        Ok(
            json!({"revision":self.revision,"state_id":self.state_id,"layout_generation":self.layout_generation,"layout":self.document.layout}),
        )
    }

    pub fn project_document(&self) -> &ProjectDocument {
        &self.document
    }

    pub fn playback_mode(&self) -> PlaybackMode {
        self.playback_mode
    }

    pub fn execution_disposition(&self) -> ExecutionDisposition {
        disposition(&self.document, self.playback_mode)
    }

    pub(super) fn has_source_document(&self) -> bool {
        has_source(&self.document)
    }

    pub(super) fn require_native_document(&self) -> Result<(), String> {
        if self.has_source_document() {
            Err("this document retains Rack source data; use the full project document API to avoid losing it".into())
        } else {
            Ok(())
        }
    }

    pub(super) fn require_document_execution(&self) -> Result<(), String> {
        if self.execution_disposition().is_runnable() {
            Ok(())
        } else {
            Err("playback is suspended: unresolved modules or automation targets remain; repair dependencies or explicitly choose partial playback and review omitted routes and automation".into())
        }
    }

    pub(super) fn validate_document_backend(
        &self,
        document: &ProjectDocument,
        mode: PlaybackMode,
        backend: Option<&dyn LiveModuleEvents>,
    ) -> Result<(), String> {
        if (has_source(document) || mode == PlaybackMode::Partial)
            && backend.is_some_and(|backend| !backend.supports_document_execution())
        {
            return Err("audio backend cannot fence imported document execution".into());
        }
        Ok(())
    }

    pub(super) fn document_after_edit(
        &self,
        candidate: &Patch,
        effects: &EditEffects,
    ) -> Result<ProjectDocument, String> {
        if let Some(document) = &effects.replacement_document {
            let mut document = document.clone();
            // Fresh-instance intent can materialize an implicit live control
            // default. Carry exactly those defaults into editable native state;
            // the immutable Rack package and all other fields remain authored.
            for entry in &mut document.graph.nodes {
                let DocumentNode::Native { node, .. } = entry else {
                    continue;
                };
                let Some(receiving) = candidate
                    .nodes
                    .iter()
                    .find(|receiving| receiving.id == node.id && receiving.kind == node.kind)
                else {
                    continue;
                };
                for &key in openpatch_core::runtime_parameter_names(&node.kind) {
                    let default = openpatch_core::runtime_parameter_descriptor(&node.kind, key)
                        .expect("listed live parameter has a descriptor")
                        .default;
                    if !node.params.contains_key(key)
                        && receiving.params.get(key).copied() == Some(default)
                    {
                        node.params.insert(key.to_owned(), default);
                    }
                }
            }
            let carrier = document.state_carrier()?;
            if carrier.patch.reference_view() != candidate.reference_view()
                || carrier.patch.assets != candidate.assets
            {
                return Err("replacement document and native carrier disagree".into());
            }
            return Ok(document);
        }
        if !self.has_source_document() {
            let mut layout = self.document.layout.clone();
            for node in &self.patch.nodes {
                if !candidate.nodes.iter().any(|new| new.id == node.id) {
                    layout.positions.remove(&node.id);
                }
            }
            return ProjectDocument::from_patch(candidate.clone(), layout);
        }
        let copies = effects
            .source_copies
            .iter()
            .filter(|(new, _)| candidate.nodes.iter().any(|node| &node.id == *new))
            .map(|(new, old)| (new.clone(), old.clone()))
            .collect();
        let mut document = self
            .document
            .with_native_patch(candidate.clone(), &copies)?;
        // Reusing an ID after removal creates a new native instance. It must not
        // inherit an old source-behavior claim merely because the ID matches.
        for node in &mut document.graph.nodes {
            if let DocumentNode::Native { node, source } = node {
                if effects.retired.contains(&node.id)
                    && !effects.source_copies.contains_key(&node.id)
                {
                    *source = None;
                }
            }
        }
        let unresolved: BTreeSet<_> = document
            .graph
            .nodes
            .iter()
            .filter_map(|node| {
                matches!(node, DocumentNode::Unresolved { .. }).then_some(node.id().to_owned())
            })
            .collect();
        document.graph.cables.retain(|cable| {
            let opaque =
                unresolved.contains(&cable.route.from) || unresolved.contains(&cable.route.to);
            !opaque
                || (!effects.cleared_source_routes
                    && !effects.retired.contains(&cable.route.from)
                    && !effects.retired.contains(&cable.route.to))
        });
        document.validate_structure()?;
        Ok(document)
    }

    pub(super) fn check_source_duplicate_routes(
        &self,
        candidate: &Patch,
        name: &str,
        args: &Value,
    ) -> Result<(), String> {
        if name != "duplicate_modules" || !self.has_source_document() {
            return Ok(());
        }
        let mode = optional_string(args, "cable_mode", "none")?;
        if !matches!(mode, "inputs" | "all") {
            return Ok(());
        }
        let selected: BTreeSet<_> = args["modules"]
            .as_array()
            .ok_or("modules must be an array")?
            .iter()
            .map(|entry| string(entry, "id"))
            .collect::<Result<_, _>>()?;
        let native_ids: BTreeSet<_> = candidate
            .cables
            .iter()
            .filter_map(|cable| cable.id.as_deref())
            .collect();
        if self.document.graph.cables.iter().any(|cable| {
            !cable
                .route
                .id
                .as_deref()
                .is_some_and(|id| native_ids.contains(id))
                && (selected.contains(cable.route.to.as_str())
                    || (mode == "all" && selected.contains(cable.route.from.as_str())))
        }) {
            return Err("duplication touches unresolved routes; use cable_mode none or a full document edit rather than silently omitting them".into());
        }
        Ok(())
    }

    pub(super) fn assign_source_safe_cable_ids(
        &self,
        candidate: &mut Patch,
        name: &str,
        args: &Value,
    ) -> Result<(), String> {
        if !self.has_source_document()
            || !(name == "duplicate_modules"
                || (name == "connect" && args.get("cable_id").is_none()))
        {
            return Ok(());
        }
        let native_ids: BTreeSet<_> = self
            .patch
            .cables
            .iter()
            .filter_map(|route| route.id.as_deref())
            .collect();
        let reserved: BTreeSet<_> = self
            .document
            .graph
            .cables
            .iter()
            .filter_map(|route| route.route.id.as_deref())
            .filter(|id| !native_ids.contains(id))
            .collect();
        let mut used: BTreeSet<String> = candidate
            .cables
            .iter()
            .filter_map(|route| route.id.clone())
            .chain(reserved.iter().map(|id| (*id).to_owned()))
            .collect();
        let mut next = 1;
        for route in &mut candidate.cables {
            if route.id.as_deref().is_some_and(|id| reserved.contains(id)) {
                while used.contains(&format!("cable_{next}")) {
                    next += 1;
                }
                let id = format!("cable_{next}");
                used.insert(id.clone());
                route.id = Some(id);
            }
        }
        Ok(())
    }

    /// Explicit replacement preserves the complete document and starts its
    /// module instances fresh. Playback defaults to Exact on every load.
    pub fn replace_project_document(
        &mut self,
        document: ProjectDocument,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        self.replace_document_layout(document, expected_revision, false)
    }

    /// Install a host-prepared document and geometry in one atomic commit.
    pub fn replace_prepared_project_document(
        &mut self,
        document: ProjectDocument,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        self.replace_document_layout(document, expected_revision, true)
    }

    fn replace_document_layout(
        &mut self,
        document: ProjectDocument,
        expected_revision: Option<u64>,
        prepared_layout: bool,
    ) -> Result<u64, String> {
        self.check_open()?;
        self.check_revision(expected_revision)?;
        if prepared_layout {
            complete_host_layout(&document, &document.layout)?;
        }
        let carrier = document.state_carrier()?;
        self.validate_document_backend(
            &document,
            PlaybackMode::Exact,
            self.live_module_events.as_deref(),
        )?;
        self.reconcile_midi_cc_learn()?;
        let mut effects = EditEffects::default();
        effects.retired = self.module_ids();
        effects.retired_cables = self.cable_ids();
        effects.replacement_document = Some(document);
        effects.playback_mode = Some(PlaybackMode::Exact);
        let revision = self.commit_with_layout(
            carrier.patch,
            expected_revision,
            None,
            &effects,
            prepared_layout,
        )?;
        self.layout_origin = if prepared_layout {
            None
        } else {
            Some(LayoutOrigin::Authored)
        };
        Ok(revision)
    }

    pub fn new_prepared_project_document(
        &mut self,
        document: ProjectDocument,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        let revision = self.replace_prepared_project_document(document, expected_revision)?;
        self.undo.clear();
        self.redo.clear();
        self.history_group = None;
        Ok(revision)
    }

    pub fn new_project_document(
        &mut self,
        document: ProjectDocument,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        let revision = self.replace_project_document(document, expected_revision)?;
        self.undo.clear();
        self.redo.clear();
        self.history_group = None;
        Ok(revision)
    }

    /// A geometry checkpoint shares the same revision/history boundary as graph edits.
    pub fn record_project_layout(
        &mut self,
        layout: ProjectLayout,
        expected_revision: u64,
    ) -> Result<Value, String> {
        self.check_revision(Some(expected_revision))?;
        let mut document = self.document.clone();
        document.layout = layout;
        if self.layout_host_attached {
            complete_host_layout(&document, &document.layout)?;
        }
        document.validate_structure()?;
        let mut effects = EditEffects::default();
        effects.replacement_document = Some(document);
        self.commit_with_layout(
            self.patch.clone(),
            Some(expected_revision),
            None,
            &effects,
            true,
        )?;
        Ok(self.snapshot())
    }

    /// Permission is a history transaction, not an implicit missing-plugin fallback.
    pub fn set_document_playback(
        &mut self,
        mode: PlaybackMode,
        expected_revision: Option<u64>,
    ) -> Result<u64, String> {
        self.check_open()?;
        self.check_revision(expected_revision)?;
        if mode == self.playback_mode {
            return Ok(self.revision);
        }
        self.validate_document_backend(&self.document, mode, self.live_module_events.as_deref())?;
        let document = if !disposition(&self.document, mode).is_runnable()
            && self.execution_disposition().is_runnable()
        {
            // Capture persistent fields before fencing live observations. No
            // guessed reset state is ever presented as a frozen live snapshot.
            self.capture_document_state(expected_revision, false)?
                .document
        } else {
            self.document.clone()
        };
        let carrier = document.state_carrier()?;
        let mut effects = EditEffects::default();
        effects.replacement_document = Some(document);
        effects.playback_mode = Some(mode);
        self.commit(carrier.patch, expected_revision, None, &effects)
    }

    pub fn capture_project_document(
        &self,
        expected_revision: Option<u64>,
    ) -> Result<ProjectDocumentSnapshot, String> {
        self.capture_document_state(expected_revision, true)
    }

    fn capture_document_state(
        &self,
        expected_revision: Option<u64>,
        require_layout_ready: bool,
    ) -> Result<ProjectDocumentSnapshot, String> {
        self.check_open()?;
        self.check_revision(expected_revision)?;
        if require_layout_ready && self.layout_pending {
            return Err(
                "host layout placement is pending; retry document capture after placement".into(),
            );
        }
        if !self.execution_disposition().is_runnable() {
            return Ok(ProjectDocumentSnapshot {
                document: self.document.clone(),
                revision: self.revision,
                state_id: self.state_id,
                layout_generation: self.layout_generation,
                state_source: "suspended_persisted",
                frame: None,
            });
        }
        let captured = self.capture_native_project_patch(expected_revision)?;
        let document = if self.has_source_document() {
            self.document
                .with_native_patch(captured.patch, &BTreeMap::new())?
        } else {
            ProjectDocument::from_patch(captured.patch, self.document.layout.clone())?
        };
        Ok(ProjectDocumentSnapshot {
            document,
            revision: captured.revision,
            state_id: captured.state_id,
            layout_generation: self.layout_generation,
            state_source: captured.state_source,
            frame: captured.frame,
        })
    }

    // Preflight a history crossing before popping either stack or fencing audio.
    // Surviving incarnations keep their live persistent fields; explicit history
    // assignments still win over those fields.
    pub(super) fn prepare_history_suspension(
        &self,
        target: &mut HistoryEntry,
    ) -> Result<Option<ProjectDocument>, String> {
        if !self.execution_disposition().is_runnable()
            || disposition(&target.document, target.playback_mode).is_runnable()
        {
            return Ok(None);
        }
        let captured = self
            .capture_document_state(Some(self.revision), false)?
            .document;
        let live = captured.state_carrier()?.patch;
        for node in &mut target.patch.nodes {
            if !target.retired_modules.contains(&node.id) {
                if let Some(current) = live
                    .nodes
                    .iter()
                    .find(|current| current.id == node.id && current.kind == node.kind)
                {
                    if let Some(state) = SavedModuleState::from_node(current) {
                        state.write_params(&mut node.params);
                    }
                    for &key in openpatch_core::runtime_parameter_names(&current.kind) {
                        if let Some(value) = current.params.get(key) {
                            node.params.insert(key.to_owned(), *value);
                        }
                    }
                }
            }
            if let Some(assignment) = target.restore_saved.get(&node.id) {
                assignment.write_params(&mut node.params);
            }
        }
        target.document = if has_source(&target.document) {
            target
                .document
                .with_native_patch(target.patch.clone(), &BTreeMap::new())?
        } else {
            ProjectDocument::from_patch(target.patch.clone(), target.document.layout.clone())?
        };
        Ok(Some(captured))
    }

    pub fn document_execution_status(&self) -> Value {
        execution_status(&self.document, self.playback_mode)
    }

    pub(super) fn document_tool(
        &mut self,
        name: &str,
        args: &Value,
    ) -> Option<Result<Value, String>> {
        if !matches!(
            name,
            "inspect_document"
                | "replace_document"
                | "load_document"
                | "save_document"
                | "set_document_playback"
        ) {
            return None;
        }
        Some((|| match name {
            "inspect_document" => Ok(
                json!({"revision":self.revision, "state_id":self.state_id, "layout_generation":self.layout_generation, "layout_pending":self.layout_pending, "document": if args["include_source_data"] == true { serde_json::to_value(&self.document).map_err(|e|e.to_string())? } else { self.document.reference_view() }, "execution": self.document_execution_status()}),
            ),
            "replace_document" | "load_document" => {
                let data = if name == "replace_document" {
                    serde_json::to_vec(&args["document"]).map_err(|e| e.to_string())?
                } else {
                    let path = self.file(string(args, "filename")?, ".json")?;
                    let mut data = Vec::new();
                    fs::File::open(path)
                        .map_err(|e| e.to_string())?
                        .take(MAX_PATCH_FILE_BYTES as u64 + 1)
                        .read_to_end(&mut data)
                        .map_err(|e| e.to_string())?;
                    data
                };
                let document = ProjectDocument::decode(&data)?;
                self.replace_project_document(document, expected_revision(args)?)?;
                Ok(self.snapshot())
            }
            "save_document" => {
                let path = self.file(string(args, "filename")?, ".json")?;
                let captured = self.capture_project_document(expected_revision(args)?)?;
                write_document_snapshot(&path, string(args, "filename")?, captured)
            }
            "set_document_playback" => {
                let mode = match string(args, "mode")? {
                    "exact" => PlaybackMode::Exact,
                    "partial" => PlaybackMode::Partial,
                    _ => return Err("mode must be exact or partial".into()),
                };
                self.set_document_playback(mode, expected_revision(args)?)?;
                Ok(self.snapshot())
            }
            _ => unreachable!(),
        })())
    }
}

fn has_source(document: &ProjectDocument) -> bool {
    !document.sources.is_empty()
        || !document.source_blobs.is_empty()
        || document.graph.nodes.iter().any(|node| {
            matches!(
                node,
                DocumentNode::Unresolved { .. }
                    | DocumentNode::Native {
                        source: Some(_),
                        ..
                    }
            )
        })
}
fn disposition(document: &ProjectDocument, mode: PlaybackMode) -> ExecutionDisposition {
    let Ok(automation) = document.automation_omissions() else {
        // Accepted documents cannot contain unsafe bindings. Keep this status
        // boundary closed if a caller supplies an invalid in-memory document.
        return ExecutionDisposition::SuspendedUnresolved;
    };
    if mode == PlaybackMode::Partial {
        ExecutionDisposition::RunnablePartial
    } else if !automation.is_empty()
        || document
            .graph
            .nodes
            .iter()
            .any(|node| matches!(node, DocumentNode::Unresolved { .. }))
    {
        ExecutionDisposition::SuspendedUnresolved
    } else {
        ExecutionDisposition::RunnableExact
    }
}

pub(super) fn execution_status(document: &ProjectDocument, mode: PlaybackMode) -> Value {
    let automation = document.automation_omissions();
    let automation_error = automation.as_ref().err();
    let omitted_automation = automation.as_deref().unwrap_or_default();
    let unresolved: BTreeSet<_> = document
        .graph
        .nodes
        .iter()
        .filter_map(|node| matches!(node, DocumentNode::Unresolved { .. }).then_some(node.id()))
        .collect();
    let omitted: Vec<_> = document
        .graph
        .cables
        .iter()
        .filter(|cable| {
            unresolved.contains(cable.route.from.as_str())
                || unresolved.contains(cable.route.to.as_str())
        })
        .map(|cable| json!({"cable_id":cable.route.id,"from":cable.route.from,"to":cable.route.to}))
        .collect();
    let substitutions: Vec<_> = document
        .graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            DocumentNode::Native {
                node,
                source: Some(source),
            } if source.mapping.as_ref().is_some_and(|mapping| {
                matches!(
                    mapping.fidelity,
                    crate::project_document::MappingFidelity::MusicalRole
                        | crate::project_document::MappingFidelity::UserSubstitution
                )
            }) =>
            {
                Some(json!({"id":node.id,"mapping":source.mapping}))
            }
            _ => None,
        })
        .collect();
    json!({"substitutions":substitutions,"mode":if mode == PlaybackMode::Exact {"exact"} else {"partial"},
        "requested_disposition":disposition(document,mode),"unresolved_modules":unresolved,
        "omitted_routes":omitted,"omitted_output":document.graph.output.as_ref().filter(|id|unresolved.contains(id.as_str())),
        "omitted_automation":omitted_automation,"automation_error":automation_error,
        "warning":(!unresolved.is_empty() || !omitted_automation.is_empty()).then_some("Omitted modules, routes and automation can change downstream normaling and musical behavior")})
}

// Encoding and disk I/O run after releasing the shared Session lock.
pub(super) fn write_document_snapshot(
    path: &Path,
    filename: &str,
    captured: ProjectDocumentSnapshot,
) -> Result<Value, String> {
    let bytes = captured.document.encode()?;
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| e.to_string())?
        .write_all(&bytes)
        .map_err(|e| e.to_string())?;
    Ok(
        json!({"filename":filename,"revision":captured.revision,"state_id":captured.state_id,"layout_generation":captured.layout_generation,"state_source":captured.state_source,"frame":captured.frame}),
    )
}
