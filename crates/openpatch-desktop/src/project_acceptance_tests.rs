//! Filesystem acceptance tests through the native document/store boundary.
//! Every file is owned by a disposable temporary workspace.
use crate::project::{Document, Store};
use openpatch_core::{Cable, Node, Patch};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn layout() -> Value {
    json!({"version":1,"zoom":0.8_f32,"pan":[-92.0,37.0],"positions":{"dc":[128.0,380.0]}})
}
fn patch(version: u32, value: f32) -> Patch {
    Patch {
        version,
        nodes: vec![Node {
            content: None,
            score: None,
            bypassed: false,
            id: "dc".into(),
            kind: "constant".into(),
            params: [("value".into(), value)].into(),
        }],
        output: Some("dc".into()),
        ..Patch::new_v2()
    }
}
fn document(value: f32) -> Document {
    Document::new(patch(2, value), layout()).unwrap()
}
fn store() -> (TempDir, Store) {
    let directory = TempDir::new().unwrap();
    let store = Store::new(directory.path()).unwrap();
    (directory, store)
}
fn read(directory: &TempDir, name: &str) -> Vec<u8> {
    fs::read(directory.path().join(name)).unwrap()
}
fn native_files(path: &Path) -> Vec<PathBuf> {
    let mut names: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            let name = path.file_name().unwrap().to_string_lossy();
            name.starts_with('.') && name.ends_with(".openpatch")
        })
        .collect();
    names.sort();
    names
}

#[test]
fn native_single_file_roundtrip_and_save_existing_include_graph_and_layout() {
    let (directory, store) = store();
    let original = document(1.25);
    let token = store.save_new("voice.openpatch", &original).unwrap();
    let bytes = read(&directory, "voice.openpatch");
    let envelope: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(envelope["format"], "openpatch-project");
    assert_eq!(envelope["version"], 2);
    let decoded = openpatch::project_document::ProjectDocument::decode(&bytes).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap(),
        serde_json::to_value(original.project()).unwrap(),
        "typed graph/layout/source roundtrip preserves f32 values exactly"
    );
    let loaded = store.load("voice.openpatch").unwrap();
    assert!(!loaded.imported);
    assert!(loaded.token.is_some());
    assert!(original.equivalent(&loaded.document));
    assert!(!directory
        .path()
        .join("voice.openpatch.layout.json")
        .exists());

    let edited = document(-2.5);
    let mut layout = edited.layout().clone();
    layout["pan"] = json!([19.0, -81.0]);
    let edited = edited.with_layout(layout).unwrap();
    let second_token = store
        .save_existing("voice.openpatch", &edited, &token)
        .unwrap();
    assert_ne!(read(&directory, "voice.openpatch"), bytes);
    assert!(edited.equivalent(&store.load("voice.openpatch").unwrap().document));
    // A subsequent save uses the new fingerprint, rather than a stale original.
    store
        .save_existing("voice.openpatch", &original, &second_token)
        .unwrap();
    assert!(original.equivalent(&store.load("voice.openpatch").unwrap().document));
}

#[test]
fn rack_import_is_read_only_and_first_native_save_preserves_full_unresolved_document() {
    use crate::document_state::{apply_document, DocumentState};
    use openpatch::{control::Session, project_document::ExecutionDisposition};
    let (directory, store) = store();
    let raw = serde_json::to_vec(&json!({
        "version":"2.6.4", "opaque_global":{"retained":true},
        "modules":[{"id":42,"plugin":"MissingVendor","model":"OpaqueVoice",
            "version":"1.2.3","pos":[1,0],"data":{"opaque":[1,2,3]}}],
        "cables":[]
    }))
    .unwrap();
    fs::write(directory.path().join("source.vcv"), &raw).unwrap();
    let imported = store.load("source.vcv").unwrap();
    assert!(imported.imported && imported.token.is_none());
    assert_eq!(imported.document.project().graph.nodes.len(), 1);
    assert!(imported.document.patch().nodes.is_empty());
    let shared = Session::new(directory.path()).unwrap().into_shared();
    let applied = apply_document(&shared, 0, &imported.document).unwrap();
    let live = shared.lock().unwrap();
    assert_eq!(
        live.execution_disposition(),
        ExecutionDisposition::SuspendedUnresolved
    );
    let mut state = DocumentState::new(live.project_document(), live.revision(), &applied.rack);
    state.mark_unsaved();
    assert!(state.dirty());
    drop(live);
    store
        .save_new("converted.openpatch", &imported.document)
        .unwrap();
    let reopened = store.load("converted.openpatch").unwrap();
    assert!(!reopened.imported && reopened.token.is_some());
    assert_eq!(
        reopened.document.encode().unwrap(),
        imported.document.encode().unwrap()
    );
    assert_eq!(read(&directory, "source.vcv"), raw);
    let path = directory.path().join("oversized.vcv");
    fs::File::create(&path)
        .unwrap()
        .set_len(16 * 1_048_576 + 1)
        .unwrap();
    assert!(store.load("oversized.vcv").unwrap_err().contains("16 MiB"));
    assert_eq!(fs::metadata(path).unwrap().len(), 16 * 1_048_576 + 1);
}

