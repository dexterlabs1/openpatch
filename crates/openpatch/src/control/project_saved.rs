//! Shared authored-content save authority. Effective automation values are
//! sampled for persistence, but do not by themselves dirty the live document.
use super::*;

#[derive(Clone, Debug)]
pub struct ProjectSaveCheckpoint {
    pub generation: u64,
    pub revision: u64,
    pub frame: Option<u64>,
    pub edit_content: Value,
}

#[derive(Clone, Debug)]
pub struct ActiveProjectSave {
    pub snapshot: ProjectDocumentSnapshot,
    pub checkpoint: ProjectSaveCheckpoint,
}

#[derive(Clone, Debug, Default)]
struct SaveBaseline {
    generation: u64,
    content: Option<Value>,
    target: Option<String>,
    revision: Option<u64>,
    frame: Option<u64>,
}

/// Shared by the live Session and desktop dirty/close presentation. Session
/// preflight clones must call `detached` to avoid mutating this live authority.
#[derive(Clone, Debug, Default)]
pub struct SharedProjectSave(Arc<Mutex<SaveBaseline>>);

impl SharedProjectSave {
    pub fn generation(&self) -> u64 {
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .generation
    }

    pub fn detached(&self) -> Self {
        let baseline = self
            .0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        Self(Arc::new(Mutex::new(baseline)))
    }

    pub fn next_generation(&self) -> Result<u64, String> {
        self.0
            .lock()
            .map_err(|_| "project save lock poisoned")?
            .generation
            .checked_add(1)
            .ok_or_else(|| "project save generation exhausted".into())
    }

    pub fn begin_document(&self, next_generation: u64) -> Result<(), String> {
        let mut baseline = self.0.lock().map_err(|_| "project save lock poisoned")?;
        if baseline.generation.checked_add(1) != Some(next_generation) {
            return Err("stale project save generation".into());
        }
        *baseline = SaveBaseline {
            generation: next_generation,
            ..SaveBaseline::default()
        };
        Ok(())
    }

    /// Call only after the captured snapshot has been successfully written.
    /// An earlier document's completion can never clean the newly opened one.
    pub fn register(
        &self,
        checkpoint: &ProjectSaveCheckpoint,
        target: Option<String>,
    ) -> Result<(), String> {
        let mut baseline = self.0.lock().map_err(|_| "project save lock poisoned")?;
        if checkpoint.generation != baseline.generation {
            return Err("save completed for an obsolete document generation".into());
        }
        baseline.content = Some(checkpoint.edit_content.clone());
        baseline.target = target;
        baseline.revision = Some(checkpoint.revision);
        baseline.frame = checkpoint.frame;
        Ok(())
    }

    /// Establish a clean, untitled startup/new-template authored baseline.
    pub fn mark_initial(&self, edit_content: Value) -> Result<(), String> {
        let mut baseline = self.0.lock().map_err(|_| "project save lock poisoned")?;
        baseline.content = Some(edit_content);
        baseline.target = None;
        baseline.revision = None;
        baseline.frame = None;
        Ok(())
    }

    pub fn mark_unsaved(&self) -> Result<(), String> {
        let mut baseline = self.0.lock().map_err(|_| "project save lock poisoned")?;
        baseline.content = None;
        baseline.target = None;
        baseline.revision = None;
        baseline.frame = None;
        Ok(())
    }

    pub fn matches_content(&self, content: &Value) -> bool {
        self.0
            .lock()
            .is_ok_and(|baseline| baseline.content.as_ref() == Some(content))
    }

    pub fn status_from_content(&self, content: Option<&Value>) -> Value {
        let Ok(baseline) = self.0.lock() else {
            return json!({"status":"unknown","state":"unknown","saved":false,"dirty":null,
                "basis":"authored_content_and_durable_state","target":null,
                "saved_revision":null,"saved_frame":null,"generation":null});
        };
        let dirty = content.map(|content| baseline.content.as_ref() != Some(content));
        let status = match dirty {
            None => "unknown",
            Some(true) if baseline.content.is_some() || baseline.target.is_some() => "modified",
            Some(_) if baseline.target.is_none() => "untitled",
            Some(false) => "saved",
            Some(true) => "modified",
        };
        json!({"status":status,"state":status,"saved":status == "saved","dirty":dirty,
            "basis":"authored_content_and_durable_state","target":baseline.target,
            "saved_revision":baseline.revision,"saved_frame":baseline.frame,
            "generation":baseline.generation})
    }
}

