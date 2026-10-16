use super::*;
use openpatch_core::wavetable_file::{decode_wavetable, encode_wavetable_wav};
use openpatch_core::{ModuleContent, WaveAsset};
use std::sync::atomic::{AtomicU64, Ordering};

struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "openpatch-wave-tools-{}-{}",
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

fn session(kind: &str) -> (Directory, Session) {
    let directory = Directory::new();
    let mut session = Session::new(&directory.0).unwrap();
    let patch: Patch = serde_json::from_value(json!({"version":2,
        "nodes":[{"id":"wave","kind":kind}],"output":"wave"}))
    .unwrap();
    session.replace_patch(patch, None).unwrap();
    (directory, session)
}

fn raw(samples: &[f32]) -> Vec<u8> {
    samples
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect()
}

fn float_stereo_wav(samples: &[f32], rate: u32) -> Vec<u8> {
    assert_eq!(samples.len() % 2, 0);
    let length = samples.len() as u32 * 4;
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + length).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16_u32.to_le_bytes());
    bytes.extend_from_slice(&3_u16.to_le_bytes());
    bytes.extend_from_slice(&2_u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 8).to_le_bytes());
    bytes.extend_from_slice(&8_u16.to_le_bytes());
    bytes.extend_from_slice(&32_u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&raw(samples));
    bytes
}

#[test]
fn import_preserves_interleaved_float_bits_and_commits_once_with_dedup_and_history() {
    let (directory, mut session) = session("wavetable_lfo");
    let samples = [vec![0.0, -0.0, 2.5, -3.0], vec![0.125; 60]].concat();
    fs::write(
        directory.0.join("stereo.wav"),
        float_stereo_wav(&samples, 32),
    )
    .unwrap();
    let before = session.revision();
    let imported = session
        .execute(
            "import_wavetable",
            json!({"id":"wave",
        "filename":"stereo.wav","expected_revision":before}),
        )
        .unwrap();
    assert_eq!(session.revision(), before + 1);
    assert_eq!(imported["source_channels"], 2);
    assert_eq!(imported["source_sample_rate"], 32);
    assert_eq!(imported["content"]["wave_points"], 32);
    assert!(imported.get("patch").is_none());
    let id = imported["asset"]["sha256"].as_str().unwrap();
    let asset = session.patch.assets[id].clone();
    assert_eq!(
        asset.samples().map(f32::to_bits).collect::<Vec<_>>(),
        samples
            .iter()
            .map(|sample| sample.to_bits())
            .collect::<Vec<_>>()
    );
    let unchanged = session.snapshot();
    let reimported = session
        .execute(
            "import_wavetable",
            json!({"id":"wave",
        "filename":"stereo.wav","expected_revision":session.revision()}),
        )
        .unwrap();
    assert_eq!(reimported["changed"], false);
    assert_eq!(session.snapshot(), unchanged);
    assert_eq!(session.patch.assets.len(), 1);
    assert!(Arc::ptr_eq(&asset, &session.patch.assets[id]));
    session.execute("undo", json!({})).unwrap();
    assert!(session.patch.nodes[0].content.is_none());
    session.execute("redo", json!({})).unwrap();
    assert_eq!(
        session.patch.nodes[0].content.as_ref().unwrap(),
        &ModuleContent::Wavetable {
            asset_id: id.into(),
            wave_points: 32,
            name: "stereo.wav".into()
        }
    );
    assert!(Arc::ptr_eq(&asset, &session.patch.assets[id]));
}

#[test]
fn invalid_or_stale_imports_leave_the_patch_and_history_unchanged() {
    let (directory, mut session) = session("wavetable_vco");
    fs::write(directory.0.join("good.f32"), raw(&[0.25; 64])).unwrap();
    fs::write(directory.0.join("partial.f32"), [0, 1, 2]).unwrap();
    fs::write(directory.0.join("nonfinite.f32"), raw(&[f32::NAN; 32])).unwrap();
    fs::write(directory.0.join("empty.f32"), []).unwrap();
    let huge = fs::File::create(directory.0.join("huge.f32")).unwrap();
    huge.set_len(16 * 1024 * 1024 + 1).unwrap();
    fs::write(
        directory.0.join("oversize.i8"),
        vec![0; openpatch_core::wavetable_file::MAX_WAVETABLE_SAMPLES + 1],
    )
    .unwrap();
    fs::create_dir(directory.0.join("directory.f32")).unwrap();
    let before = session.snapshot();
    for args in [
        json!({"id":"wave","filename":"good.f32","wave_points":33}),
        json!({"id":"wave","filename":"good.f32","wave_points":128}),
        json!({"id":"wave","filename":"good.f32","wave_points":32,"expected_revision":0}),
        json!({"id":"wave","filename":"../good.f32","wave_points":32}),
        json!({"id":"wave","filename":"partial.f32","wave_points":32}),
        json!({"id":"wave","filename":"nonfinite.f32","wave_points":32}),
        json!({"id":"wave","filename":"empty.f32","wave_points":32}),
        json!({"id":"wave","filename":"huge.f32","wave_points":32}),
        json!({"id":"wave","filename":"oversize.i8","wave_points":32}),
        json!({"id":"wave","filename":"directory.f32","wave_points":32}),
    ] {
        assert!(session.execute("import_wavetable", args).is_err());
        assert_eq!(session.snapshot(), before);
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(directory.0.join("good.f32"), directory.0.join("link.f32"))
            .unwrap();
        assert!(session
            .execute(
                "import_wavetable",
                json!({"id":"wave","filename":"link.f32","wave_points":32})
            )
            .is_err());
        assert_eq!(session.snapshot(), before);
    }
}