#[test]
fn legacy_v1_and_v2_import_preserve_sources_and_sidecars_without_native_save_token() {
    let (directory, store) = store();
    for version in [1, 2] {
        let name = format!("legacy-{version}.json");
        let mut original_patch = patch(version, version as f32);
        if version == 2 {
            original_patch.nodes[0].kind = "poly_constant".into();
            original_patch.nodes[0].params = [
                ("channels".into(), 2.0),
                ("v1".into(), 0.25),
                ("v2".into(), -0.75),
            ]
            .into();
            original_patch.nodes.extend([
                Node {
                    content: None,
                    score: None,
                    bypassed: false,
                    id: "a".into(),
                    kind: "mult".into(),
                    params: Default::default(),
                },
                Node {
                    content: None,
                    score: None,
                    bypassed: false,
                    id: "b".into(),
                    kind: "mult".into(),
                    params: Default::default(),
                },
            ]);
            original_patch.cables.extend([
                Cable {
                    input_order: None,
                    output_order: None,
                    color: None,
                    id: None,
                    from: "dc".into(),
                    output: "out".into(),
                    to: "a".into(),
                    input: "signal".into(),
                },
                Cable {
                    input_order: None,
                    output_order: None,
                    color: None,
                    id: None,
                    from: "a".into(),
                    output: "out8".into(),
                    to: "b".into(),
                    input: "signal".into(),
                },
            ]);
            original_patch.output = Some("b".into());
            original_patch.output_port = "out3".into();
        }
        let mut graph = serde_json::to_value(&original_patch).unwrap();
        if version == 1 {
            graph.as_object_mut().unwrap().remove("version");
        }
        let graph_bytes = serde_json::to_vec_pretty(&graph).unwrap();
        let sidecar_name = format!("{name}.layout.json");
        let layout_bytes = serde_json::to_vec_pretty(&layout()).unwrap();
        fs::write(directory.path().join(&name), &graph_bytes).unwrap();
        fs::write(directory.path().join(&sidecar_name), &layout_bytes).unwrap();
        let loaded = store.load(&name).unwrap();
        assert!(loaded.imported);
        assert!(loaded.token.is_none());
        assert_eq!(loaded.document.patch().version, version);
        original_patch.assign_missing_cable_ids().unwrap();
        assert_eq!(
            serde_json::to_value(loaded.document.patch()).unwrap(),
            serde_json::to_value(&original_patch).unwrap()
        );
        assert_eq!(loaded.document.layout(), &layout());
        assert_eq!(read(&directory, &name), graph_bytes);
        assert_eq!(read(&directory, &sidecar_name), layout_bytes);
        let native_name = format!("converted-{version}.openpatch");
        store.save_new(&native_name, &loaded.document).unwrap();
        assert!(loaded
            .document
            .equivalent(&store.load(&native_name).unwrap().document));
        assert_eq!(read(&directory, &name), graph_bytes);
    }
    fs::write(
        directory.path().join("without-layout.json"),
        serde_json::to_vec(&patch(2, 3.0)).unwrap(),
    )
    .unwrap();
    let loaded = store.load("without-layout.json").unwrap();
    let mut rack = crate::rack::Rack::default();
    rack.restore_layout(loaded.document.layout()).unwrap();
    assert!(loaded.imported);
}

#[test]
fn conflicting_save_rejects_external_changes_and_tokens_cannot_target_another_file() {
    let (directory, store) = store();
    let first = document(1.0);
    let token = store.save_new("voice.openpatch", &first).unwrap();
    let external = document(2.0).encode().unwrap();
    fs::write(directory.path().join("voice.openpatch"), &external).unwrap();
    assert!(store
        .save_existing("voice.openpatch", &document(3.0), &token)
        .is_err());
    assert_eq!(read(&directory, "voice.openpatch"), external);
    store.save_new("other.openpatch", &first).unwrap();
    let other = read(&directory, "other.openpatch");
    assert!(store
        .save_existing("other.openpatch", &document(4.0), &token)
        .is_err());
    assert_eq!(read(&directory, "other.openpatch"), other);
    assert!(store.save_new("other.openpatch", &document(4.0)).is_err());
    assert_eq!(read(&directory, "other.openpatch"), other);
}

