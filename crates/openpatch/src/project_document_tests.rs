use super::*;
use serde_json::json;

fn native_patch(version: u32) -> Patch {
    serde_json::from_value(json!({
        "version": version,
        "sample_rate": 44100,
        "nodes": [
            {"id": "amp", "kind": "gain", "params": {"gain": 0.75}},
            {"id": "dc", "kind": "constant", "params": {"value": 2.0}},
            {"id": "cv", "kind": "constant", "params": {"value": 0.5}}
        ],
        "cables": [
            {"id": "native-route", "from": "dc", "to": "amp", "input": "signal",
             "color": [25, 100, 180], "input_order": 3, "output_order": 2},
            {"from": "cv", "to": "amp", "input": "cv"}
        ],
        "output": "amp"
    }))
    .unwrap()
}

fn layout() -> ProjectLayout {
    serde_json::from_value(json!({
        "version": 1, "zoom": 1.75, "pan": [-420000.5, 170000.0],
        "positions": {"dc": [-30.0, 0.0], "amp": [45.0, 380.0],
            "removed / legacy module": [900.0, -12.5]}
    }))
    .unwrap()
}

fn sample_bits(patch: &Patch) -> Vec<u32> {
    let mut engine = patch.compile().unwrap();
    let mut output = [0.0; 32];
    engine.process_block(&mut output);
    output.into_iter().map(f32::to_bits).collect()
}

#[test]
fn legacy_envelope_and_raw_patch_migrate_without_changing_core_or_scheduling() {
    for version in [1, 2] {
        let original = native_patch(version);
        let original_samples = sample_bits(&original);
        let mut expected = original.clone();
        expected.assign_missing_cable_ids().unwrap();
        let expected_view = expected.reference_view();
        let layout = layout();
        let legacy = json!({
            "format": "openpatch-project", "version": 1,
            "patch": original, "layout": layout
        });
        for (input, expected_layout) in [
            (legacy, serde_json::to_value(&layout).unwrap()),
            (
                serde_json::to_value(&original).unwrap(),
                serde_json::to_value(ProjectLayout::default()).unwrap(),
            ),
        ] {
            let document = ProjectDocument::decode(&serde_json::to_vec(&input).unwrap()).unwrap();
            let projection = document.project(PlaybackMode::Exact).unwrap();
            assert_eq!(
                document
                    .native_patch_for_legacy_api()
                    .unwrap()
                    .reference_view(),
                expected_view
            );
            assert_eq!(projection.patch.reference_view(), expected_view);
            assert_eq!(sample_bits(&projection.patch), original_samples);
            assert_eq!(
                serde_json::to_value(&document.layout).unwrap(),
                expected_layout
            );
            assert!(document.sources.is_empty());
            assert!(document.source_blobs.is_empty());
            let reopened = ProjectDocument::decode(&document.encode().unwrap()).unwrap();
            assert_eq!(
                reopened
                    .project(PlaybackMode::Exact)
                    .unwrap()
                    .patch
                    .reference_view(),
                expected_view
            );
            assert_eq!(
                serde_json::to_value(reopened.layout).unwrap(),
                expected_layout
            );
        }
    }
}

fn route(id: &str, from: &str, output: &str, to: &str, input: &str) -> Cable {
    serde_json::from_value(json!({
        "id": id, "from": from, "output": output, "to": to, "input": input
    }))
    .unwrap()
}

fn retained_native_patch() -> Patch {
    let mut patch = native_patch(2);
    patch.nodes.retain(|node| node.id != "cv");
    patch.cables.truncate(1);
    patch
}

