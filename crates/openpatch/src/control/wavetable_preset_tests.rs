use super::*;
use crate::presets;
use openpatch_core::{ModuleContent, WaveAsset, WaveAssets};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "openpatch-wave-presets-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn custom(kind: &str, samples: &[f32], points: u32) -> (Node, WaveAssets) {
    let asset = Arc::new(WaveAsset::new(samples).unwrap());
    let node: Node = serde_json::from_value(json!({
        "id":"wave", "kind":kind, "params":{"position":0.375},
        "content":{"type":"wavetable","asset_id":asset.id(),
            "wave_points":points,"name":"Portable wave"}
    }))
    .unwrap();
    (node, [(asset.id().to_owned(), asset)].into_iter().collect())
}

#[test]
fn portable_wavetable_presets_retain_exact_samples_and_only_the_referenced_asset() {
    let samples = [vec![0.0, -0.0, 0.5, -0.5], vec![0.25; 60]].concat();
    for kind in ["wavetable_vco", "wavetable_lfo"] {
        let (node, mut assets) = custom(kind, &samples, 32);
        let extra = Arc::new(WaveAsset::new(&[0.75, -0.25]).unwrap());
        assets.insert(extra.id().into(), extra.clone());
        assert!(presets::capture_state(&node, None)
            .unwrap_err()
            .contains("requires"));
        let preset = presets::capture_state_with_assets(&node, None, &assets).unwrap();
        assert_eq!(preset["assets"].as_object().unwrap().len(), 1);
        assert!(preset["assets"].get(extra.id()).is_none());
        let portable: Value =
            serde_json::from_slice(&serde_json::to_vec(&preset).unwrap()).unwrap();
        let restored = presets::assets_from_preset(&portable).unwrap();
        let id = portable["content"]["asset_id"].as_str().unwrap();
        assert_eq!(restored[id], assets[id]);
        assert_eq!(
            restored[id].samples().map(f32::to_bits).collect::<Vec<_>>(),
            samples
                .iter()
                .map(|value| value.to_bits())
                .collect::<Vec<_>>()
        );
        let mut target = node.clone();
        target.content = None;
        presets::apply(&mut target, &portable).unwrap();
        assert_eq!(target.content, node.content);
        assert_eq!(target.params["position"], 0.375);
        let summary = presets::summary(&portable).unwrap();
        assert_eq!(summary["portable_preset"], false);
        assert!(summary["assets"][id].get("data").is_none());
        assert!(presets::validate(&summary).is_err());
    }
}

#[test]
fn wavetable_preset_validation_rejects_missing_unused_corrupt_and_mismatched_assets() {
    let (node, assets) = custom("wavetable_vco", &[0.25; 64], 32);
    let valid = presets::capture_state_with_assets(&node, None, &assets).unwrap();
    let id = valid["content"]["asset_id"].as_str().unwrap();
    let mut invalid = Vec::new();
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("assets");
    invalid.push(missing);
    let mut unused = valid.clone();
    let extra = WaveAsset::new(&[0.75, -0.25]).unwrap();
    unused["assets"][extra.id()] = serde_json::to_value(&extra).unwrap();
    invalid.push(unused);
    let mut corrupt = valid.clone();
    corrupt["assets"][id]["data"] = json!("invalid!");
    invalid.push(corrupt);
    let mut mismatched = valid.clone();
    mismatched["assets"][id] = serde_json::to_value(&extra).unwrap();
    invalid.push(mismatched);
    let mut oversized_wave = valid.clone();
    oversized_wave["content"]["wave_points"] = json!(128);
    invalid.push(oversized_wave);
    let mut wrong_shape = valid.clone();
    wrong_shape["content"]["wave_points"] = json!(33);
    invalid.push(wrong_shape);
    let mut wrong_kind = valid.clone();
    wrong_kind["kind"] = json!("multi_vco");
    invalid.push(wrong_kind);
    let mut invalid_parameter = valid.clone();
    invalid_parameter["params"]["sync_mode"] = json!(0.5);
    invalid.push(invalid_parameter);
    let mut orphan = valid.clone();
    orphan.as_object_mut().unwrap().remove("content");
    invalid.push(orphan);
    for preset in invalid {
        assert!(presets::validate(&preset).is_err());
    }
}