#[test]
fn invalid_document_save_and_non_file_targets_leave_existing_data_intact() {
    let (directory, store) = store();
    store.save_new("voice.openpatch", &document(1.0)).unwrap();
    let before = read(&directory, "voice.openpatch");
    let invalid = document(2.0);
    let mut layout = invalid.layout().clone();
    layout["zoom"] = json!(900.0);
    assert!(invalid.with_layout(layout).is_err());
    assert_eq!(read(&directory, "voice.openpatch"), before);
    assert!(!directory.path().join("invalid.openpatch").exists());
    let mut invalid = patch(2, 2.0);
    invalid.output = Some("missing".into());
    assert!(Document::new(invalid, self::layout()).is_err());
    assert_eq!(read(&directory, "voice.openpatch"), before);
    fs::create_dir(directory.path().join("directory.openpatch")).unwrap();
    assert!(store.load("directory.openpatch").is_err());
    assert!(store
        .save_new("directory.openpatch", &document(4.0))
        .is_err());
    assert!(directory.path().join("directory.openpatch").is_dir());
}

#[test]
fn malformed_unknown_or_unsupported_document_and_layout_fields_are_rejected() {
    let (directory, store) = store();
    let native: Value = serde_json::from_slice(&document(1.0).encode().unwrap()).unwrap();
    let mut variants = Vec::new();
    for (key, value) in [
        ("format", json!("another-app")),
        ("version", json!(99)),
        ("unknown", json!(true)),
        ("patch", json!({"version":99})),
        ("layout", Value::Null),
    ] {
        let mut value_document = native.clone();
        value_document[key] = value;
        variants.push(value_document);
    }
    for bad_layout in [
        json!({"version":2,"zoom":1,"pan":[0,0],"positions":{}}),
        json!({"version":1,"zoom":1,"pan":[0,0],"positions":{},"unknown":1}),
        json!({"version":1,"zoom":0.2,"pan":[0,0],"positions":{}}),
        json!({"version":1,"zoom":1,"pan":[0],"positions":{}}),
        json!({"version":1,"zoom":1,"pan":[0,0],"positions":{"dc":[0,"bad"]}}),
        json!({"version":1,"zoom":1,"pan":[0,0],"positions":{"dc":[0,1e30]}}),
    ] {
        assert!(Document::new(patch(2, 1.0), bad_layout.clone()).is_err());
        let mut value_document = native.clone();
        value_document["layout"] = bad_layout;
        variants.push(value_document);
    }
    for (index, value) in variants.iter().enumerate() {
        let name = format!("malformed-{index}.openpatch");
        let bytes = serde_json::to_vec(value).unwrap();
        fs::write(directory.path().join(&name), &bytes).unwrap();
        assert!(store.load(&name).is_err(), "accepted {value}");
        assert_eq!(read(&directory, &name), bytes);
    }
    fs::write(directory.path().join("broken.openpatch"), b"{incomplete").unwrap();
    assert!(store.load("broken.openpatch").is_err());
}

#[test]
fn malformed_present_legacy_layout_rejects_import_without_modifying_either_file() {
    let (directory, store) = store();
    let graph = serde_json::to_vec(&patch(1, 2.0)).unwrap();
    fs::write(directory.path().join("legacy.json"), &graph).unwrap();
    let invalid = br#"{"version":1,"zoom":99,"pan":[0,0],"positions":{}}"#;
    fs::write(directory.path().join("legacy.json.layout.json"), invalid).unwrap();
    assert!(store.load("legacy.json").is_err());
    assert_eq!(read(&directory, "legacy.json"), graph);
    assert_eq!(read(&directory, "legacy.json.layout.json"), invalid);
}