fn source_document() -> ProjectDocument {
    let blob = SourceBlob::new(b"\0\xffopaque sample\n".to_vec()).unwrap();
    let blob_id = blob.id();
    let package = SourcePackage {
        patch: Arc::new(json!({
            "version": "2.6.4", "vendor-global": {"nested": [null, {"keep": true}]},
            "modules": [
                {"id": 7, "plugin": "Fundamental", "model": "Source", "version": "2.0",
                 "params": [{"id": 0, "value": 2.0}], "data": {"native-unknown": [1, 2]}},
                {"id": 42, "plugin": "Unavailable", "model": "Opaque", "version": "9.1",
                 "pos": [18, 1], "data": {"arbitrary": [{"deep": [false, null, "keep me"]}],
                 "bytes-reference": "modules/42/raw.bin"}}
            ],
            "cables": [
                {"id": 11, "outputModuleId": 7, "inputModuleId": 8,
                 "vendor-route": {"appearance": ["red", {"alpha": 0.75}]}},
                {"id": 12, "outputModuleId": 7, "inputModuleId": 42,
                 "vendor-route": {"nested": [1, {"future": true}]}},
                {"id": 13, "outputModuleId": 42, "inputModuleId": 8,
                 "vendor-route": {"nested": [{"future": "retained"}]}}
            ]
        })),
        assets: BTreeMap::from([("modules/42/raw.bin".into(), blob_id.clone())]),
        missing_assets: BTreeSet::from(["/external/original.wav".into()]),
    };
    let package_id = package.id().unwrap();
    let source = |index, mapping| SourceModule {
        package: package_id.clone(),
        index,
        mapping,
    };
    let mut document = ProjectDocument::from_patch(retained_native_patch(), layout()).unwrap();
    for node in &mut document.graph.nodes {
        if let DocumentNode::Native {
            node,
            source: provenance,
        } = node
        {
            if node.id == "dc" {
                *provenance = Some(source(
                    0,
                    Some(SourceMapping {
                        id: "rack-test/source".into(),
                        version: 1,
                        fidelity: MappingFidelity::SourceBehavior,
                    }),
                ));
            }
        }
    }
    document.graph.nodes.push(DocumentNode::Unresolved {
        id: "opaque-42".into(),
        source: source(1, None),
        ports: vec![
            ObservedPort {
                direction: PortDirection::Input,
                index: 0,
            },
            ObservedPort {
                direction: PortDirection::Output,
                index: 3,
            },
        ],
    });
    document.graph.cables[0].source = Some(SourceCable {
        package: package_id.clone(),
        index: 0,
    });
    for (index, cable) in [
        route("into-opaque", "dc", "out", "opaque-42", "vcv_in_0"),
        route("from-opaque", "opaque-42", "vcv_out_3", "amp", "cv"),
    ]
    .into_iter()
    .enumerate()
    {
        document.graph.cables.push(DocumentCable {
            route: cable,
            source: Some(SourceCable {
                package: package_id.clone(),
                index: index + 1,
            }),
        });
    }
    document.graph.output = Some("opaque-42".into());
    document.sources.insert(package_id, Arc::new(package));
    document.source_blobs.insert(blob_id, blob);
    document.validate_structure().unwrap();
    document
}

#[test]
fn opaque_packages_blobs_and_source_routes_survive_save_and_reopen() {
    let document = source_document();
    let encoded = document.encode().unwrap();
    let reopened = ProjectDocument::decode(&encoded).unwrap();
    let (package_id, original) = document.sources.first_key_value().unwrap();
    let recovered = &reopened.sources[package_id];
    assert_eq!(recovered.patch, original.patch);
    assert_eq!(recovered.assets, original.assets);
    assert_eq!(recovered.missing_assets, original.missing_assets);
    let blob_id = &recovered.assets["modules/42/raw.bin"];
    assert_eq!(
        reopened.source_blobs[blob_id].bytes(),
        b"\0\xffopaque sample\n"
    );
    let unresolved = reopened
        .graph
        .nodes
        .iter()
        .find(|node| node.id() == "opaque-42")
        .unwrap();
    let DocumentNode::Unresolved { source, ports, .. } = unresolved else {
        panic!("unresolved module lost")
    };
    assert_eq!(
        reopened.source_module(source).unwrap(),
        &original.patch["modules"][1]
    );
    assert_eq!(
        ports.iter().map(ObservedPort::key).collect::<Vec<_>>(),
        ["vcv_in_0", "vcv_out_3"]
    );
    let cable = reopened
        .graph
        .cables
        .iter()
        .find(|cable| cable.route.id.as_deref() == Some("from-opaque"))
        .unwrap();
    let cable_source = cable.source.as_ref().unwrap();
    assert_eq!(&cable_source.package, package_id);
    assert_eq!(cable_source.index, 2);
    assert_eq!(
        serde_json::to_value(&cable.route).unwrap(),
        serde_json::to_value(&document.graph.cables[2].route).unwrap()
    );
    assert_eq!(
        serde_json::to_value(reopened.layout).unwrap(),
        serde_json::to_value(document.layout).unwrap()
    );
}

