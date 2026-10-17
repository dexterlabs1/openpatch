//! Resolve the startup document before the shared session is exposed over MCP.
//! The Store moves into the application so recovery claims never lose their locks.
use crate::{
    project::{Document, FileToken, Store},
    rack::Rack,
};
use openpatch::control::Session;
use openpatch_core::catalog;
use std::path::Path;

pub struct Startup {
    pub store: Option<Store>,
    pub document: Document,
    pub revision: u64,
    pub state_id: u64,
    pub active_file: Option<String>,
    pub active_token: Option<FileToken>,
    pub title: String,
    pub status: Option<String>,
    pub imported: bool,
    pub offer_recovery: bool,
}

pub fn factory_document() -> Document {
    let mut patch = openpatch_core::analog_demo_patch();
    patch.version = 2;
    let mut rack = Rack::default();
    rack.reconcile(&patch, &catalog());
    Document::new(patch, rack.layout()).expect("bundled factory project is valid")
}

pub fn prepare(
    workspace: &Path,
    explicit: Option<&str>,
    smoke: bool,
) -> Result<(Session, Startup), String> {
    if smoke && explicit.is_some() {
        return Err("--patch cannot be combined with --smoke-test".into());
    }
    let mut status = None;
    let store = match Store::new(workspace) {
        Ok(store) => Some(store),
        Err(error) if explicit.is_some() => {
            return Err(format!(
                "Cannot open requested project: project storage unavailable: {error}"
            ));
        }
        Err(error) => {
            status = Some(format!("Project storage unavailable: {error}"));
            None
        }
    };
    let mut document = factory_document();
    let mut active_file = None;
    let mut active_token = None;
    let mut title = "Reference analog voice".to_owned();
    let mut imported = false;
    if let Some(name) = explicit {
        let loaded = store
            .as_ref()
            .expect("explicit startup requires storage")
            .load(name)
            .map_err(|error| format!("Cannot open requested project '{name}': {error}"))?;
        document = loaded.document;
        imported = loaded.imported;
        active_token = loaded.token;
        if !imported {
            active_file = Some(name.to_owned());
        }
        title = name
            .trim_end_matches(".openpatch")
            .trim_end_matches(".json")
            .to_owned();
        if imported {
            status = Some("Imported legacy patch. Save as a native project to preserve graph and layout together.".into());
        }
    } else if !smoke {
        if let Some(store) = &store {
            match store.load_template() {
                Ok(Some(template)) => {
                    document = template;
                    title = "Untitled".into();
                }
                Ok(None) => {}
                Err(error) => {
                    status = Some(format!(
                        "Startup template unavailable; opened the factory patch: {error}"
                    ));
                }
            }
        }
    }
    let offer_recovery = !smoke
        && explicit.is_none()
        && store
            .as_ref()
            .is_some_and(|store| !store.recoveries().is_empty());
    let mut session = Session::new(workspace)?;
    session.new_project_document(document.project().clone(), None)?;
    let startup = Startup {
        store,
        document,
        revision: session.revision(),
        state_id: session.state_id(),
        active_file,
        active_token,
        title,
        status,
        imported,
        offer_recovery,
    };
    Ok((session, startup))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn custom_document() -> Document {
        let patch = serde_json::from_value(json!({"version":2,"nodes":[
            {"id":"custom","kind":"constant","params":{"value":3}}
        ],"output":"custom"}))
        .unwrap();
        Document::new(
            patch,
            json!({"version":1,"zoom":0.85,"pan":[40,50],
            "positions":{"custom":[210,380]}}),
        )
        .unwrap()
    }

    #[test]
    fn absent_template_starts_factory_with_coherent_session_and_layout() {
        let workspace = tempfile::tempdir().unwrap();
        let (session, startup) = prepare(workspace.path(), None, false).unwrap();
        assert!(startup.document.equivalent(&factory_document()));
        assert_eq!(
            serde_json::to_value(session.patch()).unwrap(),
            serde_json::to_value(startup.document.patch()).unwrap()
        );
        assert_eq!(
            (session.revision(), session.state_id()),
            (startup.revision, startup.state_id)
        );
        assert!(startup.active_file.is_none() && startup.active_token.is_none());
        assert!(!startup.imported && !startup.offer_recovery);
        assert!(startup.status.is_none());
        assert_eq!(session.snapshot()["history"]["undo_steps"], 0);
        assert_eq!(session.snapshot()["history"]["redo_steps"], 0);
        assert!(!startup.document.layout()["positions"]
            .as_object()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn user_template_supplies_graph_and_layout_but_smoke_uses_factory() {
        let workspace = tempfile::tempdir().unwrap();
        let custom = custom_document();
        {
            let store = Store::new(workspace.path()).unwrap();
            store.save_template(&custom, None).unwrap();
        }
        let (_, startup) = prepare(workspace.path(), None, false).unwrap();
        assert!(startup.document.equivalent(&custom));
        assert_eq!(startup.title, "Untitled");
        assert!(startup.active_file.is_none() && startup.active_token.is_none());
        drop(startup);
        let (_, smoke) = prepare(workspace.path(), None, true).unwrap();
        assert!(smoke.document.equivalent(&factory_document()));
        assert!(!smoke.offer_recovery);
        assert!(prepare(workspace.path(), Some("anything.openpatch"), true).is_err());
    }

    #[test]
    fn explicit_native_and_legacy_projects_outrank_template_and_recovery() {
        let workspace = tempfile::tempdir().unwrap();
        let custom = custom_document();
        {
            let store = Store::new(workspace.path()).unwrap();
            store.save_template(&factory_document(), None).unwrap();
            store.save_new("selected.openpatch", &custom).unwrap();
            store.save_recovery(&factory_document()).unwrap();
        }
        std::fs::write(
            workspace.path().join("legacy.json"),
            serde_json::to_vec(custom.patch()).unwrap(),
        )
        .unwrap();
        std::fs::write(
            workspace.path().join("legacy.json.layout.json"),
            serde_json::to_vec(custom.layout()).unwrap(),
        )
        .unwrap();
        let (_, native) = prepare(workspace.path(), Some("selected.openpatch"), false).unwrap();
        assert!(native.document.equivalent(&custom));
        assert_eq!(native.active_file.as_deref(), Some("selected.openpatch"));
        assert!(native.active_token.is_some());
        assert!(!native.imported && !native.offer_recovery);
        assert_eq!(native.store.as_ref().unwrap().recoveries().len(), 1);
        // A second Store cannot claim the recovery while the prepared Store lives.
        assert!(Store::new(workspace.path())
            .unwrap()
            .recoveries()
            .is_empty());
        drop(native);
        let (_, legacy) = prepare(workspace.path(), Some("legacy.json"), false).unwrap();
        assert!(legacy.document.equivalent(&custom));
        assert!(legacy.imported && legacy.active_file.is_none() && legacy.active_token.is_none());
        assert!(!legacy.offer_recovery);
        assert!(legacy.status.as_ref().unwrap().contains("Imported"));
        drop(legacy);
        let (_, normal) = prepare(workspace.path(), None, false).unwrap();
        assert!(normal.offer_recovery);
        assert!(normal.document.equivalent(&factory_document()));
        assert_eq!(normal.store.as_ref().unwrap().recoveries().len(), 1);
        drop(normal);
        let (_, smoke) = prepare(workspace.path(), None, true).unwrap();
        assert!(!smoke.offer_recovery);
        assert_eq!(smoke.store.as_ref().unwrap().recoveries().len(), 1);
        assert!(smoke.document.equivalent(&factory_document()));
    }

    #[test]
    fn corrupt_template_falls_back_without_rewriting_or_removing_user_bytes() {
        let workspace = tempfile::tempdir().unwrap();
        let path = workspace.path().join(".openpatch-template.openpatch");
        let corrupt = b"{broken user template";
        std::fs::write(&path, corrupt).unwrap();
        let (_, startup) = prepare(workspace.path(), None, false).unwrap();
        assert!(startup.document.equivalent(&factory_document()));
        assert!(startup.status.as_ref().unwrap().contains("template"));
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
        drop(startup);
        let (_, smoke) = prepare(workspace.path(), None, true).unwrap();
        assert!(smoke.document.equivalent(&factory_document()));
        assert!(
            smoke.status.is_none(),
            "smoke does not attempt to load the user template"
        );
        assert_eq!(std::fs::read(&path).unwrap(), corrupt);
    }

    #[test]
    fn invalid_explicit_project_fails_instead_of_silently_using_template() {
        let workspace = tempfile::tempdir().unwrap();
        Store::new(workspace.path())
            .unwrap()
            .save_template(&custom_document(), None)
            .unwrap();
        std::fs::write(workspace.path().join("broken.openpatch"), b"invalid").unwrap();
        for name in [
            "broken.openpatch",
            "missing.openpatch",
            "../outside.openpatch",
        ] {
            assert!(prepare(workspace.path(), Some(name), false).is_err());
        }
    }
}
