use super::*;
use crate::project_document::hosted::{HostedIdentity, HostedModule, HostedState};
use crate::project_document::{DocumentCable, DOCUMENT_VERSION};

fn offset() -> HostedModule {
    HostedModule::offset(
        HostedIdentity {
            plugin: "Bogaudio".into(),
            model: "Bogaudio-Offset".into(),
            version: "2.6.47".into(),
            rack_version: "2.6.6".into(),
            plugin_sha256: "264e55e1389274c3873439372c5c51f48f63168bd6c98fb2c6ffec170224b6f6"
                .into(),
            runtime_sha256: "55dd40f7dff09a6ed7b93374b150f33931034d3754c128da7b209ea37b6aaf1c"
                .into(),
        },
        0.25,
        0.5,
        true,
        false,
    )
    .unwrap()
}

fn document() -> ProjectDocument {
    let patch = serde_json::from_value(json!({"version":2,"sample_rate":48000,
        "nodes":[{"id":"dc","kind":"constant","params":{"value":0.5}},
                 {"id":"amp","kind":"gain","params":{"gain":0.5}}],
        "cables":[],"output":"amp"}))
    .unwrap();
    let mut document = ProjectDocument::from_patch(patch, ProjectLayout::default()).unwrap();
    document.graph.nodes.push(DocumentNode::Hosted {
        id: "offset".into(),
        module: offset(),
        source: None,
    });
    for route in [
        json!({"id":"host-in","from":"dc","output":"out","to":"offset","input":"in"}),
        json!({"id":"host-out","from":"offset","output":"out","to":"amp","input":"signal"}),
    ] {
        document.graph.cables.push(DocumentCable {
            route: serde_json::from_value(route).unwrap(),
            source: None,
        });
    }
    for (i, id) in ["dc", "offset", "amp"].iter().enumerate() {
        document
            .layout
            .positions
            .insert((*id).into(), [i as f32 * 180., 0.]);
    }
    document.validate_structure().unwrap();
    document
}

fn hosted(document: &ProjectDocument) -> &HostedModule {
    document
        .graph
        .nodes
        .iter()
        .find_map(|node| match node {
            DocumentNode::Hosted { module, .. } => Some(module),
            _ => None,
        })
        .unwrap()
}

fn full(document: &ProjectDocument) -> Value {
    serde_json::to_value(document).unwrap()
}

#[test]
fn hosted_document_roundtrip_keeps_routes_but_never_claims_native_execution() {
    let document = document();
    let reopened = ProjectDocument::decode(&document.encode().unwrap()).unwrap();
    assert_eq!(full(&reopened), full(&document));
    assert!(document.project(PlaybackMode::Exact).is_err());
    assert!(document.native_patch_for_legacy_api().is_err());
    let partial = document.project(PlaybackMode::Partial).unwrap();
    assert_eq!(partial.unresolved_modules, vec!["offset"]);
    assert_eq!(partial.omitted_routes.len(), 2);
    assert!(partial.patch.cables.is_empty());
    assert!(partial.patch.nodes.iter().all(|node| node.id != "offset"));
    let compact = document.reference_view();
    let state = &compact["graph"]["nodes"][2]["module"];
    assert!(state.get("state").is_none());
    assert_eq!(
        state["state_reference"]["sha256"],
        hosted(&document).state.id()
    );
    assert_eq!(
        state["state_reference"]["bytes"],
        hosted(&document).state.encoded_len()
    );
    assert!(ProjectDocument::decode(&serde_json::to_vec(&compact).unwrap()).is_err());

    let mut bad_port = document.clone();
    bad_port.graph.cables[0].route.input = "unknown".into();
    assert!(bad_port.validate_structure().is_err());
    let mut collision = partial.patch;
    let mut node = collision.nodes[0].clone();
    node.id = "offset".into();
    collision.nodes.push(node);
    assert!(document
        .with_native_patch(collision, &BTreeMap::new())
        .is_err());
}