#[test]
fn default_wave_presets_restore_default_content_and_legacy_presets_keep_their_shape() {
    let (mut custom_node, _) = custom("wavetable_lfo", &[0.25; 32], 32);
    let mut default_node = custom_node.clone();
    default_node.content = None;
    let default = presets::capture_state(&default_node, None).unwrap();
    assert!(default.get("assets").is_none());
    assert!(default.get("content").is_none());
    presets::apply(&mut custom_node, &default).unwrap();
    assert!(custom_node.content.is_none());
    let notes: Node = serde_json::from_value(json!({"id":"notes","kind":"notes"})).unwrap();
    assert_eq!(
        presets::capture_state(&notes, None).unwrap(),
        json!({
            "format":"openpatch-module-preset","version":1,"kind":"notes",
            "params":{},"bypassed":false,"content":{"type":"notes","text":""}
        })
    );
    let mut annotation = notes;
    annotation.content = Some(ModuleContent::Notes {
        text: "Retain me".into(),
    });
    presets::apply(
        &mut annotation,
        &json!({"format":"openpatch-module-preset",
        "version":1,"kind":"notes","params":{}}),
    )
    .unwrap();
    assert_eq!(annotation.effective_notes_text(), Some("Retain me"));
}

#[test]
fn maximum_wave_preset_roundtrips_bounded_file_and_hashes_the_exact_reviewed_bytes() {
    let directory = Directory::new();
    let (node, assets) = custom(
        "wavetable_lfo",
        &vec![0.25; openpatch_core::wavetable_file::MAX_WAVETABLE_SAMPLES],
        32,
    );
    let preset = presets::capture_state_with_assets(&node, None, &assets).unwrap();
    let path = directory.0.join("maximum.opreset");
    presets::save_new(&path, &preset).unwrap();
    let bytes = fs::read(&path).unwrap();
    assert!(bytes.len() > 2 * 1024 * 1024 && bytes.len() < presets::MAX_BYTES);
    let (decoded, digest) = presets::read_with_digest(&path).unwrap();
    assert_eq!(decoded, preset);
    assert_eq!(digest, format!("{:x}", Sha256::digest(&bytes)));
    fs::write(&path, [bytes, vec![b'\n']].concat()).unwrap();
    let (same, changed_digest) = presets::read_with_digest(&path).unwrap();
    assert_eq!(same, preset);
    assert_ne!(changed_digest, digest);
    fs::write(&path, vec![b' '; presets::MAX_BYTES + 1]).unwrap();
    assert!(presets::read(&path).unwrap_err().contains("4 MiB"));
}

#[test]
fn wavetable_preset_tools_omit_payloads_until_explicitly_requested() {
    let directory = Directory::new();
    let mut session = Session::new(&directory.0).unwrap();
    let (node, assets) = custom("wavetable_lfo", &[0.25; 64], 32);
    let mut patch = Patch::new_v2();
    patch.output = Some(node.id.clone());
    patch.nodes.push(node);
    patch.assets = assets;
    session.replace_patch(patch, None).unwrap();
    let compact = session
        .execute("capture_module_preset", json!({"id":"wave"}))
        .unwrap();
    assert!(compact.get("preset").is_none());
    assert!(compact["preset_summary"]["assets"]
        .as_object()
        .unwrap()
        .values()
        .all(|asset| asset.get("data").is_none()));
    let full = session
        .execute(
            "capture_module_preset",
            json!({"id":"wave","include_asset_data":true}),
        )
        .unwrap();
    presets::validate(&full["preset"]).unwrap();
    let saved = session
        .execute(
            "save_module_preset",
            json!({"filename":"wave.opreset","preset":full["preset"]}),
        )
        .unwrap();
    assert!(saved.get("preset").is_none());
    let read = session
        .execute("read_module_preset", json!({"filename":"wave.opreset"}))
        .unwrap();
    assert!(read.get("preset").is_none());
    assert_eq!(read["file_sha256"].as_str().unwrap().len(), 64);
    let exported = session
        .execute(
            "read_module_preset",
            json!({"filename":"wave.opreset","include_asset_data":true}),
        )
        .unwrap();
    assert_eq!(exported["preset"], full["preset"]);
}