#[test]
fn basename_extension_and_size_limits_apply_to_native_and_legacy_reads_and_writes() {
    let (directory, store) = store();
    for name in [
        "",
        ".",
        "..",
        "../escape.openpatch",
        "/absolute.openpatch",
        "nested/voice.openpatch",
        "nested\\voice.openpatch",
        "unicode-é.openpatch",
        "wrong.json",
        "wrong.txt",
    ] {
        assert!(
            store.save_new(name, &document(1.0)).is_err(),
            "accepted save name {name:?}"
        );
    }
    let too_long = format!("{}.openpatch", "a".repeat(111));
    assert_eq!(too_long.len(), 121);
    assert!(store.save_new(&too_long, &document(1.0)).is_err());
    let longest = format!("{}.openpatch", "a".repeat(110));
    store.save_new(&longest, &document(1.0)).unwrap();
    assert!(store.load(&longest).is_ok());
    for name in [
        "../escape.json",
        "/absolute.json",
        "nested/voice.json",
        "nested\\voice.json",
        "unicode-é.json",
    ] {
        assert!(store.load(name).is_err());
    }
    // Sparse oversized files exercise metadata rejection without allocating a
    // pair of 64MiB test buffers; error text proves size wins over JSON parsing.
    for (name, prefix) in [
        ("large.openpatch", document(1.0).encode().unwrap()),
        ("large.json", serde_json::to_vec(&patch(1, 1.0)).unwrap()),
    ] {
        let path = directory.path().join(name);
        fs::write(&path, prefix).unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(path)
            .unwrap()
            .set_len(openpatch_core::MAX_PATCH_FILE_BYTES as u64 + 1)
            .unwrap();
        assert!(store.load(name).unwrap_err().contains("64 MiB"));
    }
    fs::write(
        directory.path().join("sidecar.json"),
        serde_json::to_vec(&patch(2, 1.0)).unwrap(),
    )
    .unwrap();
    let mut sidecar = serde_json::to_vec(&layout()).unwrap();
    sidecar.resize(1024 * 1024 + 1, b' ');
    fs::write(directory.path().join("sidecar.json.layout.json"), sidecar).unwrap();
    assert!(store.load("sidecar.json").is_err());
}

#[cfg(unix)]
#[test]
fn symlinked_native_legacy_and_sidecar_files_are_never_followed_or_replaced() {
    use std::os::unix::fs::symlink;
    let (directory, store) = store();
    let outside = TempDir::new().unwrap();
    let target = outside.path().join("original.openpatch");
    let bytes = document(1.0).encode().unwrap();
    fs::write(&target, &bytes).unwrap();
    symlink(&target, directory.path().join("linked.openpatch")).unwrap();
    assert!(store.load("linked.openpatch").is_err());
    assert!(store.save_new("linked.openpatch", &document(2.0)).is_err());
    assert_eq!(fs::read(&target).unwrap(), bytes);
    let graph = serde_json::to_vec(&patch(1, 1.0)).unwrap();
    let graph_path = outside.path().join("legacy.json");
    fs::write(&graph_path, &graph).unwrap();
    symlink(&graph_path, directory.path().join("linked.json")).unwrap();
    assert!(store.load("linked.json").is_err());
    fs::write(directory.path().join("sidecar.json"), &graph).unwrap();
    let layout_path = outside.path().join("layout.json");
    fs::write(&layout_path, serde_json::to_vec(&layout()).unwrap()).unwrap();
    symlink(
        &layout_path,
        directory.path().join("sidecar.json.layout.json"),
    )
    .unwrap();
    assert!(store.load("sidecar.json").is_err());
    assert_eq!(fs::read(&graph_path).unwrap(), graph);
}

#[test]
fn active_instances_never_claim_or_clear_each_others_recovery() {
    let directory = TempDir::new().unwrap();
    let first = Store::new(directory.path()).unwrap();
    first.save_recovery(&document(1.0)).unwrap();
    let first_path = native_files(directory.path()).pop().unwrap();
    let second = Store::new(directory.path()).unwrap();
    assert!(second.recoveries().is_empty());
    second.save_recovery(&document(2.0)).unwrap();
    assert_eq!(native_files(directory.path()).len(), 2);
    first.clear_recovery().unwrap();
    assert!(!first_path.exists());
    let second_path = native_files(directory.path()).pop().unwrap();
    let bytes = fs::read(&second_path).unwrap();
    first.clear_recovery().unwrap(); // Idempotent and does not delete the other instance.
    assert_eq!(fs::read(&second_path).unwrap(), bytes);
    drop(first);
    assert_eq!(fs::read(&second_path).unwrap(), bytes);
    second.clear_recovery().unwrap();
    assert!(native_files(directory.path()).is_empty());
}