impl Session {
    pub fn project_save_baseline(&self) -> SharedProjectSave {
        self.project_save.clone()
    }

    /// Compare authored graph/layout/source references plus durable live state.
    /// Transient/effective runtime parameter values are deliberately excluded.
    pub fn project_edit_content(&self, expected_revision: Option<u64>) -> Result<Value, String> {
        self.check_open()?;
        self.check_revision(expected_revision)?;
        if self.layout_pending {
            return Err(
                "host layout placement is pending; retry saved-state observation after placement"
                    .into(),
            );
        }
        let states = if self.execution_disposition().is_runnable() {
            self.capture_project_running(expected_revision)?
                .saved_states
        } else {
            configured_document_states(&self.document)
        };
        Ok(saved_content(
            &self.document,
            serde_json::to_value(&self.document.layout).map_err(|error| error.to_string())?,
            &states,
        ))
    }

    pub fn project_save_status(&self) -> Value {
        match self.project_edit_content(None) {
            Ok(content) => {
                let mut status = self.project_save.status_from_content(Some(&content));
                status["revision"] = json!(self.revision);
                status
            }
            Err(error) => {
                let mut status = self.project_save.status_from_content(None);
                status["reason"] = json!(error);
                status["revision"] = json!(self.revision);
                status
            }
        }
    }

    /// Capture persistence and its authored baseline while the caller holds the
    /// same Session lock. The durable state is read from that exact snapshot,
    /// without making a second callback observation.
    pub fn capture_active_project_save(
        &self,
        expected_revision: Option<u64>,
    ) -> Result<ActiveProjectSave, String> {
        let snapshot = self.capture_project_document(expected_revision)?;
        let states = configured_document_states(&snapshot.document);
        let edit_content = saved_content(
            &self.document,
            serde_json::to_value(&self.document.layout).map_err(|error| error.to_string())?,
            &states,
        );
        let generation = self
            .project_save
            .0
            .lock()
            .map_err(|_| "project save lock poisoned")?
            .generation;
        let checkpoint = ProjectSaveCheckpoint {
            generation,
            revision: snapshot.revision,
            frame: snapshot.frame,
            edit_content,
        };
        Ok(ActiveProjectSave {
            snapshot,
            checkpoint,
        })
    }
}

pub fn configured_document_states(project: &ProjectDocument) -> BTreeMap<String, SavedModuleState> {
    project
        .graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            DocumentNode::Native { node, .. } => {
                SavedModuleState::from_node(node).map(|state| (node.id.clone(), state))
            }
            DocumentNode::Unresolved { .. } => None,
        })
        .collect()
}

// Source package/blob keys and native asset IDs are validated content digests.
// Their reference view therefore tracks every persisted byte without expanding
// immutable payloads during each UI observation. Normalize only native state.
pub fn saved_content(
    project: &ProjectDocument,
    layout: Value,
    states: &BTreeMap<String, SavedModuleState>,
) -> Value {
    let bits = states
        .iter()
        .filter_map(|(id, state)| match state {
            SavedModuleState::RandomValues { values, rng } => {
                Some((id, json!({"values":values,"rng":rng})))
            }
            SavedModuleState::ShiftRegister { values, rng } => {
                Some((id, json!({"values":values,"rng":rng})))
            }
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    let mut document = project.reference_view();
    document["layout"] = layout;
    for entry in document["graph"]["nodes"]
        .as_array_mut()
        .expect("serialized document nodes")
    {
        if entry["type"] != "native" {
            continue;
        }
        let node = &mut entry["node"];
        if let Some(state) = node["id"].as_str().and_then(|id| states.get(id)) {
            let mut typed: openpatch_core::Node =
                serde_json::from_value(node.clone()).expect("serialized native node");
            state.write_node(&mut typed);
            *node = serde_json::to_value(typed).expect("native node serializes");
        }
    }
    json!({"document":document,"persistent_bits":bits})
}

#[cfg(test)]
mod tests {
    use super::*;
    include!("project_saved_tests.rs");
}
