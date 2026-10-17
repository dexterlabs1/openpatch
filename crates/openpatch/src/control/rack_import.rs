use super::*;
use crate::project_document::{MappingFidelity, SourceMapping};

impl Session {
    pub fn import_vcv_bytes(
        &mut self,
        bytes: &[u8],
        expected: Option<u64>,
    ) -> Result<Value, String> {
        self.check_open()?;
        self.check_revision(expected)?;
        let imported = crate::vcv_import::import(bytes)?;
        self.replace_project_document(imported.document, expected)?;
        self.layout_origin = Some(LayoutOrigin::SourceImport);
        Ok(
            json!({"revision":self.revision,"report":imported.report,"execution":self.document_execution_status()}),
        )
    }

    fn commit_dependency_repair(
        &mut self,
        document: ProjectDocument,
        expected: Option<u64>,
    ) -> Result<Value, String> {
        let carrier = document.state_carrier()?;
        let mut effects = EditEffects::default();
        effects.replacement_document = Some(document);
        self.commit(carrier.patch, expected, None, &effects)?;
        Ok(self.snapshot())
    }

    pub(super) fn rack_import_tool(
        &mut self,
        name: &str,
        args: &Value,
    ) -> Option<Result<Value, String>> {
        if !matches!(
            name,
            "import_vcv" | "substitute_unresolved_module" | "remove_unresolved_module"
        ) {
            return None;
        }
        Some((|| {
            self.check_open()?;
            self.check_revision(expected_revision(args)?)?;
            if name == "import_vcv" {
                let path = self.file(string(args, "filename")?, ".vcv")?;
                let mut bytes = Vec::new();
                fs::File::open(path)
                    .map_err(|e| e.to_string())?
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|e| e.to_string())?;
                return self.import_vcv_bytes(&bytes, expected_revision(args)?);
            }
            let id = string(args, "id")?;
            let mut document = self.document.clone();
            let index = document
                .graph
                .nodes
                .iter()
                .position(|node| node.id() == id)
                .ok_or("unresolved module not found")?;
            let DocumentNode::Unresolved { source, .. } = &document.graph.nodes[index] else {
                return Err("module is already native".into());
            };
            if name == "remove_unresolved_module" {
                document.graph.nodes.remove(index);
                document
                    .graph
                    .cables
                    .retain(|cable| cable.route.from != id && cable.route.to != id);
                if document.graph.output.as_deref() == Some(id) {
                    document.graph.output = None;
                    document.graph.output_port = "out".into();
                }
                document.layout.positions.remove(id);
            } else {
                let kind = string(args, "kind")?;
                let catalog = catalog();
                let module = catalog["modules"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|module| module["kind"] == kind)
                    .ok_or("unknown native module kind")?;
                let inputs = port_map(args, "inputs", &module["inputs"])?;
                let outputs = port_map(args, "outputs", &module["outputs"])?;
                for cable in &mut document.graph.cables {
                    if cable.route.to == id {
                        cable.route.input = inputs
                            .get(&cable.route.input)
                            .ok_or("every incident input needs a mapping")?
                            .clone();
                    }
                    if cable.route.from == id {
                        cable.route.output = outputs
                            .get(&cable.route.output)
                            .ok_or("every incident output needs a mapping")?
                            .clone();
                    }
                }
                if document.graph.output.as_deref() == Some(id) {
                    document.graph.output_port = outputs
                        .get(&document.graph.output_port)
                        .ok_or("selected output needs a mapping")?
                        .clone();
                }
                let mut source = source.clone();
                source.mapping = Some(SourceMapping {
                    id: "openpatch/user-substitution".into(),
                    version: 1,
                    fidelity: MappingFidelity::UserSubstitution,
                });
                let node:Node=serde_json::from_value(json!({"id":id,"kind":kind,"params":args.get("params").cloned().unwrap_or_else(||json!({}))})).map_err(|e|e.to_string())?;
                document.graph.nodes[index] = DocumentNode::Native {
                    node,
                    source: Some(source),
                };
            }
            self.commit_dependency_repair(document, expected_revision(args)?)
        })())
    }
}
fn port_map(args: &Value, key: &str, ports: &Value) -> Result<BTreeMap<String, String>, String> {
    let mappings: BTreeMap<String, String> = serde_json::from_value(args[key].clone())
        .map_err(|_| format!("{key} must map source port keys to native port names"))?;
    for (source, target) in &mappings {
        let prefix = if key == "inputs" {
            "vcv_in_"
        } else {
            "vcv_out_"
        };
        let index = source
            .strip_prefix(prefix)
            .and_then(|i| i.parse::<u32>().ok())
            .ok_or("invalid source port key")?;
        if source != &format!("{prefix}{index}") {
            return Err("source port key is not canonical".into());
        }
        if !ports
            .as_array()
            .is_some_and(|ports| ports.iter().any(|port| port == target))
        {
            return Err(format!("unknown native port '{target}'"));
        }
    }
    Ok(mappings)
}

