mod hosted_activation {
    use super::*;
    use openpatch::binary_host::{control::HostedControl, Fence};
    use openpatch::control::{DocumentExecutionRequest, HostLifecycleTicket, Session};
    use openpatch::project_document::{
        hosted::{HostedIdentity, HostedModule},
        DocumentNode, ProjectDocument, ProjectLayout,
    };

    fn document() -> ProjectDocument {
        let patch: Patch = serde_json::from_value(json!({
            "version":2,"sample_rate":48000,
            "nodes":[{"id":"source","kind":"constant","params":{"value":0.5}},
                     {"id":"amp","kind":"stock_vca","params":{"level":0.5,"response":1}}],
            "cables":[],"output":"amp"
        }))
        .unwrap();
        let mut document = ProjectDocument::from_patch(patch, ProjectLayout::default()).unwrap();
        document.graph.nodes.push(DocumentNode::Hosted {
            id: "binary".into(),
            source: None,
            module: HostedModule::offset(
                HostedIdentity {
                    plugin: "Bogaudio".into(),
                    model: "Bogaudio-Offset".into(),
                    version: "2.6.47".into(),
                    rack_version: "2.6.6".into(),
                    plugin_sha256:
                        "264e55e1389274c3873439372c5c51f48f63168bd6c98fb2c6ffec170224b6f6".into(),
                    runtime_sha256:
                        "55dd40f7dff09a6ed7b93374b150f33931034d3754c128da7b209ea37b6aaf1c".into(),
                },
                0.,
                0.5,
                true,
                false,
            )
            .unwrap(),
        });
        for route in [
            json!({"id":"host-in","from":"source","to":"binary","output":"out","input":"in"}),
            json!({"id":"host-out","from":"binary","to":"amp","output":"out","input":"signal"}),
        ] {
            document
                .graph
                .cables
                .push(serde_json::from_value(json!({"route":route})).unwrap());
        }
        document
    }

    fn session_ticket() -> (tempfile::TempDir, Session, Arc<HostLifecycleTicket>) {
        let directory = tempfile::tempdir().unwrap();
        let mut session = Session::new(directory.path()).unwrap();
        session.replace_project_document(document(), None).unwrap();
        session.request_delayed_hosted_execution(None).unwrap();
        let ticket = session.audio_snapshot().unwrap();
        (directory, session, ticket)
    }

    fn control(
        ticket: &HostLifecycleTicket,
        incarnation: u64,
    ) -> (
        Arc<HostedControl>,
        openpatch::binary_host::control::HostedControlReader,
    ) {
        let DocumentExecutionRequest::HostedDelayed(grant) = &ticket.execution else {
            panic!("hosted grant missing");
        };
        HostedControl::new(
            grant.clone(),
            &ticket.document,
            Fence {
                incarnation,
                generation: ticket.document_generation,
                revision: ticket.revision.max(1),
            },
            ticket.revision,
        )
        .unwrap()
    }

    fn engine(
        ticket: &HostLifecycleTicket,
    ) -> (
        Engine,
        openpatch::binary_host::graph::HostedCompilationProof,
    ) {
        let DocumentExecutionRequest::HostedDelayed(grant) = &ticket.execution else {
            panic!("hosted grant missing");
        };
        let (engine, _external, proof) = grant
            .prepare(&ticket.document, ticket.document_generation)
            .unwrap()
            .compile_attested(&ticket.banks)
            .unwrap();
        (engine, proof)
    }

    fn publish_running(performance: &Performance, incarnation: u64) {
        // The runner publishes these only after processing its own incarnation.
        performance.shared.hosted_status.store(2, Ordering::Release);
        performance
            .shared
            .hosted_status_incarnation
            .store(incarnation, Ordering::Release);
    }

