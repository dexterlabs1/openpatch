//! Imported voice exercises shared MCP edits/history/document persistence and
//! actual Core processing. This never opens an audio device or native window.
use super::*;
use openpatch_core::PortFrame;

struct Workspace(PathBuf);
impl Workspace {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "openpatch-rack264-voice-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn source_voice() -> Vec<u8> {
    serde_json::to_vec_pretty(&json!({
        "version":"2.6.6",
        "modules":[
            {"id":1,"plugin":"Fundamental","model":"VCO","version":"2.6.4","pos":[0,0],
             "params":[{"id":2,"value":0}],"future":{"retain":"oscillator"}},
            {"id":2,"plugin":"Fundamental","model":"VCF","version":"2.6.4","pos":[10,0],
             "params":[{"id":0,"value":0.7},{"id":2,"value":0.2},{"id":4,"value":0.1},
                       {"id":5,"value":0},{"id":6,"value":0}]},
            {"id":3,"plugin":"Fundamental","model":"ADSR","version":"2.6.4","pos":[18,0],
             "params":[{"id":0,"value":0},{"id":1,"value":0},{"id":2,"value":0.8},{"id":3,"value":0}]},
            {"id":4,"plugin":"Fundamental","model":"VCA-1","version":"2.6.4","pos":[26,0],
             "params":[{"id":0,"value":0.8}]}
        ],
        "cables":[
            {"id":11,"outputModuleId":1,"outputId":2,"inputModuleId":2,"inputId":3},
            {"id":12,"outputModuleId":2,"outputId":0,"inputModuleId":4,"inputId":1},
            {"id":13,"outputModuleId":1,"outputId":3,"inputModuleId":3,"inputId":4},
            {"id":14,"outputModuleId":3,"outputId":0,"inputModuleId":4,"inputId":0}
        ],
        "future":{"retain":"entire original patch"}
    }))
    .unwrap()
}

fn rendered(session: &Session, width: u8) -> Vec<PortFrame> {
    assert_eq!(
        session.execution_disposition(),
        ExecutionDisposition::RunnableExact
    );
    let mut engine = session.patch().compile().unwrap();
    let mut frames = vec![PortFrame::default(); 8192];
    engine.process_poly_block(&mut frames);
    assert!(frames
        .iter()
        .all(|f| f.values.iter().all(|v| v.is_finite())));
    assert!(frames[8..].iter().all(|f| f.channels == width));
    for lane in 0..usize::from(width) {
        let rms = (frames.iter().map(|f| f.values[lane].powi(2)).sum::<f32>()
            / frames.len() as f32)
            .sqrt();
        assert!(rms > 0.001, "silent imported lane {lane}: rms={rms}");
    }
    frames
}

#[test]
fn rack264_voice_import_edits_undo_and_saved_reopen_keep_source_and_real_signal() {
    let workspace = Workspace::new();
    let original = source_voice();
    fs::write(workspace.0.join("voice.vcv"), &original).unwrap();
    let mut session = Session::new(&workspace.0).unwrap();
    let imported = session
        .execute("import_vcv", json!({"filename":"voice.vcv"}))
        .unwrap();
    assert_eq!(imported["report"]["mapped_count"], 4);
    assert_eq!(imported["report"]["unresolved_count"], 0);
    let sources = session.project_document().sources.clone();
    let source_blobs = session.project_document().source_blobs.clone();
    session
        .execute("set_output", json!({"id":"vcv_m_4","port":"out"}))
        .unwrap();
    let mono = rendered(&session, 1);
    session
        .execute(
            "set_parameter",
            json!({"id":"vcv_m_2","parameter":"frequency","value":0.45}),
        )
        .unwrap();
    assert_ne!(
        rendered(&session, 1),
        mono,
        "real filter edit must affect the voice"
    );
    let edited = session.project_document().encode().unwrap();
    session.execute("undo", json!({})).unwrap();
    assert_eq!(rendered(&session, 1), mono);
    session.execute("redo", json!({})).unwrap();
    assert_eq!(session.project_document().encode().unwrap(), edited);

    let mut params = serde_json::Map::from_iter([("channels".into(), json!(16))]);
    for lane in 1..=16 {
        params.insert(format!("v{lane}"), json!((lane as f32 - 8.) / 12.));
    }
    session
        .execute(
            "add_module",
            json!({"id":"pitch","kind":"poly_constant","params":params}),
        )
        .unwrap();
    session
        .execute(
            "connect",
            json!({"from":"pitch","to":"vcv_m_1","input":"pitch"}),
        )
        .unwrap();
    let poly = rendered(&session, 16);
    assert!(poly.iter().any(|frame| frame.values[0] != frame.values[15]));
    session
        .execute("save_document", json!({"filename":"voice.json"}))
        .unwrap();
    let saved = session.project_document().encode().unwrap();
    let mut reopened = Session::new(&workspace.0).unwrap();
    reopened
        .execute("load_document", json!({"filename":"voice.json"}))
        .unwrap();
    assert_eq!(reopened.project_document().encode().unwrap(), saved);
    assert_eq!(rendered(&reopened, 16), poly);
    assert_eq!(
        serde_json::to_value(&reopened.project_document().sources).unwrap(),
        serde_json::to_value(&sources).unwrap()
    );
    assert_eq!(
        reopened.project_document().source_blobs.len(),
        source_blobs.len()
    );
    for (id, original) in &source_blobs {
        assert_eq!(
            reopened.project_document().source_blobs[id].bytes(),
            original.bytes()
        );
    }
    assert_eq!(fs::read(workspace.0.join("voice.vcv")).unwrap(), original);
}