#[test]
fn orphan_recovery_is_claimed_once_and_restore_keeps_original_project_unchanged() {
    let directory = TempDir::new().unwrap();
    let original = document(1.0);
    let recovered = document(9.0);
    {
        let owner = Store::new(directory.path()).unwrap();
        owner.save_new("voice.openpatch", &original).unwrap();
        owner.save_recovery(&recovered).unwrap();
    } // Simulated process lifetime ends: file remains, ownership lock releases.
    let original_bytes = fs::read(directory.path().join("voice.openpatch")).unwrap();
    let claimant = Store::new(directory.path()).unwrap();
    let names = claimant.recoveries();
    assert_eq!(names.len(), 1);
    let other = Store::new(directory.path()).unwrap();
    assert!(other.recoveries().is_empty()); // First claimant holds the orphan lock.
    let loaded = claimant.load_recovery(&names[0]).unwrap();
    assert!(!loaded.imported);
    assert!(loaded.token.is_none());
    assert!(recovered.equivalent(&loaded.document));
    assert_eq!(
        fs::read(directory.path().join("voice.openpatch")).unwrap(),
        original_bytes
    );
    assert!(other.load_recovery(&names[0]).is_err());
    assert!(other.discard_recovery(&names[0]).is_err());
    claimant
        .save_new("recovered.openpatch", &loaded.document)
        .unwrap();
    assert_eq!(
        fs::read(directory.path().join("voice.openpatch")).unwrap(),
        original_bytes
    );
    claimant.discard_recovery(&names[0]).unwrap();
    assert!(claimant.load_recovery(&names[0]).is_err());
    assert!(recovered.equivalent(&claimant.load("recovered.openpatch").unwrap().document));
}

#[test]
fn voltage_utility_parameters_cables_and_bypass_survive_native_and_recovery_roundtrips() {
    let (directory, store) = store();
    let mut patch: Patch =
        serde_json::from_str(include_str!("../../../examples/voltage-utilities.json")).unwrap();
    let mutes = patch.nodes.iter_mut().find(|n| n.id == "mutes").unwrap();
    mutes.bypassed = true;
    mutes.params.insert("mute10".into(), 1.0);
    let original = Document::new(
        patch,
        json!({"version":1,"zoom":1.0,"pan":[24,12],"positions":{}}),
    )
    .unwrap();
    store.save_new("utilities.openpatch", &original).unwrap();
    let loaded = store.load("utilities.openpatch").unwrap().document;
    assert!(loaded.equivalent(&original));
    let encoded: Value = serde_json::from_slice(&read(&directory, "utilities.openpatch")).unwrap();
    let nodes = encoded["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| &entry["node"])
        .collect::<Vec<_>>();
    assert_eq!(
        nodes.iter().find(|n| n["id"] == "mutes").unwrap()["bypassed"],
        true
    );
    assert!(nodes
        .iter()
        .filter(|n| n["id"] != "mutes")
        .all(|n| n.get("bypassed").is_none()));
    store.save_recovery(&original).unwrap();
    drop(store);
    let store = Store::new(directory.path()).unwrap();
    let names = store.recoveries();
    assert_eq!(names.len(), 1);
    let recovered = store.load_recovery(&names[0]).unwrap().document;
    assert!(recovered.equivalent(&original));

    let legacy = br#"{"version":2,"nodes":[{"id":"dc","kind":"constant"}]}"#;
    fs::write(directory.path().join("old.json"), legacy).unwrap();
    let imported = store.load("old.json").unwrap();
    assert!(!imported.document.patch().nodes[0].bypassed);
    assert_eq!(read(&directory, "old.json"), legacy);
}

#[test]
fn gate_shaping_settings_and_bypass_roundtrip_without_saved_manual_button_state() {
    let (_directory, store) = store();
    let mut patch: Patch =
        serde_json::from_str(include_str!("../../../examples/gate-shaping-voice.json")).unwrap();
    let rescale = patch
        .nodes
        .iter_mut()
        .find(|n| n.kind == "rescale")
        .unwrap();
    rescale.bypassed = true;
    rescale.params.extend([
        ("multiplier".into(), 100.0),
        ("reflect_min".into(), 1.0),
        ("reflect_max".into(), 1.0),
        ("min".into(), -2.0),
        ("max".into(), 3.0),
    ]);
    patch
        .nodes
        .iter_mut()
        .find(|n| n.kind == "process")
        .unwrap()
        .params
        .insert("slew".into(), -3.0);
    let document = Document::new(
        patch,
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{}}),
    )
    .unwrap();
    store.save_new("gates.openpatch", &document).unwrap();
    let loaded = store.load("gates.openpatch").unwrap().document;
    assert!(document.equivalent(&loaded));
    let value: Value = serde_json::from_slice(&document.encode().unwrap()).unwrap();
    for entry in value["graph"]["nodes"].as_array().unwrap() {
        let node = &entry["node"];
        assert!(node.get("pressed").is_none());
        assert!(node.get("manual").is_none());
        if node["kind"] == "logic" {
            assert!(node
                .get("params")
                .is_none_or(|p| p.as_object().unwrap().is_empty()));
        }
    }
}