#[test]
fn exact_refuses_unresolved_and_explicit_partial_reports_every_affected_route_and_output() {
    let document = source_document();
    let before = document.encode().unwrap();
    assert!(document
        .project(PlaybackMode::Exact)
        .unwrap_err()
        .contains("unresolved"));
    assert_eq!(document.encode().unwrap(), before);
    let projection = document.project(PlaybackMode::Partial).unwrap();
    let mut expected = retained_native_patch();
    expected.output = None;
    assert_eq!(projection.patch.reference_view(), expected.reference_view());
    assert_eq!(projection.unresolved_modules, ["opaque-42"]);
    assert_eq!(projection.omitted_output.as_deref(), Some("opaque-42"));
    assert_eq!(
        projection.omitted_routes,
        vec![
            OmittedRoute {
                cable_id: "into-opaque".into(),
                from: "dc".into(),
                to: "opaque-42".into()
            },
            OmittedRoute {
                cable_id: "from-opaque".into(),
                from: "opaque-42".into(),
                to: "amp".into()
            },
        ]
    );
    assert_eq!(document.encode().unwrap(), before);

    // An unrelated supported output stays selected during partial playback.
    let mut native_output = document.clone();
    native_output.graph.output = Some("amp".into());
    let projection = native_output.project(PlaybackMode::Partial).unwrap();
    assert_eq!(projection.patch.output.as_deref(), Some("amp"));
    assert!(projection.omitted_output.is_none());
}

fn assert_invalid_without_payload_loss(document: &ProjectDocument, message: &str) {
    let before = serde_json::to_value(document).unwrap();
    let error = document.validate_structure().unwrap_err();
    assert!(
        error.contains(message),
        "{error:?} did not contain {message:?}"
    );
    assert!(document.encode().is_err());
    assert!(document.project(PlaybackMode::Exact).is_err());
    assert!(document.project(PlaybackMode::Partial).is_err());
    assert!(ProjectDocument::decode(&serde_json::to_vec(&before).unwrap()).is_err());
    assert_eq!(serde_json::to_value(document).unwrap(), before);
    let package = document.sources.values().next().unwrap();
    assert_eq!(
        package.patch["modules"][1]["data"]["arbitrary"][0]["deep"][2],
        "keep me"
    );
    assert_eq!(
        document.source_blobs.values().next().unwrap().bytes(),
        b"\0\xffopaque sample\n"
    );
}