#[test]
fn old_document_migration_preserves_source_assets_layout_and_graph_version() {
    let document = super::document_tests::imported();
    let mut old = full(&document);
    old["version"] = json!(2);
    let migrated = ProjectDocument::decode(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(migrated.version, DOCUMENT_VERSION);
    assert_eq!(full(&migrated), full(&document));
    let mut falsely_old = full(&self::document());
    falsely_old["version"] = json!(2);
    assert!(
        ProjectDocument::decode(&serde_json::to_vec(&falsely_old).unwrap())
            .unwrap_err()
            .contains("require project document version 3")
    );
}

#[test]
fn hosted_and_native_edits_share_history_and_preserve_save_generation() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new(dir.path()).unwrap();
    let original = document();
    session
        .replace_project_document(original.clone(), None)
        .unwrap();
    let generation = session.project_save_baseline().generation();
    let layout_generation = session.layout_generation();
    let captured = session.capture_active_project_save(None).unwrap();
    session
        .project_save_baseline()
        .register(&captured.checkpoint, Some("original.json".into()))
        .unwrap();
    assert_eq!(session.project_save_status()["dirty"], false);
    let revision = session.revision;
    session
        .execute(
            "set_hosted_parameter",
            json!({"id":"offset","parameter":"offset","value":0.1,"expected_revision":revision}),
        )
        .unwrap();
    assert_eq!(
        hosted(session.project_document())
            .parameter("offset")
            .unwrap(),
        0.1_f32
    );
    assert_eq!(session.project_save_baseline().generation(), generation);
    assert_eq!(session.layout_generation(), layout_generation);
    assert_eq!(session.project_save_status()["dirty"], true);
    let after_parameter = full(session.project_document());
    assert!(session.execute("set_hosted_option", json!({"id":"offset","option":"offset_first","value":false,"expected_revision":revision})).is_err());
    assert_eq!(full(session.project_document()), after_parameter);
    session
        .execute(
            "set_hosted_option",
            json!({"id":"offset","option":"offset_first","value":false}),
        )
        .unwrap();
    session
        .execute(
            "set_hosted_option",
            json!({"id":"offset","option":"disable_output_limit","value":true}),
        )
        .unwrap();
    let after_hosted = full(session.project_document());
    session
        .execute(
            "set_parameter",
            json!({"id":"dc","parameter":"value","value":0.125}),
        )
        .unwrap();
    assert_eq!(
        hosted(session.project_document()).state,
        hosted(&ProjectDocument::decode(&serde_json::to_vec(&after_hosted).unwrap()).unwrap())
            .state
    );
    assert_eq!(session.project_document().graph.cables.len(), 2);
    let edited = full(session.project_document());
    for _ in 0..4 {
        session.execute("undo", json!({})).unwrap();
    }
    assert_eq!(full(session.project_document()), full(&original));
    assert_eq!(session.project_save_status()["dirty"], false);
    for _ in 0..4 {
        session.execute("redo", json!({})).unwrap();
    }
    assert_eq!(full(session.project_document()), edited);
    assert_eq!(
        session.document_execution_status()["hosted_modules"][0]["execution"],
        "not_active"
    );
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert!(session
        .execute("save_patch", json!({"filename":"lossy.json"}))
        .is_err());
    session
        .execute("save_document", json!({"filename":"hosted.json"}))
        .unwrap();
    let mut fresh = Session::new(dir.path()).unwrap();
    fresh
        .execute("load_document", json!({"filename":"hosted.json"}))
        .unwrap();
    assert_eq!(full(fresh.project_document()), edited);
    assert_eq!(
        fresh.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
}

#[test]
fn unknown_hosted_state_is_preserved_and_unsupported_edits_are_atomic() {
    let mut document = document();
    if let DocumentNode::Hosted { module, .. } = &mut document.graph.nodes[2] {
        let mut state = module.state.as_ref().clone();
        state["future-custom"] = json!({"keep":[null,{"nested":"opaque"}]});
        module.state = HostedState::new(state).unwrap();
    }
    document.validate_structure().unwrap();
    assert!(hosted(&document).validate_supported_offset().is_err());
    assert_eq!(
        full(&ProjectDocument::decode(&document.encode().unwrap()).unwrap()),
        full(&document)
    );
    let dir = tempfile::tempdir().unwrap();
    let mut session = Session::new(dir.path()).unwrap();
    session
        .replace_project_document(document.clone(), None)
        .unwrap();
    let revision = session.revision;
    let history = session.execute("inspect_history", json!({})).unwrap();
    assert!(session
        .set_hosted_parameter("offset", "scale", 0.5, None)
        .is_err());
    assert!(session
        .set_hosted_option("offset", "offset_first", false, None)
        .is_err());
    assert_eq!(session.revision, revision);
    assert_eq!(
        session.execute("inspect_history", json!({})).unwrap(),
        history
    );
    assert_eq!(full(session.project_document()), full(&document));
    assert!(
        session.document_execution_status()["hosted_modules"][0]["capability_error"].is_string()
    );
}