#[test]
fn layered_stereo_morph_native_roundtrip_preserves_all_mixing_settings_and_named_routes() {
    let (_directory, store) = store();
    let mut patch: Patch =
        serde_json::from_str(include_str!("../../../examples/layered-stereo-morph.json")).unwrap();
    let mix = patch.nodes.iter_mut().find(|n| n.id == "mix").unwrap();
    mix.params.insert("mix_exp".into(), 1.0);
    mix.params.insert("level4".into(), 0.0);
    mix.bypassed = true;
    let fade = patch.nodes.iter_mut().find(|n| n.id == "morph").unwrap();
    fade.params.insert("law".into(), 0.0);
    fade.params.insert("cv_amount".into(), -0.75);
    let document = Document::new(
        patch,
        json!({"version":1,"zoom":0.85,"pan":[24,12],"positions":{"mix":[0,0],"morph":[390,0]}}),
    )
    .unwrap();
    store.save_new("morph.openpatch", &document).unwrap();
    let loaded = store.load("morph.openpatch").unwrap().document;
    assert!(loaded.equivalent(&document));
    assert!(loaded
        .patch()
        .cables
        .iter()
        .any(|c| c.from == "mix" && c.output == "out4" && c.to == "morph" && c.input == "b"));
    assert!(loaded
        .patch()
        .cables
        .iter()
        .any(|c| c.from == "morph" && c.output == "complement" && c.input == "right"));
}

#[test]
fn duplicated_group_native_roundtrip_preserves_fresh_ids_bypass_named_cables_and_geometry() {
    let (directory, store) = store();
    let mut session = openpatch::control::Session::new(directory.path()).unwrap();
    let patch:Patch=serde_json::from_value(json!({"version":2,"nodes":[{"id":"voltage","kind":"poly_constant","params":{"channels":2,"v1":1,"v2":2}},{"id":"split","kind":"poly_split","bypassed":true},{"id":"out","kind":"audio_out"}],"cables":[{"from":"voltage","to":"split","input":"signal"},{"from":"split","output":"out2","to":"out","input":"signal"}],"output":"out"})).unwrap();
    session.replace_patch(patch, None).unwrap();
    session.execute("duplicate_modules",json!({"modules":[{"id":"voltage","new_id":"voltage-copy"},{"id":"split","new_id":"split-copy"}],"cable_mode":"inputs"})).unwrap();
    let layout = json!({"version":1,"zoom":0.8_f32,"pan":[-100,12],"positions":{"voltage":[0,0],"split":[360,0],"voltage-copy":[660,0],"split-copy":[1020,0],"out":[0,420]}});
    let document = Document::new(session.patch().clone(), layout).unwrap();
    store.save_new("copies.openpatch", &document).unwrap();
    let loaded = store.load("copies.openpatch").unwrap().document;
    assert!(loaded.equivalent(&document));
    assert_eq!(loaded.patch().output.as_deref(), Some("out"));
    assert!(
        loaded
            .patch()
            .nodes
            .iter()
            .find(|n| n.id == "split-copy")
            .unwrap()
            .bypassed
    );
    assert!(loaded
        .patch()
        .cables
        .iter()
        .any(|c| c.from == "voltage-copy" && c.to == "split-copy"));
    assert!(!loaded
        .patch()
        .cables
        .iter()
        .any(|c| c.from == "split-copy" && c.to == "out"));
    assert_eq!(
        loaded.layout()["positions"]["split-copy"],
        json!([1020.0, 0.0])
    );
}