#[test]
fn invalid_digests_and_source_references_fail_without_discarding_retained_content() {
    let original = source_document();
    let mut bad_digest = original.clone();
    let (id, blob) = bad_digest.source_blobs.pop_first().unwrap();
    assert_ne!(id, "0".repeat(64));
    bad_digest.source_blobs.insert("0".repeat(64), blob);
    assert_invalid_without_payload_loss(&bad_digest, "digest");

    let mut bad_package_digest = original.clone();
    let (_, package) = bad_package_digest.sources.pop_first().unwrap();
    bad_package_digest.sources.insert("0".repeat(64), package);
    assert_invalid_without_payload_loss(&bad_package_digest, "package identity");

    let mut missing_blob = original.clone();
    let (_, package) = missing_blob.sources.pop_first().unwrap();
    let mut package = (*package).clone();
    package
        .assets
        .insert("modules/42/missing.bin".into(), "a".repeat(64));
    let package_id = package.id().unwrap();
    missing_blob
        .sources
        .insert(package_id.clone(), Arc::new(package));
    for node in &mut missing_blob.graph.nodes {
        match node {
            DocumentNode::Native {
                source: Some(source),
                ..
            }
            | DocumentNode::Unresolved { source, .. } => source.package = package_id.clone(),
            DocumentNode::Native { source: None, .. } => {}
        }
    }
    for cable in &mut missing_blob.graph.cables {
        if let Some(source) = &mut cable.source {
            source.package = package_id.clone();
        }
    }
    assert_invalid_without_payload_loss(&missing_blob, "blob is missing");

    let mut missing_package = original.clone();
    let DocumentNode::Unresolved { source, .. } = missing_package.graph.nodes.last_mut().unwrap()
    else {
        unreachable!()
    };
    source.package = "absent-package".into();
    assert_invalid_without_payload_loss(&missing_package, "original source module");

    let mut missing_module = original.clone();
    let DocumentNode::Unresolved { source, .. } = missing_module.graph.nodes.last_mut().unwrap()
    else {
        unreachable!()
    };
    source.index = 100;
    assert_invalid_without_payload_loss(&missing_module, "original source module");

    let mut missing_cable = original;
    missing_cable.graph.cables[2].source.as_mut().unwrap().index = 100;
    assert_invalid_without_payload_loss(&missing_cable, "original source cable");
}

#[test]
fn duplicate_and_unobserved_source_ports_are_rejected_without_payload_loss() {
    let mut duplicate = source_document();
    let DocumentNode::Unresolved { ports, .. } = duplicate.graph.nodes.last_mut().unwrap() else {
        unreachable!()
    };
    ports.push(ports[0].clone());
    assert_invalid_without_payload_loss(&duplicate, "observed source ports");

    let mut unobserved = source_document();
    let DocumentNode::Unresolved { ports, .. } = unobserved.graph.nodes.last_mut().unwrap() else {
        unreachable!()
    };
    ports.retain(|port| port.direction != PortDirection::Output);
    assert_invalid_without_payload_loss(&unobserved, "unobserved source port");
}

#[test]
fn inspection_is_compact_and_cannot_be_loaded_as_a_complete_document() {
    let document = source_document();
    assert!(document
        .native_patch_for_legacy_api()
        .unwrap_err()
        .contains("discard source content"));
    let view = document.reference_view();
    let text = serde_json::to_string(&view).unwrap();
    assert_eq!(view["format"], "openpatch-project-reference");
    assert!(!text.contains("keep me"));
    assert!(!text.contains("opaque sample"));
    assert!(!text.contains(&STANDARD.encode(b"\0\xffopaque sample\n")));
    assert_eq!(view["source_blobs"].as_object().unwrap().len(), 1);
    assert_eq!(
        view["graph"]["nodes"].as_array().unwrap().len(),
        document.graph.nodes.len()
    );
    assert!(ProjectDocument::decode(text.as_bytes()).is_err());
}

#[test]
#[ignore = "reads the untracked, user-selected OPENPATCH_VCV_REFERENCE file"]
fn selected_real_vcv_reference_is_read_without_losing_its_container() {
    let path = std::env::var("OPENPATCH_VCV_REFERENCE").expect("set reference path");
    let bytes = std::fs::read(path).unwrap();
    let archive = crate::vcv_archive::read(&bytes).unwrap();
    let patch: Value = serde_json::from_slice(&archive.patch_json).unwrap();
    let modules = patch["modules"].as_array().unwrap();
    let cables = patch["cables"].as_array().unwrap();
    assert!(!modules.is_empty());
    println!(
        "VCV reference: {} modules, {} cables, {} assets",
        modules.len(),
        cables.len(),
        archive.assets.len()
    );
}