    #[test]
    fn full_hosted_plan_never_authorizes_its_native_carrier_and_waits_for_running_ack() {
        let (_directory, _session, ticket) = session_ticket();
        let (performance, reader, _timing) = Performance::new(48_000);
        let (engine, proof) = engine(&ticket);
        let carrier = ticket.document.state_carrier().unwrap().patch;
        performance.sync_patch(&carrier, ticket.revision);
        let native = performance
            .plan(&carrier, &engine, ticket.revision)
            .unwrap();
        let (hosted, _hosted_reader) = control(&ticket, 501);
        let plan = performance
            .plan_hosted(&ticket, &engine, hosted.clone(), &proof)
            .unwrap();
        assert!(
            !reader.execution_permitted(&plan),
            "preparation alone cannot activate"
        );
        performance
            .register_hosted_control(&ticket, hosted.clone())
            .unwrap();
        assert!(!reader.execution_permitted(&native));
        assert!(reader.execution_permitted(&plan));
        assert_eq!(
            performance.snapshot()["execution"]["disposition"],
            "suspended_unresolved"
        );
        assert!(!performance.execution_available());
        reader.acknowledge_execution(&plan);
        assert!(
            !performance.execution_available(),
            "priming is not available"
        );
        publish_running(&performance, 500);
        assert!(
            !performance.execution_available(),
            "another runner cannot publish availability"
        );
        publish_running(&performance, 501);
        assert!(performance.execution_available());
        assert_eq!(performance.shared.execution_ack.load(Ordering::Acquire), 0);
        hosted.close();
        assert!(!reader.execution_permitted(&plan));
        assert!(!performance.execution_available());
        assert_eq!(performance.snapshot()["available"], false);
    }

    #[test]
    fn hosted_plan_rejects_missing_grant_wrong_document_and_omitted_engine() {
        let (_directory, _session, ticket) = session_ticket();
        let (performance, _reader, _timing) = Performance::new(48_000);
        let (engine, proof) = engine(&ticket);
        let (hosted, _hosted_reader) = control(&ticket, 502);
        let carrier = ticket.document.state_carrier().unwrap().patch;
        let native_engine = carrier.compile_with_wave_banks(&ticket.banks).unwrap();
        assert!(performance
            .plan_hosted(&ticket, &native_engine, hosted.clone(), &proof)
            .is_err());
        let DocumentExecutionRequest::HostedDelayed(grant) = &ticket.execution else {
            panic!("hosted grant missing");
        };
        let (same_ports_other_engine, _, _) = grant
            .prepare(&ticket.document, ticket.document_generation)
            .unwrap()
            .compile_attested(&ticket.banks)
            .unwrap();
        assert!(performance
            .plan_hosted(&ticket, &same_ports_other_engine, hosted.clone(), &proof)
            .is_err());
        let mut wrong = (*ticket).clone();
        wrong.document_generation += 1;
        assert!(performance
            .plan_hosted(&wrong, &engine, hosted.clone(), &proof)
            .is_err());
        wrong = (*ticket).clone();
        Arc::make_mut(&mut wrong.document).graph.output = Some("source".into());
        assert!(performance
            .plan_hosted(&wrong, &engine, hosted.clone(), &proof)
            .is_err());
        wrong = (*ticket).clone();
        wrong.execution = DocumentExecutionRequest::Native(ExecutionDisposition::RunnablePartial);
        assert!(performance
            .plan_hosted(&wrong, &engine, hosted, &proof)
            .is_err());
    }