#[cfg(test)]
mod tests {
    use super::super::document_tests::imported;
    use super::*;
    #[test]
    fn dependency_repair_is_explicit_atomic_undoable_and_preserves_original_data() {
        let mut session = Session::new(&std::env::temp_dir()).unwrap();
        let original = imported();
        session
            .replace_project_document(original.clone(), None)
            .unwrap();
        let before = session.snapshot();
        assert!(session
            .execute(
                "substitute_unresolved_module",
                json!({"id":"unknown","kind":"gain","inputs":{},"outputs":{}})
            )
            .is_err());
        assert_eq!(session.snapshot(), before);
        session.execute("substitute_unresolved_module",json!({"id":"unknown","kind":"gain","params":{"gain":0.5},"inputs":{"vcv_in_0":"signal"},"outputs":{}})).unwrap();
        assert_eq!(
            session.execution_disposition(),
            ExecutionDisposition::RunnableExact
        );
        assert_eq!(
            session.document_execution_status()["substitutions"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(session.patch().cables[0].input, "signal");
        let original_id = original.sources.keys().next().unwrap();
        assert_eq!(
            session.project_document().sources[original_id].patch,
            original.sources[original_id].patch
        );
        assert_eq!(session.project_document().source_blobs.len(), 1);
        let repaired = session.project_document().encode().unwrap();
        session.execute("undo", json!({})).unwrap();
        assert_eq!(
            session.execution_disposition(),
            ExecutionDisposition::SuspendedUnresolved
        );
        assert_eq!(
            serde_json::to_value(session.project_document()).unwrap(),
            serde_json::to_value(&original).unwrap()
        );
        session.execute("redo", json!({})).unwrap();
        assert_eq!(session.project_document().encode().unwrap(), repaired);
        assert!(
            session
                .execute(
                    "substitute_unresolved_module",
                    json!({"id":"unknown","kind":"gain","inputs":{},"outputs":{}})
                )
                .is_err(),
            "cannot silently re-substitute an already native instance"
        );
    }
    #[test]
    fn unresolved_removal_retires_only_its_routes_and_undo_restores_everything() {
        let mut session = Session::new(&std::env::temp_dir()).unwrap();
        let document = imported();
        session
            .replace_project_document(document.clone(), None)
            .unwrap();
        let before = session.snapshot();
        assert!(session
            .execute("remove_unresolved_module", json!({"id":"dc"}))
            .is_err());
        assert_eq!(session.snapshot(), before);
        session
            .execute("remove_unresolved_module", json!({"id":"unknown"}))
            .unwrap();
        assert!(session.project_document().graph.cables.is_empty());
        assert_eq!(session.project_document().graph.nodes.len(), 1);
        assert_eq!(
            session.project_document().sources.len(),
            1,
            "immutable originals remain recoverable after removal"
        );
        session.execute("undo", json!({})).unwrap();
        assert_eq!(
            session.project_document().encode().unwrap(),
            document.encode().unwrap()
        );
    }
    #[test]
    fn workspace_vcv_import_is_read_only_and_requires_explicit_partial_permission() {
        let dir = tempfile_dir();
        let source = dir.join("source.vcv");
        let bytes=serde_json::to_vec(&json!({"version":"2.6.6","modules":[{"id":12,"plugin":"Missing","model":"Voice","version":"1.0.0","pos":[1,0],"data":{"future":true}}],"cables":[]})).unwrap();
        fs::write(&source, &bytes).unwrap();
        let mut session = Session::new(&dir).unwrap();
        let result = session
            .execute("import_vcv", json!({"filename":"source.vcv"}))
            .unwrap();
        assert_eq!(result["report"]["unresolved_count"], 1);
        assert_eq!(
            session.execution_disposition(),
            ExecutionDisposition::SuspendedUnresolved
        );
        assert_eq!(fs::read(&source).unwrap(), bytes);
        let before = session.snapshot();
        assert!(session
            .execute("import_vcv", json!({"filename":"../source.vcv"}))
            .is_err());
        assert_eq!(session.snapshot(), before);
        session
            .execute("save_document", json!({"filename":"saved.json"}))
            .unwrap();
        session
            .execute("load_document", json!({"filename":"saved.json"}))
            .unwrap();
        assert_eq!(
            session.execution_disposition(),
            ExecutionDisposition::SuspendedUnresolved
        );
        fs::remove_dir_all(dir).unwrap();
    }
    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "openpatch-vcv-control-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