#[test]
fn native_wavetable_asset_migration_preserves_samples_and_validates_identity() {
    let mut patch = Patch::new_v2();
    let asset = openpatch_core::default_wavetable_asset();
    patch.assets.insert(asset.id().to_owned(), asset.clone());
    let document = ProjectDocument::from_patch(patch.clone(), ProjectLayout::default()).unwrap();
    let reopened = ProjectDocument::decode(&document.encode().unwrap()).unwrap();
    let projected = reopened.project(PlaybackMode::Exact).unwrap().patch;
    assert_eq!(projected.reference_view(), patch.reference_view());
    assert_eq!(
        projected.assets[asset.id()]
            .decoded_samples()
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>(),
        asset
            .decoded_samples()
            .iter()
            .map(|x| x.to_bits())
            .collect::<Vec<_>>()
    );
    let mut invalid = reopened;
    invalid.graph.assets.insert("forged".into(), asset);
    assert!(invalid.validate_structure().is_err());
    assert!(invalid.encode().is_err());
}

#[test]
fn carrier_allows_native_editing_without_granting_exact_playback() {
    let document = source_document();
    let before = document.encode().unwrap();
    let carrier = document.state_carrier().unwrap();
    assert_eq!(carrier.unresolved_modules, ["opaque-42"]);
    assert_eq!(carrier.omitted_routes.len(), 2);
    assert_eq!(carrier.omitted_output.as_deref(), Some("opaque-42"));
    assert!(document.project(PlaybackMode::Exact).is_err());

    let mut candidate = carrier.patch;
    let node = candidate.nodes.iter_mut().find(|n| n.id == "dc").unwrap();
    node.params.insert("value".into(), 4.25);
    node.bypassed = true;
    candidate.cables[0].color = Some([10, 20, 30]);
    let edited = document
        .with_native_patch(candidate, &BTreeMap::new())
        .unwrap();
    let carrier = edited.state_carrier().unwrap();
    assert_eq!(
        carrier
            .patch
            .nodes
            .iter()
            .find(|n| n.id == "dc")
            .unwrap()
            .params["value"],
        4.25
    );
    assert!(
        carrier
            .patch
            .nodes
            .iter()
            .find(|n| n.id == "dc")
            .unwrap()
            .bypassed
    );
    assert_eq!(carrier.omitted_routes.len(), 2);
    assert_eq!(carrier.omitted_output.as_deref(), Some("opaque-42"));
    assert!(edited.project(PlaybackMode::Exact).is_err());
    assert_eq!(
        edited
            .graph
            .cables
            .iter()
            .find(|c| c.route.id.as_deref() == Some("native-route"))
            .unwrap()
            .source
            .as_ref()
            .unwrap()
            .index,
        0
    );
    for id in ["into-opaque", "from-opaque"] {
        assert_eq!(
            serde_json::to_value(
                edited
                    .graph
                    .cables
                    .iter()
                    .find(|c| c.route.id.as_deref() == Some(id))
                    .unwrap()
            )
            .unwrap(),
            serde_json::to_value(
                document
                    .graph
                    .cables
                    .iter()
                    .find(|c| c.route.id.as_deref() == Some(id))
                    .unwrap()
            )
            .unwrap()
        );
    }
    let original_source = document
        .graph
        .nodes
        .iter()
        .find(|n| n.id() == "dc")
        .unwrap()
        .source()
        .unwrap();
    let edited_source = edited
        .graph
        .nodes
        .iter()
        .find(|n| n.id() == "dc")
        .unwrap()
        .source()
        .unwrap();
    assert_eq!(
        serde_json::to_value(original_source).unwrap(),
        serde_json::to_value(edited_source).unwrap()
    );
    let reopened = ProjectDocument::decode(&edited.encode().unwrap()).unwrap();
    assert_eq!(reopened.encode().unwrap(), edited.encode().unwrap());
    // A history entry can keep the prior document without mutation or copying
    // immutable package/blob payloads.
    assert_eq!(document.encode().unwrap(), before);
    let package_id = original_source.package.as_str();
    assert!(Arc::ptr_eq(
        &document.sources[package_id],
        &edited.sources[package_id]
    ));
    assert!(Arc::ptr_eq(
        &document.source_blobs.values().next().unwrap().0,
        &edited.source_blobs.values().next().unwrap().0
    ));
}