#[test]
fn idless_project_migration_is_clean_and_preserves_original_disk_tokens() {
    use crate::document_state::{apply_document, DocumentState};
    use crate::rack::Rack;
    use openpatch::control::Session;

    for version in [1, 2] {
        let (directory, store) = store();
        let mut original = openpatch_core::demo_patch();
        original.version = version;
        assert!(original.cables.iter().all(|c| c.id.is_none()));
        let mut rack = Rack::default();
        rack.reconcile(&original, &openpatch_core::catalog());
        let layout = rack.layout();
        // Deliberately bypass Document::new to represent an actual older file.
        let old_native = serde_json::to_vec_pretty(&json!({
            "format":"openpatch-project", "version":1,
            "patch":original, "layout":layout
        }))
        .unwrap();
        let old_json = serde_json::to_vec_pretty(&original).unwrap();
        fs::write(directory.path().join("old.openpatch"), &old_native).unwrap();
        fs::write(directory.path().join("old.json"), &old_json).unwrap();
        let sidecar = serde_json::to_vec_pretty(&layout).unwrap();
        fs::write(directory.path().join("old.json.layout.json"), &sidecar).unwrap();

        for name in ["old.openpatch", "old.json"] {
            let loaded = store.load(name).unwrap();
            assert_eq!(loaded.document.layout(), &layout);
            assert_eq!(loaded.imported, name.ends_with(".json"));
            assert_eq!(loaded.token.is_some(), name.ends_with(".openpatch"));
            assert!(loaded
                .document
                .patch()
                .cables
                .iter()
                .all(|c| c.id.is_some()));
            let shared = Session::new(directory.path()).unwrap().into_shared();
            let applied = apply_document(&shared, 0, &loaded.document).unwrap();
            assert_eq!(
                applied.revision, 1,
                "migration is part of the load, not another edit"
            );
            assert_eq!(
                serde_json::to_value(&applied.patch).unwrap(),
                serde_json::to_value(loaded.document.patch()).unwrap()
            );
            assert_eq!(applied.rack.layout(), layout);
            let mut state =
                DocumentState::new(loaded.document.project(), applied.revision, &applied.rack);
            state.mark_saved(&loaded.document);
            assert!(
                !state.dirty(),
                "normalization must not dirty a loaded project"
            );
            let mut live = shared.lock().unwrap();
            live.execute(
                "set_parameter",
                json!({"id":"tone","parameter":"frequency","value":111}),
            )
            .unwrap();
            state.observe(live.patch(), live.revision(), &applied.rack);
            assert!(state.dirty());
            live.execute("undo", json!({})).unwrap();
            state.observe(live.patch(), live.revision(), &applied.rack);
            assert!(
                !state.dirty(),
                "undo returns to the canonical saved baseline"
            );
        }
        assert_eq!(read(&directory, "old.openpatch"), old_native);
        assert_eq!(read(&directory, "old.json"), old_json);
        assert_eq!(read(&directory, "old.json.layout.json"), sidecar);

        let loaded = store.load("old.openpatch").unwrap();
        let token = store
            .save_existing(
                "old.openpatch",
                &loaded.document,
                loaded.token.as_ref().unwrap(),
            )
            .unwrap();
        assert_ne!(
            read(&directory, "old.openpatch"),
            old_native,
            "the first save persists assigned IDs"
        );
        assert!(loaded
            .document
            .equivalent(&store.load("old.openpatch").unwrap().document));
        // A subsequent external edit must still invalidate the newly published token.
        fs::write(directory.path().join("old.openpatch"), &old_native).unwrap();
        assert!(store
            .save_existing("old.openpatch", &loaded.document, &token)
            .unwrap_err()
            .contains("changed on disk"));
        assert_eq!(read(&directory, "old.openpatch"), old_native);
    }
}

