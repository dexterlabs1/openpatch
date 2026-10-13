//! Filesystem acceptance tests through the native document/store boundary.
//! Every file is owned by a disposable temporary workspace.
use crate::project::{Document, Store};
use openpatch_core::{Cable, Node, Patch};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn layout() -> Value {
    json!({"version":1,"zoom":0.8,"pan":[-92.0,37.0],"positions":{"dc":[128.0,380.0]}})
}
fn patch(version: u32, value: f32) -> Patch {
    Patch {
        version,
        nodes: vec![Node {
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
    assert_eq!(envelope["version"], 1);
    assert_eq!(
        envelope["patch"],
        serde_json::to_value(&original.patch).unwrap()
    );
    assert_eq!(envelope["layout"], original.layout);
    assert_eq!(envelope.as_object().unwrap().len(), 4);
    let loaded = store.load("voice.openpatch").unwrap();
    assert!(!loaded.imported);
    assert!(loaded.token.is_some());
    assert!(original.equivalent(&loaded.document));
    assert!(!directory
        .path()
        .join("voice.openpatch.layout.json")
        .exists());

    let mut edited = document(-2.5);
    edited.layout["pan"] = json!([19.0, -81.0]);
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
                    bypassed: false,
                    id: "a".into(),
                    kind: "mult".into(),
                    params: Default::default(),
                },
                Node {
                    bypassed: false,
                    id: "b".into(),
                    kind: "mult".into(),
                    params: Default::default(),
                },
            ]);
            original_patch.cables.extend([
                Cable {
                    from: "dc".into(),
                    output: "out".into(),
                    to: "a".into(),
                    input: "signal".into(),
                },
                Cable {
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
        assert_eq!(loaded.document.patch.version, version);
        assert_eq!(
            serde_json::to_value(&loaded.document.patch).unwrap(),
            serde_json::to_value(&original_patch).unwrap()
        );
        assert_eq!(loaded.document.layout, layout());
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
    rack.restore_layout(&loaded.document.layout).unwrap();
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
    let token = store.save_new("voice.openpatch", &document(1.0)).unwrap();
    let before = read(&directory, "voice.openpatch");
    let mut invalid = document(2.0);
    invalid.layout["zoom"] = json!(900.0);
    assert!(store
        .save_existing("voice.openpatch", &invalid, &token)
        .is_err());
    assert_eq!(read(&directory, "voice.openpatch"), before);
    assert!(store.save_new("invalid.openpatch", &invalid).is_err());
    assert!(!directory.path().join("invalid.openpatch").exists());
    invalid = document(2.0);
    invalid.patch.output = Some("missing".into());
    assert!(store
        .save_existing("voice.openpatch", &invalid, &token)
        .is_err());
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
        ("version", json!(2)),
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
    let mut native = document(1.0).encode().unwrap();
    native.resize(2 * 1024 * 1024 + 1, b' '); // Still valid JSON: reject size, not syntax.
    fs::write(directory.path().join("large.openpatch"), native).unwrap();
    assert!(store.load("large.openpatch").is_err());
    let mut legacy = serde_json::to_vec(&patch(1, 1.0)).unwrap();
    legacy.resize(1024 * 1024 + 1, b' ');
    fs::write(directory.path().join("large.json"), legacy).unwrap();
    assert!(store.load("large.json").is_err());
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
    let nodes = encoded["patch"]["nodes"].as_array().unwrap();
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
    assert!(!imported.document.patch.nodes[0].bypassed);
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
    for node in value["patch"]["nodes"].as_array().unwrap() {
        assert!(node.get("pressed").is_none());
        assert!(node.get("manual").is_none());
        if node["kind"] == "logic" {
            assert!(node
                .get("params")
                .is_none_or(|p| p.as_object().unwrap().is_empty()));
        }
    }
}