#[test]
fn deleting_native_node_removes_only_its_incident_routes_and_layout() {
    let mut document = source_document();
    document
        .layout
        .positions
        .insert("opaque-42".into(), [800.0, 0.0]);
    let before = document.encode().unwrap();
    let mut candidate = document.state_carrier().unwrap().patch;
    candidate.nodes.retain(|n| n.id != "dc");
    candidate.cables.retain(|c| c.from != "dc" && c.to != "dc");
    let edited = document
        .with_native_patch(candidate, &BTreeMap::new())
        .unwrap();
    assert!(edited.graph.nodes.iter().all(|n| n.id() != "dc"));
    assert!(!edited.layout.positions.contains_key("dc"));
    assert_eq!(
        edited.layout.positions["amp"],
        document.layout.positions["amp"]
    );
    assert_eq!(edited.layout.positions["opaque-42"], [800.0, 0.0]);
    assert!(edited
        .layout
        .positions
        .contains_key("removed / legacy module"));
    let carrier = edited.state_carrier().unwrap();
    assert!(carrier.patch.cables.is_empty());
    assert_eq!(carrier.omitted_routes.len(), 1);
    assert_eq!(carrier.omitted_routes[0].cable_id, "from-opaque");
    assert_eq!(carrier.omitted_output.as_deref(), Some("opaque-42"));
    assert_eq!(edited.source_blobs.len(), document.source_blobs.len());
    assert_eq!(edited.sources.len(), document.sources.len());
    assert_eq!(document.encode().unwrap(), before);
}

#[test]
fn duplicate_retains_source_assets_and_original_ids_with_fresh_native_identity() {
    let document = source_document();
    let mut candidate = document.state_carrier().unwrap().patch;
    let mut duplicate = candidate
        .nodes
        .iter()
        .find(|n| n.id == "dc")
        .unwrap()
        .clone();
    duplicate.id = "dc-copy".into();
    duplicate.params.insert("value".into(), 9.0);
    candidate.nodes.push(duplicate);
    let mut fresh = candidate
        .nodes
        .iter()
        .find(|n| n.id == "dc")
        .unwrap()
        .clone();
    fresh.id = "fresh".into();
    candidate.nodes.push(fresh);
    let mut copied_cable = candidate.cables[0].clone();
    copied_cable.id = Some("copied-route".into());
    copied_cable.from = "dc-copy".into();
    candidate.cables.push(copied_cable);
    // Editing a retained route changes its current endpoints while its source
    // package still describes the original Rack cable.
    candidate.cables[0].from = "dc-copy".into();
    let asset = openpatch_core::default_wavetable_asset();
    candidate
        .assets
        .insert(asset.id().to_owned(), asset.clone());
    let copies = BTreeMap::from([("dc-copy".into(), "dc".into())]);
    let edited = document
        .with_native_patch(candidate.clone(), &copies)
        .unwrap();
    let copied = edited
        .graph
        .nodes
        .iter()
        .find(|n| n.id() == "dc-copy")
        .unwrap();
    assert_eq!(
        edited.source_module(copied.source().unwrap()).unwrap()["id"],
        7
    );
    assert!(copied.source().unwrap().mapping.is_some());
    assert!(edited
        .graph
        .nodes
        .iter()
        .find(|n| n.id() == "fresh")
        .unwrap()
        .source()
        .is_none());
    assert!(edited
        .graph
        .cables
        .iter()
        .find(|c| c.route.id.as_deref() == Some("copied-route"))
        .unwrap()
        .source
        .is_none());
    let original_route = edited
        .graph
        .cables
        .iter()
        .find(|c| c.route.id.as_deref() == Some("native-route"))
        .unwrap();
    assert_eq!(original_route.source.as_ref().unwrap().index, 0);
    assert_eq!(original_route.route.from, "dc-copy");
    let original_source = original_route.source.as_ref().unwrap();
    assert_eq!(
        edited.sources[&original_source.package].patch["cables"][original_source.index]
            ["outputModuleId"],
        7
    );
    assert_eq!(
        edited.graph.assets[asset.id()].decoded_samples(),
        asset.decoded_samples()
    );
    assert_eq!(
        edited.state_carrier().unwrap().patch.reference_view(),
        candidate.reference_view()
    );
    let reopened = ProjectDocument::decode(&edited.encode().unwrap()).unwrap();
    assert_eq!(reopened.encode().unwrap(), edited.encode().unwrap());
    let package = reopened.sources.values().next().unwrap();
    assert_eq!(package.patch["modules"][0]["id"], 7);
    assert_eq!(
        reopened.source_blobs[&package.assets["modules/42/raw.bin"]].bytes(),
        b"\0\xffopaque sample\n"
    );
}

