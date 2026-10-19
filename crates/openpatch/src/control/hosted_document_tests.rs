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
fn document_load_publishes_its_new_generation_before_notifying_the_backend() {
    struct Observer {
        save: SharedProjectSave,
        seen: Mutex<Vec<u64>>,
    }
    impl LiveModuleEvents for Observer {
        fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
            unreachable!()
        }
        fn snapshot(&self) -> Value {
            Value::Null
        }
        fn supports_document_execution(&self) -> bool {
            true
        }
        fn sync_patch(&self, _: &Patch, _: u64) {
            self.seen.lock().unwrap().push(self.save.generation());
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let mut session = Session::new(temp.path()).unwrap();
    let observer = Arc::new(Observer {
        save: session.project_save.clone(),
        seen: Mutex::new(Vec::new()),
    });
    session
        .set_live_module_events(Some(observer.clone()))
        .unwrap();
    let previous = session.project_save.generation();
    session.replace_project_document(document(), None).unwrap();
    assert_eq!(observer.seen.lock().unwrap().last(), Some(&(previous + 1)));
    let committed_generation = session.project_save.generation();
    let committed_revision = session.revision();
    let mut invalid = document();
    invalid.graph.cables[0].route.input = "missing".into();
    assert!(session.replace_project_document(invalid, None).is_err());
    assert_eq!(session.project_save.generation(), committed_generation);
    assert_eq!(session.revision(), committed_revision);
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn full_execution_tickets_keep_permission_document_and_retry_identity_together() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = Session::new(temp.path()).unwrap();
    session.replace_project_document(document(), None).unwrap();
    let first = session.audio_snapshot().unwrap();
    assert_eq!(first.document_generation, session.project_save.generation());
    assert_eq!(full(&first.document), full(session.project_document()));
    assert_eq!(
        first.native_projection().unwrap().1,
        ExecutionDisposition::SuspendedUnresolved
    );
    let permission_revision = session
        .request_delayed_hosted_execution(Some(session.revision()))
        .unwrap();
    assert_eq!(session.playback_mode(), PlaybackMode::Exact);
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    assert!(
        !session.acknowledge_audio_snapshot(first.id),
        "a new permission decision supersedes the old request"
    );
    let requested = session.audio_snapshot().unwrap();
    assert_eq!(requested.revision, permission_revision);
    assert!(matches!(
        requested.execution,
        DocumentExecutionRequest::HostedDelayed(_)
    ));
    assert!(requested
        .native_projection()
        .unwrap_err()
        .contains("native projection refused"));
    assert_eq!(requested.document.graph.nodes.len(), 3);
    assert_eq!(requested.document.graph.cables.len(), 2);
    session
        .set_hosted_parameter("offset", "offset", -0.5, None)
        .unwrap();
    assert!(Arc::ptr_eq(&requested, &session.audio_snapshot().unwrap()));
    assert_eq!(
        hosted(&requested.document).parameter("offset").unwrap(),
        0.25
    );
    assert!(session.acknowledge_audio_snapshot(requested.id));
    let edited = session.audio_snapshot().unwrap();
    assert_eq!(hosted(&edited.document).parameter("offset").unwrap(), -0.5);
    assert!(matches!(
        edited.execution,
        DocumentExecutionRequest::HostedDelayed(_)
    ));
    assert_eq!(
        session.document_execution_status()["delayed_hosted_request"]["active"],
        false
    );
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn delayed_permission_is_ephemeral_and_cannot_be_restored_by_undo_or_a_load() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = Session::new(temp.path()).unwrap();
    session.replace_project_document(document(), None).unwrap();
    let authored = session.project_edit_content(None).unwrap();
    session.request_delayed_hosted_execution(None).unwrap();
    assert_eq!(session.project_edit_content(None).unwrap(), authored);
    assert!(session.clone().hosted_execution_grant.is_none());
    session
        .set_hosted_parameter("offset", "scale", 0.75, None)
        .unwrap();
    session.execute("undo", json!({})).unwrap();
    assert!(
        session.hosted_execution_grant.is_some(),
        "a knob undo keeps the same topology"
    );
    session
        .execute("add_module", json!({"id":"new","kind":"constant"}))
        .unwrap();
    assert!(session.hosted_execution_grant.is_none());
    session.execute("undo", json!({})).unwrap();
    assert!(
        session.hosted_execution_grant.is_none(),
        "undo cannot revive permission"
    );
    session.request_delayed_hosted_execution(None).unwrap();
    let generation = session.project_save.generation();
    session.replace_project_document(document(), None).unwrap();
    assert!(session.hosted_execution_grant.is_none());
    assert_eq!(session.project_save.generation(), generation + 1);
    let ticket = session.audio_snapshot().unwrap();
    assert_eq!(ticket.document_generation, generation + 1);
    assert!(matches!(
        ticket.execution,
        DocumentExecutionRequest::Native(ExecutionDisposition::SuspendedUnresolved)
    ));
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn revocation_supersedes_a_sealed_hosted_request_without_losing_ordered_effects() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = Session::new(temp.path()).unwrap();
    session.replace_project_document(document(), None).unwrap();
    session.request_delayed_hosted_execution(None).unwrap();
    let hosted = session.audio_snapshot().unwrap();
    assert!(!hosted.delta.is_empty());
    let effects = hosted.delta.clone();
    session.revoke_delayed_hosted_execution(None).unwrap();
    assert!(!session.acknowledge_audio_snapshot(hosted.id));
    let revoked = session.audio_snapshot().unwrap();
    assert_eq!(revoked.delta, effects);
    assert_eq!(
        revoked.native_projection().unwrap().1,
        ExecutionDisposition::SuspendedUnresolved
    );
    assert_eq!(revoked.revision, session.revision());
    assert!(session.acknowledge_audio_snapshot(revoked.id));
    assert!(session.audio_snapshot().unwrap().delta.is_empty());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn undo_of_an_identical_host_only_load_revokes_the_new_incarnation_permission() {
    let mut document = document();
    document
        .graph
        .nodes
        .retain(|node| matches!(node, DocumentNode::Hosted { .. }));
    document.graph.output = Some("offset".into());
    document.graph.output_port = "out".into();
    document.graph.cables = vec![DocumentCable {
        route: serde_json::from_value(
            json!({"id":"feedback", "from":"offset", "output":"out", "to":"offset", "input":"in"}),
        )
        .unwrap(),
        source: None,
    }];
    document.layout.positions.retain(|id, _| id == "offset");
    let temp = tempfile::tempdir().unwrap();
    let mut session = Session::new(temp.path()).unwrap();
    session
        .replace_project_document(document.clone(), None)
        .unwrap();
    session
        .replace_project_document(document.clone(), None)
        .unwrap();
    session.request_delayed_hosted_execution(None).unwrap();
    assert!(session.patch.nodes.is_empty());
    session.execute("undo", json!({})).unwrap();
    assert_eq!(full(session.project_document()), full(&document));
    assert!(
        session.hosted_execution_grant.is_none(),
        "load history must fence hosted incarnations even when the native carrier is empty"
    );
    session.execute("redo", json!({})).unwrap();
    assert!(session.hosted_execution_grant.is_none());
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn delayed_request_never_authorizes_native_only_offline_tools_even_in_partial_mode() {
    let temp = tempfile::tempdir().unwrap();
    let mut session = Session::new(temp.path()).unwrap();
    session.replace_project_document(document(), None).unwrap();
    session
        .set_document_playback(PlaybackMode::Partial, None)
        .unwrap();
    session.request_delayed_hosted_execution(None).unwrap();
    assert!(session
        .require_document_execution()
        .unwrap_err()
        .contains("native-only"));
    assert!(session
        .execute(
            "render_audio",
            json!({"filename":"omitted.wav","seconds":0.01})
        )
        .is_err());
    assert!(!temp.path().join("omitted.wav").exists());
    session.revoke_delayed_hosted_execution(None).unwrap();
    session.require_document_execution().unwrap();
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

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod live_admission {
    use super::*;
    use crate::binary_host::control::{
        HostedControl, HostedControlReader, ParameterAssignment, COMMAND_CAPACITY,
    };
    use crate::binary_host::{Fence, MAX_EVENTS};

    struct Backend(Arc<HostedControl>);

    impl LiveModuleEvents for Backend {
        fn trigger(&self, _: &str, _: ModuleAction) -> Result<Value, String> {
            Err("this admission fixture has no native action executor".into())
        }

        fn snapshot(&self) -> Value {
            serde_json::to_value(self.0.snapshot()).unwrap()
        }

        fn supports_document_execution(&self) -> bool {
            true
        }

        fn reserve_hosted_parameters(
            &self,
            proposal: HostedEditProposal<'_>,
        ) -> Result<Option<Box<dyn HostedEditAdmission>>, String> {
            proposal.reserve_with_control(&self.0).map(Some)
        }
    }

    fn fixture() -> (
        tempfile::TempDir,
        Session,
        Arc<HostedControl>,
        HostedControlReader,
    ) {
        let directory = tempfile::tempdir().unwrap();
        let mut session = Session::new(directory.path()).unwrap();
        session.replace_project_document(document(), None).unwrap();
        let saved = session.capture_active_project_save(None).unwrap();
        session
            .project_save_baseline()
            .register(&saved.checkpoint, Some("admission-baseline.json".into()))
            .unwrap();
        session.request_delayed_hosted_execution(None).unwrap();
        let request = session.audio_snapshot().unwrap();
        let DocumentExecutionRequest::HostedDelayed(grant) = &request.execution else {
            panic!("fixture requires the explicit full-document execution grant");
        };
        let (control, reader) = HostedControl::new(
            grant.clone(),
            &request.document,
            Fence {
                incarnation: 71,
                generation: request.document_generation,
                revision: 901,
            },
            session.revision(),
        )
        .unwrap();
        session
            .set_live_module_events(Some(Arc::new(Backend(Arc::clone(&control)))))
            .unwrap();
        (directory, session, control, reader)
    }

    fn authored_checkpoint(session: &Session) -> Value {
        json!({
            "document": full(session.project_document()),
            "revision": session.revision(),
            "state_id": session.state_id,
            "history": session.history(),
            "save": session.project_save_status(),
            "generation": session.project_save_baseline().generation(),
            "layout_generation": session.layout_generation(),
        })
    }

    fn fill_remaining(session: &Session, control: &Arc<HostedControl>) {
        let value = hosted(session.project_document())
            .parameter("offset")
            .unwrap();
        let batch = [ParameterAssignment {
            parameter: 0,
            value,
        }; MAX_EVENTS];
        let mut remaining = COMMAND_CAPACITY - control.snapshot().outstanding_commands;
        while remaining > 0 {
            let count = remaining.min(MAX_EVENTS);
            control
                .reserve(&batch[..count])
                .unwrap()
                .publish(session.revision());
            remaining -= count;
        }
        assert_eq!(control.snapshot().outstanding_commands, COMMAND_CAPACITY);
    }

    #[test]
    fn equal_value_edit_undo_and_redo_each_deliver_their_own_command() {
        let (_directory, mut session, control, mut reader) = fixture();
        let original = full(session.project_document());
        let generation = session.project_save_baseline().generation();
        let edit_revision = session
            .set_hosted_parameter("offset", "offset", 0.25, Some(session.revision()))
            .unwrap();
        session.execute("undo", json!({})).unwrap();
        let undo_revision = session.revision();
        session.execute("redo", json!({})).unwrap();
        let redo_revision = session.revision();
        assert_eq!(full(session.project_document()), original);
        assert_eq!(session.project_save_baseline().generation(), generation);
        assert_eq!(session.project_save_status()["dirty"], false);
        assert_eq!(control.snapshot().latest_accepted_sequence, 3);
        assert!(control.snapshot().applied.is_none());
        for (index, revision) in [edit_revision, undo_revision, redo_revision]
            .into_iter()
            .enumerate()
        {
            let event = reader.next_for_sample(0).unwrap().unwrap();
            assert_eq!(event.sequence, index as u64 + 1);
            assert_eq!(event.wire_order, index as u32);
            assert_eq!(event.parameter, 0);
            assert_eq!(event.value, 0.25);
            assert_eq!(event.authored_revision, revision);
        }
        assert!(reader.next_for_sample(0).unwrap().is_none());
        reader.acknowledge(1, 384).unwrap();
        let applied = control.snapshot().applied.unwrap();
        assert_eq!(applied.sequence, 3);
        assert_eq!(applied.authored_revision, redo_revision);
        assert_eq!(applied.values.offset, 0.25);
        assert_eq!(control.snapshot().outstanding_commands, 0);
    }

    #[test]
    fn ordinary_edit_and_undo_share_the_reader_while_native_edits_emit_no_hosted_commands() {
        let (_directory, mut session, control, mut reader) = fixture();
        let edit_revision = session
            .set_hosted_parameter("offset", "offset", -0.75, None)
            .unwrap();
        assert_eq!(control.snapshot().latest_accepted_sequence, 1);
        session
            .execute(
                "set_parameter",
                json!({"id":"dc", "parameter":"value", "value":0.125}),
            )
            .unwrap();
        assert_eq!(control.snapshot().latest_accepted_sequence, 1);
        session.execute("undo", json!({})).unwrap();
        assert_eq!(control.snapshot().latest_accepted_sequence, 1);
        assert_eq!(
            hosted(session.project_document())
                .parameter("offset")
                .unwrap(),
            -0.75
        );
        session.execute("undo", json!({})).unwrap();
        let undo_revision = session.revision();
        assert_eq!(
            hosted(session.project_document())
                .parameter("offset")
                .unwrap(),
            0.25
        );
        let edit = reader.next_for_sample(37).unwrap().unwrap();
        let undo = reader.next_for_sample(37).unwrap().unwrap();
        assert_eq!(
            (edit.sequence, edit.value, edit.authored_revision),
            (1, -0.75, edit_revision)
        );
        assert_eq!(
            (undo.sequence, undo.value, undo.authored_revision),
            (2, 0.25, undo_revision)
        );
        assert!(reader.next_for_sample(37).unwrap().is_none());
        reader.acknowledge(1, 384).unwrap();
        assert_eq!(control.snapshot().applied.unwrap().values.offset, 0.25);
        assert_eq!(control.snapshot().applied.unwrap().sequence, 2);
    }

    #[test]
    fn queue_full_and_stale_revision_reject_before_authored_history_or_baseline_changes() {
        let (_directory, mut session, control, _reader) = fixture();
        let before = authored_checkpoint(&session);
        let stale = session.revision() - 1;
        assert!(session
            .set_hosted_parameter("offset", "offset", -0.75, Some(stale))
            .unwrap_err()
            .contains("revision"));
        assert_eq!(authored_checkpoint(&session), before);
        assert_eq!(control.snapshot().latest_accepted_sequence, 0);
        assert!(!control.snapshot().reservation_outstanding);
        fill_remaining(&session, &control);
        let full_sequence = control.snapshot().latest_accepted_sequence;
        assert!(session
            .set_hosted_parameter("offset", "offset", -0.75, None)
            .unwrap_err()
            .contains("queue is full"));
        assert_eq!(authored_checkpoint(&session), before);
        assert_eq!(control.snapshot().latest_accepted_sequence, full_sequence);
        assert!(!control.snapshot().reservation_outstanding);
        assert!(session
            .set_hosted_parameter("offset", "offset", -0.75, Some(stale))
            .unwrap_err()
            .contains("revision"));
        assert_eq!(authored_checkpoint(&session), before);
        assert_eq!(control.snapshot().latest_accepted_sequence, full_sequence);
    }

    #[test]
    fn saturated_undo_and_redo_preserve_both_history_stacks_and_save_baseline() {
        for action in ["undo", "redo"] {
            let (_directory, mut session, control, _reader) = fixture();
            session
                .set_hosted_parameter("offset", "scale", -0.5, None)
                .unwrap();
            if action == "redo" {
                session.execute("undo", json!({})).unwrap();
            }
            fill_remaining(&session, &control);
            let before = authored_checkpoint(&session);
            let sequence = control.snapshot().latest_accepted_sequence;
            assert!(session
                .execute(action, json!({}))
                .unwrap_err()
                .contains("queue is full"));
            assert_eq!(authored_checkpoint(&session), before);
            assert_eq!(control.snapshot().latest_accepted_sequence, sequence);
            assert!(!control.snapshot().reservation_outstanding);
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[test]
fn public_hosted_activation_is_explicit_revision_fenced_and_never_saved_or_undo_revived() {
    let workspace = tempfile::tempdir().unwrap();
    let mut session = Session::new(workspace.path()).unwrap();
    session.replace_project_document(document(), None).unwrap();
    let original = session.project_document().encode().unwrap();
    let revision = session.revision();
    assert!(session
        .execute(
            "set_hosted_execution",
            json!({"enabled":true,"expected_revision":revision-1})
        )
        .is_err());
    assert!(session.hosted_execution_grant.is_none());
    let result = session
        .execute(
            "set_hosted_execution",
            json!({"enabled":true,"expected_revision":revision}),
        )
        .unwrap();
    assert_eq!(result["revision"], revision + 1);
    assert_eq!(
        session.document_execution_status()["delayed_hosted_request"]["active"],
        false
    );
    assert_eq!(session.project_document().encode().unwrap(), original);
    session
        .execute(
            "set_hosted_parameter",
            json!({"id":"offset","parameter":"offset","value":0.5,"history_group":"one-drag"}),
        )
        .unwrap();
    let grouped_state = session.state_id();
    session
        .execute(
            "set_hosted_parameter",
            json!({"id":"offset","parameter":"offset","value":0.75,"history_group":"one-drag"}),
        )
        .unwrap();
    assert_eq!(session.state_id(), grouped_state);
    session.execute("undo", json!({})).unwrap();
    assert_eq!(session.project_document().encode().unwrap(), original);
    session.execute("redo", json!({})).unwrap();
    session
        .execute("set_hosted_execution", json!({"enabled":false}))
        .unwrap();
    assert!(session.hosted_execution_grant.is_none());
    session.execute("undo", json!({})).unwrap();
    assert!(session.hosted_execution_grant.is_none());
}