#[test]
fn exports_write_original_samples_and_never_replace_an_existing_file() {
    let (directory, mut session) = session("wavetable_vco");
    let samples = [vec![-0.0, 0.0, 2.5, -3.0], vec![0.125; 60]].concat();
    fs::write(directory.0.join("source.f32"), raw(&samples)).unwrap();
    session
        .execute(
            "import_wavetable",
            json!({"id":"wave","filename":"source.f32","wave_points":32}),
        )
        .unwrap();
    let before = session.snapshot();
    let exported = session
        .execute(
            "export_wavetable",
            json!({"id":"wave","filename":"exact.f32"}),
        )
        .unwrap();
    assert_eq!(
        fs::read(directory.0.join("exact.f32")).unwrap(),
        raw(&samples)
    );
    assert!(exported.get("patch").is_none());
    session
        .execute(
            "export_wavetable",
            json!({"id":"wave","filename":"converted.wav"}),
        )
        .unwrap();
    let wav = fs::read(directory.0.join("converted.wav")).unwrap();
    assert_eq!(wav, encode_wavetable_wav(&samples, 32).unwrap());
    assert_eq!(
        decode_wavetable(&wav, "wav").unwrap().suggested_wave_length,
        Some(32)
    );
    assert!(session
        .execute(
            "export_wavetable",
            json!({"id":"wave","filename":"exact.f32"})
        )
        .is_err());
    assert_eq!(
        fs::read(directory.0.join("exact.f32")).unwrap(),
        raw(&samples)
    );
    assert!(session
        .execute(
            "export_wavetable",
            json!({"id":"wave","filename":"../escape.f32"})
        )
        .is_err());
    assert_eq!(session.snapshot(), before);
}

#[test]
fn existing_asset_selection_supports_atomic_batches_and_inspection_is_bounded() {
    let (directory, mut session) = session("wavetable_lfo");
    fs::write(directory.0.join("source.f32"), raw(&[0.25; 1024])).unwrap();
    let imported = session
        .execute(
            "import_wavetable",
            json!({"id":"wave","filename":"source.f32","wave_points":32}),
        )
        .unwrap();
    let id = imported["asset"]["sha256"].as_str().unwrap();
    let args = json!({"id":"wave","asset_id":id,"wave_points":64,"name":"Second interpretation"});
    let before = session.snapshot();
    let preview = session
        .execute(
            "preview_edits",
            json!({"edits":[{"tool":"set_wavetable","arguments":args}]}),
        )
        .unwrap();
    assert_eq!(preview["patch"]["nodes"][0]["content"]["wave_points"], 64);
    assert_eq!(session.snapshot(), before);
    session
        .execute(
            "apply_edits",
            json!({"edits":[{"tool":"set_wavetable","arguments":args}]}),
        )
        .unwrap();
    let after = session.snapshot();
    session.execute("set_wavetable", args).unwrap();
    assert_eq!(session.snapshot(), after);
    assert!(session.execute("apply_edits",json!({"edits":[
        {"tool":"set_parameter","arguments":{"id":"wave","parameter":"position","value":0.75}},
        {"tool":"set_wavetable","arguments":{"id":"wave","asset_id":id,"wave_points":2048,"name":"Invalid"}}
    ]})).is_err());
    assert_eq!(session.snapshot(), after);
    let compact = session
        .execute("inspect_wavetable", json!({"id":"wave"}))
        .unwrap();
    assert!(compact.get("samples").is_none());
    assert!(compact["asset"].get("data").is_none());
    let chunk = session
        .execute(
            "inspect_wavetable",
            json!({"id":"wave","offset":1000,"limit":256}),
        )
        .unwrap();
    assert_eq!(chunk["samples"].as_array().unwrap().len(), 24);
    assert!(serde_json::to_vec(&chunk).unwrap().len() <= 8192);
    assert!(session
        .execute("inspect_wavetable", json!({"id":"wave","limit":257}))
        .is_err());
    assert!(session
        .execute(
            "inspect_wavetable",
            json!({"id":"wave","offset":1025,"limit":1})
        )
        .is_err());
    assert_eq!(session.snapshot(), after);
}

#[test]
fn default_wave_export_materializes_the_production_bank_without_editing_the_patch() {
    let (directory, mut session) = session("wavetable_lfo");
    let before = session.snapshot();
    session
        .execute(
            "export_wavetable",
            json!({"id":"wave","filename":"basic.f32"}),
        )
        .unwrap();
    let decoded =
        decode_wavetable(&fs::read(directory.0.join("basic.f32")).unwrap(), "f32").unwrap();
    let actual = WaveAsset::new(&decoded.samples).unwrap();
    assert_eq!(actual, *openpatch_core::default_wavetable_asset());
    assert!(session.patch.assets.is_empty());
    assert_eq!(session.snapshot(), before);
}