#[test]
fn changed_native_output_wins_over_preserved_unresolved_selection() {
    let document = source_document();
    let mut candidate = document.state_carrier().unwrap().patch;
    candidate.output = Some("amp".into());
    let edited = document
        .with_native_patch(candidate, &BTreeMap::new())
        .unwrap();
    assert_eq!(edited.graph.output.as_deref(), Some("amp"));
    assert!(edited.state_carrier().unwrap().omitted_output.is_none());
    let mut clear = edited.state_carrier().unwrap().patch;
    clear.output = None;
    let cleared = edited.with_native_patch(clear, &BTreeMap::new()).unwrap();
    assert!(cleared.graph.output.is_none());
}

#[test]
fn native_overlay_rejects_ambiguous_ids_stale_copies_and_scheduling_changes_atomically() {
    let document = source_document();
    let before = document.encode().unwrap();
    let patch = document.state_carrier().unwrap().patch;
    for copies in [
        BTreeMap::from([("dc".into(), "amp".into())]),
        BTreeMap::from([("missing".into(), "dc".into())]),
        BTreeMap::from([("new".into(), "missing".into())]),
        BTreeMap::from([("new".into(), "opaque-42".into())]),
    ] {
        let mut candidate = patch.clone();
        let mut node = candidate.nodes[0].clone();
        node.id = "new".into();
        candidate.nodes.push(node);
        assert!(document.with_native_patch(candidate, &copies).is_err());
    }
    let mut collision = patch.clone();
    collision.nodes[0].id = "opaque-42".into();
    collision.cables.clear();
    assert!(document
        .with_native_patch(collision, &BTreeMap::new())
        .is_err());
    let mut collision = patch.clone();
    collision.cables[0].id = Some("into-opaque".into());
    assert!(document
        .with_native_patch(collision, &BTreeMap::new())
        .is_err());
    let mut missing_id = patch.clone();
    missing_id.cables[0].id = None;
    assert!(document
        .with_native_patch(missing_id, &BTreeMap::new())
        .is_err());
    let mut duplicate_id = patch.clone();
    duplicate_id.nodes.push(duplicate_id.nodes[0].clone());
    assert!(document
        .with_native_patch(duplicate_id, &BTreeMap::new())
        .is_err());
    let mut dangling = patch.clone();
    dangling.cables[0].from = "gone".into();
    assert!(document
        .with_native_patch(dangling, &BTreeMap::new())
        .is_err());
    let mut scheduling = patch;
    scheduling.version = 1;
    assert!(document
        .with_native_patch(scheduling, &BTreeMap::new())
        .is_err());
    assert_eq!(document.encode().unwrap(), before);
}