#[test]
fn stacked_cable_ids_colors_order_and_multiplicity_survive_native_save_and_recovery() {
    let (directory, store) = store();
    let patch: Patch = serde_json::from_value(json!({"version":2,
        "nodes":[{"id":"dc","kind":"poly_constant","params":{"channels":2,"v1":1,"v2":2}}, {"id":"sum","kind":"poly_sum"}],
        "cables":[{"id":"second","from":"dc","to":"sum","input":"signal","color":[17,201,94]},{"id":"first","from":"dc","to":"sum","input":"signal"}],"output":"sum"})).unwrap();
    let original = Document::new(patch, layout()).unwrap();
    store.save_new("stack.openpatch", &original).unwrap();
    assert!(original.equivalent(&store.load("stack.openpatch").unwrap().document));
    let mut engine = original.patch().compile().unwrap();
    let mut signal = [0.0; 8];
    engine.process_block(&mut signal);
    assert_eq!(signal[0], 0.0, "cables retain the one-sample delay");
    assert!(signal[1..].iter().all(|&v| v == 6.0));
    store.save_recovery(&original).unwrap();
    drop(store);
    let claimant = Store::new(directory.path()).unwrap();
    let names = claimant.recoveries();
    assert_eq!(names.len(), 1);
    let recovered = claimant.load_recovery(&names[0]).unwrap().document;
    assert!(original.equivalent(&recovered));
    assert_eq!(
        recovered
            .patch()
            .cables
            .iter()
            .map(|c| c.id.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["second", "first"]
    );
}

#[test]
fn independent_plug_order_survives_native_template_export_recovery_and_dirty_tracking() {
    use openpatch_core::{cable_endpoint_order, promote_cable_endpoints, CableEndpoint};
    let (directory, store) = store();
    let patch: Patch = serde_json::from_value(json!({"version":2,
        "nodes":[{"id":"dc","kind":"constant","params":{"value":1}},
                 {"id":"sum","kind":"poly_sum"}],
        "cables":[
            {"id":"a","from":"dc","to":"sum","input":"signal","input_order":20,"output_order":1},
            {"id":"b","from":"dc","to":"sum","input":"signal","input_order":10,"output_order":2}
        ],"output":"sum"}))
    .unwrap();
    let mut rack = crate::rack::Rack::default();
    rack.reconcile(&patch, &openpatch_core::catalog());
    let original = Document::new(patch.clone(), rack.layout()).unwrap();
    let mut state = crate::document_state::DocumentState::new(original.project(), 1, &rack);
    assert!(!state.dirty());
    assert_eq!(
        cable_endpoint_order(&patch.cables, CableEndpoint::Input, "sum", "signal"),
        [1, 0]
    );
    assert_eq!(
        cable_endpoint_order(&patch.cables, CableEndpoint::Output, "dc", "out"),
        [0, 1]
    );
    store.save_new("ordered.openpatch", &original).unwrap();
    store.save_template(&original, None).unwrap();
    fs::write(
        directory.path().join("exported.json"),
        serde_json::to_vec(&patch).unwrap(),
    )
    .unwrap();
    for loaded in [
        store.load("ordered.openpatch").unwrap().document,
        store.load_template().unwrap().unwrap(),
        store.load("exported.json").unwrap().document,
    ] {
        assert_eq!(
            serde_json::to_value(loaded.patch()).unwrap(),
            serde_json::to_value(&patch).unwrap()
        );
        assert_eq!(
            cable_endpoint_order(
                &loaded.patch().cables,
                CableEndpoint::Input,
                "sum",
                "signal"
            ),
            [1, 0]
        );
        assert_eq!(
            cable_endpoint_order(&loaded.patch().cables, CableEndpoint::Output, "dc", "out"),
            [0, 1]
        );
    }
    let mut promoted = patch.clone();
    assert!(promote_cable_endpoints(&mut promoted.cables, CableEndpoint::Input, &[1]).unwrap());
    assert!(state.observe(&promoted, 2, &rack));
    assert!(
        state.dirty(),
        "a same-route plug promotion is a saved document edit"
    );
    assert_eq!(
        promoted
            .cables
            .iter()
            .map(|c| c.id.as_deref())
            .collect::<Vec<_>>(),
        [Some("a"), Some("b")]
    );
    assert_eq!(
        promoted
            .cables
            .iter()
            .map(|c| c.output_order)
            .collect::<Vec<_>>(),
        [Some(1), Some(2)]
    );
    let edited = Document::new(promoted.clone(), rack.layout()).unwrap();
    let encoded = edited.encode().unwrap();
    assert!(!promote_cable_endpoints(&mut promoted.cables, CableEndpoint::Input, &[1]).unwrap());
    assert_eq!(
        Document::new(promoted.clone(), rack.layout())
            .unwrap()
            .encode()
            .unwrap(),
        encoded
    );
    state.mark_saved(&edited);
    assert!(!state.dirty());
    state.observe(&patch, 3, &rack);
    assert!(
        state.dirty(),
        "undoing a saved promotion changes document content"
    );
    state.observe(&promoted, 4, &rack);
    assert!(!state.dirty(), "redo returns to saved ordering");
    store.save_recovery(&original).unwrap();
    drop(store);
    let claimant = Store::new(directory.path()).unwrap();
    let recoveries = claimant.recoveries();
    assert_eq!(recoveries.len(), 1);
    assert!(original.equivalent(&claimant.load_recovery(&recoveries[0]).unwrap().document));
}