    #[test]
    fn compatible_hosted_edits_keep_permission_and_native_intent_but_current_revocation_closes_it()
    {
        let (_directory, mut session, ticket) = session_ticket();
        let (performance, reader, _timing) = Performance::new(48_000);
        session
            .set_live_module_events(Some(performance.clone()))
            .unwrap();
        let (engine, proof) = engine(&ticket);
        let (hosted, _hosted_reader) = control(&ticket, 503);
        let plan = performance
            .plan_hosted(&ticket, &engine, hosted.clone(), &proof)
            .unwrap();
        performance
            .register_hosted_control(&ticket, hosted.clone())
            .unwrap();
        session.acknowledge_audio_snapshot(ticket.id);
        let native_intent = performance.native_intent_stamp();
        session
            .execute(
                "set_hosted_parameter",
                json!({
                    "id":"binary","parameter":"scale","value":0.25
                }),
            )
            .unwrap();
        let current = session.audio_snapshot().unwrap();
        assert_eq!(performance.native_intent_stamp(), native_intent);
        assert!(reader.execution_permitted(&plan));
        let carrier = current.document.state_carrier().unwrap().patch;
        performance.sync_document(DocumentExecutionContext {
            document: &ticket.document,
            document_generation: ticket.document_generation,
            request: DocumentExecutionRequest::Native(ExecutionDisposition::SuspendedUnresolved),
            native_carrier: &carrier,
            revision: ticket.revision,
            fence_native: true,
        });
        assert!(
            hosted.is_open(),
            "an old context cannot close a newer binding"
        );
        assert!(reader.execution_permitted(&plan));
        // Explicit equal native assignments must still force the controller to
        // prepare the native pending intent, even with an unchanged patch JSON.
        let assignments = BTreeMap::from([("amp".into(), BTreeMap::from([("level".into(), 0.5)]))]);
        performance.sync_document_with_intents(
            DocumentExecutionContext {
                document: &current.document,
                document_generation: current.document_generation,
                request: current.execution.clone(),
                native_carrier: &carrier,
                revision: current.revision,
                fence_native: true,
            },
            &assignments,
            &[],
        );
        assert_ne!(performance.native_intent_stamp(), native_intent);
        performance.sync_document(DocumentExecutionContext {
            document: &current.document,
            document_generation: current.document_generation,
            request: DocumentExecutionRequest::Native(ExecutionDisposition::SuspendedUnresolved),
            native_carrier: &carrier,
            revision: current.revision,
            fence_native: true,
        });
        assert!(!hosted.is_open());
        assert!(!reader.execution_permitted(&plan));
        assert_eq!(performance.shared.hosted_request.load(Ordering::Acquire), 0);
        assert!(!performance.execution_available());
    }

    #[test]
    fn fresh_hosted_activation_resets_fault_and_requires_its_own_status_and_ack() {
        let (_directory, _session, ticket) = session_ticket();
        let (performance, reader, _timing) = Performance::new(48_000);
        let (engine, proof) = engine(&ticket);
        let (old, _old_reader) = control(&ticket, 504);
        let old_plan = performance
            .plan_hosted(&ticket, &engine, old.clone(), &proof)
            .unwrap();
        performance
            .register_hosted_control(&ticket, old.clone())
            .unwrap();
        publish_running(&performance, 504);
        reader.acknowledge_execution(&old_plan);
        assert!(performance.execution_available());
        reader
            .fail_hosted_execution(openpatch::binary_host::runner::RunnerFault::UnsupportedProfile);
        assert!(!performance.execution_available());
        assert!(!reader.execution_permitted(&old_plan));
        let (fresh, _fresh_reader) = control(&ticket, 505);
        let plan = performance
            .plan_hosted(&ticket, &engine, fresh.clone(), &proof)
            .unwrap();
        performance.register_hosted_control(&ticket, fresh).unwrap();
        assert!(!old.is_open());
        assert!(!reader.execution_permitted(&old_plan));
        assert!(reader.execution_permitted(&plan));
        assert!(!performance.execution_available());
        publish_running(&performance, 504);
        reader.acknowledge_execution(&plan);
        assert!(!performance.execution_available());
        publish_running(&performance, 505);
        assert!(performance.execution_available());
        performance
            .request_document_execution(ticket.revision + 1, ExecutionDisposition::RunnablePartial);
        assert!(
            !reader.execution_permitted(&plan),
            "native resume cannot authorize a hosted plan"
        );
    }
}
