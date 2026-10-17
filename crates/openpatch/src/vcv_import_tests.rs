use super::*;
use serde_json::json;
use std::io::Write;

fn module(id: u64, model: &str) -> Value {
    json!({"id":id,"plugin":"Fundamental","model":model,"version":"2.6.4","pos":[id,1]})
}
fn cable(id: u64, from: u64, output: u32, to: u64, input: u32) -> Value {
    json!({"id":id,"outputModuleId":from,"outputId":output,"inputModuleId":to,"inputId":input,"color":"#EaB308","inputPlugOrder":2,"outputPlugOrder":1,"future":{"retained":true}})
}
fn run(patch: Value) -> Result<ImportResult, String> {
    import(&serde_json::to_vec(&patch).unwrap())
}
fn native(document: &ProjectDocument, index: usize) -> &Node {
    match &document.graph.nodes[index] {
        DocumentNode::Native { node, .. } => node,
        _ => panic!("expected native module"),
    }
}
fn tar_patch(patch: &Value, assets: &[(&str, &[u8])]) -> Vec<u8> {
    let mut archive = tar::Builder::new(Vec::new());
    let bytes = serde_json::to_vec(patch).unwrap();
    for (path, data) in
        std::iter::once(("patch.json", bytes.as_slice())).chain(assets.iter().copied())
    {
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive.append_data(&mut header, path, data).unwrap();
    }
    let raw = archive.into_inner().unwrap();
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
    encoder.write_all(&raw).unwrap();
    encoder.finish().unwrap()
}

#[test]
fn imports_source_controls_routing_provenance_and_layout_without_changing_original() {
    let mut vca = module(42, "VCA-1");
    vca["params"] = json!([{"id":0,"value":0.37},{"id":1,"value":0}]);
    vca["bypass"] = json!(true);
    vca["data"] = json!({"future":[1,2,3]});
    let patch = json!({"version":"2.6.6","zoom":0.5,"gridOffset":[-2,1],"future":"retain", "modules":[module(7,"Noise"),vca],"cables":[cable(99,7,6,42,1)]});
    let original = serde_json::to_vec_pretty(&patch).unwrap();
    let result = import(&original).unwrap();
    assert_eq!(
        (result.report.mapped_count, result.report.unresolved_count),
        (2, 0)
    );
    let document = result.document;
    let node = native(&document, 1);
    assert_eq!(node.kind, "stock_vca");
    assert!(node.bypassed);
    assert_eq!(node.params["level"], 0.37);
    assert_eq!(node.params["response"], 0.0);
    assert_eq!(document.graph.cables[0].route.output, "black");
    assert_eq!(document.graph.cables[0].route.input, "signal");
    assert_eq!(document.graph.cables[0].route.color, Some([234, 179, 8]));
    assert_eq!(document.graph.cables[0].route.input_order, Some(2));
    assert_eq!(document.layout.positions["vcv_m_42"], [630.0, 420.0]);
    assert_eq!(document.layout.pan, [15.0, -210.0]);
    let package = &document.sources[&result.report.package_id];
    assert_eq!(*package.patch, patch);
    assert_eq!(
        document.source_blobs[&package.assets["patch.json"]].bytes(),
        original
    );
    let source = document.graph.nodes[1].source().unwrap();
    assert_eq!(source.index, 1);
    assert_eq!(source.mapping.as_ref().unwrap().version, 1);
    assert_eq!(
        source.mapping.as_ref().unwrap().fidelity,
        MappingFidelity::SourceBehavior
    );
    let reopened = ProjectDocument::decode(&document.encode().unwrap()).unwrap();
    assert_eq!(
        reopened.source_module(source).unwrap()["data"]["future"],
        json!([1, 2, 3])
    );
    assert!(reopened
        .project(crate::project_document::PlaybackMode::Exact)
        .is_ok());
}

#[test]
fn unknown_versions_remain_recoverable_and_suspend_exact() {
    let mut unknown = module(1, "VCA-1");
    unknown["version"] = json!("2.6.5");
    let mut filter = module(2, "VCF");
    filter["version"] = json!("2.6.5");
    let mut oscillator = module(3, "VCO");
    oscillator["version"] = json!("2.6.3");
    let patch = json!({"modules":[unknown,filter,oscillator,module(4,"VCA-1")],"cables":[cable(4,1,123,4,0),cable(5,3,2,2,3)]});
    let result = run(patch.clone()).unwrap();
    assert_eq!(result.report.unresolved_count, 3);
    assert_eq!(result.report.omitted_route_count, 2);
    let document = result.document;
    assert!(document
        .project(crate::project_document::PlaybackMode::Exact)
        .is_err());
    assert_eq!(
        document
            .project(crate::project_document::PlaybackMode::Partial)
            .unwrap()
            .omitted_routes
            .len(),
        2
    );
    match &document.graph.nodes[0] {
        DocumentNode::Unresolved { ports, source, .. } => {
            assert_eq!(ports[0].index, 123);
            assert!(source.mapping.is_none());
        }
        _ => panic!("unrecognized version must stay unresolved"),
    }
    assert_eq!(document.graph.cables[0].route.output, "vcv_out_123");
    assert_eq!(document.graph.cables[0].route.input, "cv");
    assert_eq!(*document.sources[&result.report.package_id].patch, patch);
}

#[test]
fn rack264_voice_maps_exact_controls_removed_slots_and_preserves_source_archive() {
    let mut oscillator = module(1, "VCO");
    oscillator["params"] = json!([
        {"id":0,"value":0.37}, {"id":1,"value":0}, {"id":2,"value":-54},
        {"id":3,"value":-999}, {"id":4,"value":-0.75}, {"id":5,"value":0.99},
        {"id":6,"value":0.625}, {"id":7,"value":1}
    ]);
    oscillator["future_source_metadata"] = json!({"opaque":true});
    let mut filter = module(2, "VCF");
    filter["params"] = json!([
        {"id":0,"value":0.625}, {"id":1,"value":-999}, {"id":2,"value":0.875},
        {"id":3,"value":-0.5}, {"id":4,"value":-1}, {"id":5,"value":0},
        {"id":6,"value":0.75}
    ]);
    filter["bypass"] = json!(true);
    let patch =
        json!({"version":"2.6.6","modules":[oscillator,filter],"cables":[cable(7,1,2,2,3)]});
    let result = import(&tar_patch(&patch, &[("modules/2/opaque.bin", b"retain")])).unwrap();
    assert_eq!(
        (result.report.mapped_count, result.report.unresolved_count),
        (2, 0)
    );
    let oscillator = native(&result.document, 0);
    assert_eq!(oscillator.kind, "rack264_vco");
    assert_eq!(oscillator.params.len(), 6);
    for (name, value) in [
        ("soft_sync", 1.0),
        ("frequency", -54.0),
        ("fm_amount", -0.75),
        ("pulse_width", 0.99),
        ("pwm_amount", 0.625),
        ("linear_fm", 1.0),
    ] {
        assert_eq!(oscillator.params[name], value);
    }
    let filter = native(&result.document, 1);
    assert_eq!(filter.kind, "rack264_vcf");
    assert!(filter.bypassed);
    assert_eq!(filter.params.len(), 6);
    for (name, value) in [
        ("frequency", 0.625),
        ("resonance", 0.875),
        ("cutoff_cv_amount", -0.5),
        ("drive", -1.0),
        ("resonance_cv_amount", 0.0),
        ("drive_cv_amount", 0.75),
    ] {
        assert_eq!(filter.params[name], value);
    }
    let document = &result.document;
    assert_eq!(document.graph.cables[0].route.output, "saw");
    assert_eq!(document.graph.cables[0].route.input, "signal");
    assert_eq!(document.graph.cables[0].route.color, Some([234, 179, 8]));
    let package = &document.sources[&result.report.package_id];
    assert_eq!(*package.patch, patch);
    assert_eq!(
        document.source_blobs[&package.assets["patch.json"]].bytes(),
        serde_json::to_vec(&patch).unwrap()
    );
    assert_eq!(
        document.source_blobs[&package.assets["modules/2/opaque.bin"]].bytes(),
        b"retain"
    );
    for (index, model) in [(0, "VCO"), (1, "VCF")] {
        let mapping = document.graph.nodes[index]
            .source()
            .unwrap()
            .mapping
            .as_ref()
            .unwrap();
        assert_eq!(mapping.id, format!("vcv/Fundamental/{model}/2.6.4"));
        assert_eq!(mapping.fidelity, MappingFidelity::SourceBehavior);
    }
    assert!(result
        .report
        .warnings
        .iter()
        .any(|warning| warning.contains("previous-input")
            && warning.contains("undefined")
            && warning.contains("module-local")));
    let reopened = ProjectDocument::decode(&document.encode().unwrap()).unwrap();
    assert_eq!(reopened.encode().unwrap(), document.encode().unwrap());
    assert!(reopened
        .project(crate::project_document::PlaybackMode::Exact)
        .is_ok());
}

#[test]
fn rack264_vcf_loader_defaults_distinguish_absent_empty_partial_and_explicit_depths() {
    let mut modules = vec![module(1, "VCF")];
    for (index, params) in [
        json!([]),
        json!([{"id":0,"value":0.625}]),
        json!([{"id":5,"value":0},{"id":6,"value":0}]),
        json!([{"id":5,"value":-0.5},{"id":6,"value":0.75}]),
        json!([{"id":6,"value":0.25}]),
        json!([{"id":5,"value":-0.5}]),
    ]
    .into_iter()
    .enumerate()
    {
        let mut source = module(index as u64 + 2, "VCF");
        source["params"] = params;
        modules.push(source);
    }
    let result = run(json!({"modules":modules,"cables":[]})).unwrap();
    let catalog = openpatch_core::catalog();
    let defaults = &catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"] == "rack264_vcf")
        .unwrap()["params"];
    for (index, expected) in [
        (0, [0.0, 0.0]),
        (1, [1.0, 1.0]),
        (2, [1.0, 1.0]),
        (3, [0.0, 0.0]),
        (4, [-0.5, 0.75]),
        (5, [1.0, 0.25]),
        (6, [-0.5, 1.0]),
    ] {
        let node = native(&result.document, index);
        for (name, expected) in ["resonance_cv_amount", "drive_cv_amount"]
            .into_iter()
            .zip(expected)
        {
            let actual = node
                .params
                .get(name)
                .copied()
                .unwrap_or_else(|| defaults[name]["default"].as_f64().unwrap() as f32);
            assert_eq!(actual, expected, "module {index}, {name}");
        }
        assert!(!node.params.contains_key("resonance"));
        assert!(!node.params.contains_key("drive"));
    }
    assert_eq!(native(&result.document, 2).params["frequency"], 0.625);
}

#[test]
fn rack264_vco_sync_inversion_constructor_defaults_and_source_port_topology() {
    let mut hard = module(1, "VCO");
    hard["params"] = json!([{"id":1,"value":1},{"id":2,"value":54},{"id":5,"value":0.01}]);
    let cables: Vec<_> = (0..4)
        .map(|port| cable(port as u64 + 1, 1, port, 2, port))
        .chain((0..4).map(|port| cable(port as u64 + 5, 2, port % 2, 1, port)))
        .collect();
    let result =
        run(json!({"modules":[hard,module(2,"VCF"),module(3,"VCO")],"cables":cables})).unwrap();
    assert_eq!(native(&result.document, 0).params["soft_sync"], 0.0);
    assert_eq!(native(&result.document, 0).params["frequency"], 54.0);
    assert_eq!(native(&result.document, 0).params["pulse_width"], 0.01);
    assert!(native(&result.document, 2).params.is_empty());
    for (route, (output, input)) in result.document.graph.cables.iter().zip([
        ("sine", "cutoff_cv"),
        ("triangle", "resonance_cv"),
        ("saw", "drive_cv"),
        ("square", "signal"),
        ("lowpass", "pitch"),
        ("highpass", "fm"),
        ("lowpass", "sync"),
        ("highpass", "pwm"),
    ]) {
        assert_eq!(route.route.output, output);
        assert_eq!(route.route.input, input);
    }
    for (model, output, input) in [("VCO", 4, 0), ("VCO", 0, 4), ("VCF", 2, 0), ("VCF", 0, 4)] {
        assert!(
            run(json!({"modules":[module(1,model)],"cables":[cable(1,1,output,1,input)]})).is_err()
        );
    }
}

#[test]
fn rack264_voice_unrepresentable_parameters_and_unknown_data_remain_unresolved() {
    let catalog = openpatch_core::catalog();
    let filter = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"] == "rack264_vcf")
        .unwrap();
    let low = filter["params"]["frequency"]["min"].as_f64().unwrap() as f32;
    let high = filter["params"]["frequency"]["max"].as_f64().unwrap() as f32;
    for frequency in [low, high] {
        let mut source = module(1, "VCF");
        source["params"] = json!([{"id":0,"value":frequency}]);
        assert_eq!(
            run(json!({"modules":[source]}))
                .unwrap()
                .report
                .mapped_count,
            1
        );
    }
    for (model, id, value) in [
        ("VCO", 1, 0.5),
        ("VCO", 2, -54.01),
        ("VCO", 2, 54.01),
        ("VCO", 4, -1.01),
        ("VCO", 5, 0.0),
        ("VCO", 6, 1.01),
        ("VCO", 7, 0.5),
        ("VCO", 8, 0.0),
        ("VCF", 0, low - 0.001),
        ("VCF", 0, high + 0.001),
        ("VCF", 2, 1.01),
        ("VCF", 3, -1.01),
        ("VCF", 4, -1.01),
        ("VCF", 5, 1.01),
        ("VCF", 6, -1.01),
        ("VCF", 7, 0.0),
    ] {
        let mut source = module(1, model);
        source["params"] = json!([{"id":id,"value":value}]);
        let patch = json!({"modules":[source]});
        let result = run(patch.clone()).unwrap();
        assert_eq!(
            result.report.unresolved_count, 1,
            "{model} parameter {id}={value}"
        );
        assert!(result.report.modules[0].reason.is_some());
        assert_eq!(
            *result.document.sources[&result.report.package_id].patch,
            patch
        );
    }
    for model in ["VCO", "VCF"] {
        for data in [json!({"future":true}), json!([]), json!(1)] {
            let mut source = module(1, model);
            source["data"] = data;
            assert_eq!(
                run(json!({"modules":[source]}))
                    .unwrap()
                    .report
                    .unresolved_count,
                1
            );
        }
        for data in [json!({}), Value::Null] {
            let mut source = module(1, model);
            source["data"] = data;
            assert_eq!(
                run(json!({"modules":[source]}))
                    .unwrap()
                    .report
                    .mapped_count,
                1
            );
        }
        for version in [json!("2.6.3"), json!("2.6.5"), json!("2.6.6"), Value::Null] {
            let mut source = module(1, model);
            source["version"] = version;
            assert_eq!(
                run(json!({"modules":[source]}))
                    .unwrap()
                    .report
                    .unresolved_count,
                1
            );
        }
    }
}

#[test]
fn adsr_lfo_and_mixer_map_source_indices_units_and_durable_flags() {
    let mut adsr = module(1, "ADSR");
    adsr["params"] = json!([{"id":0,"value":0.0752575},{"id":7,"value":-0.75},{"id":8,"value":0}]);
    let mut lfo = module(2, "LFO");
    lfo["params"] = json!([{"id":2,"value":-2.321928},{"id":4,"value":0.7},{"id":5,"value":0.3}]);
    let mut mixer = module(3, "VCMixer");
    mixer["params"] = json!([{"id":0,"value":2},{"id":1,"value":1.3}]);
    mixer["data"] = json!({"chExp":true,"mixExp":1,"future":"retained"});
    let result =
        run(json!({"modules":[adsr,lfo,mixer],"cables":[cable(1,2,3,1,5),cable(2,1,0,3,5)]}))
            .unwrap();
    let doc = result.document;
    assert_eq!(native(&doc, 0).kind, "adsr_eg");
    assert_eq!(native(&doc, 0).params["attack"], 0.0752575);
    assert_eq!(native(&doc, 0).params["release_amount"], -0.75);
    assert_eq!(native(&doc, 1).params["rate"], -2.321928);
    assert_eq!(native(&doc, 1).params["pulse_width"], 0.3);
    assert_eq!(native(&doc, 2).params["mix_level"], 2.0);
    assert_eq!(native(&doc, 2).params["channel_exp"], 1.0);
    assert_eq!(native(&doc, 2).params["mix_exp"], 0.0);
    assert_eq!(doc.graph.cables[0].route.output, "square");
    assert_eq!(doc.graph.cables[0].route.input, "retrigger");
    assert_eq!(doc.graph.cables[1].route.input, "cv1");
    assert!(doc.state_carrier().is_ok());
}

#[test]
fn wavetable_lfo_uses_embedded_audio_and_preserves_all_original_archive_members() {
    let mut lfo = module(5, "LFO2");
    lfo["data"] = json!({"waveLen":2,"filename":"custom.wav","future":{"opaque":true}});
    lfo["params"] = json!([{"id":2,"value":-3.0},{"id":5,"value":-1.0}]);
    let patch = json!({"modules":[lfo],"cables":[]});
    // Source WAV sample-rate hint wins over module data, like Rack's onAdd load.
    let wave =
        openpatch_core::wavetable_file::encode_wavetable_wav(&[-1.0, 0.0, 0.5, 1.0], 4).unwrap();
    let bytes = tar_patch(
        &patch,
        &[
            ("modules/5/wavetable.wav", &wave),
            ("unknown/音.bin", b"opaque"),
        ],
    );
    let result = import(&bytes).unwrap();
    assert_eq!(result.report.mapped_count, 1);
    assert_eq!(result.report.asset_count, 2);
    let doc = &result.document;
    let node = native(doc, 0);
    match node.content.as_ref().unwrap() {
        ModuleContent::Wavetable {
            asset_id,
            wave_points,
            name,
        } => {
            assert_eq!(*wave_points, 4);
            assert_eq!(name, "custom.wav");
            assert_eq!(doc.graph.assets[asset_id].sample_count(), 4);
        }
        _ => panic!("table content missing"),
    }
    let package = &doc.sources[&result.report.package_id];
    assert_eq!(
        doc.source_blobs[&package.assets["modules/5/wavetable.wav"]].bytes(),
        wave
    );
    assert_eq!(
        doc.source_blobs[&package.assets["unknown/音.bin"]].bytes(),
        b"opaque"
    );
    assert_eq!(
        doc.encode().unwrap(),
        import(&bytes).unwrap().document.encode().unwrap()
    );
    assert!(doc.state_carrier().is_ok());
    let missing = run(patch).unwrap();
    assert_eq!(missing.report.unresolved_count, 1);
    assert!(missing.document.sources[&missing.report.package_id]
        .missing_assets
        .contains("modules/5/wavetable.wav"));
}

#[test]
fn unsupported_saved_controls_or_removed_incident_ports_never_quietly_drop() {
    let mut adsr = module(1, "ADSR");
    adsr["params"] = json!([{"id":8,"value":1}]);
    let mut vca = module(2, "VCA-1");
    vca["params"] = json!([{"id":55,"value":0.4}]);
    let result = run(
        json!({"modules":[adsr,vca,module(3,"LFO"),module(4,"Noise")],"cables":[cable(8,4,0,3,1)]}),
    )
    .unwrap();
    assert_eq!(
        (result.report.mapped_count, result.report.unresolved_count),
        (1, 3)
    );
    assert_eq!(result.document.graph.cables[0].route.input, "vcv_in_1");
    assert!(result
        .report
        .modules
        .iter()
        .take(3)
        .all(|module| module.reason.is_some()));
}

#[test]
fn rejects_malformed_ids_params_and_routes_before_returning_a_document() {
    let good = json!({"modules":[module(1,"Noise"),module(2,"VCA-1")],"cables":[cable(3,1,0,2,1)]});
    let mut bad = Vec::new();
    for id in [
        json!(-1),
        json!(1.5),
        json!("1"),
        json!(u64::MAX),
        Value::Null,
    ] {
        let mut value = good.clone();
        value["modules"][0]["id"] = id;
        bad.push(value);
    }
    let mut duplicate = good.clone();
    duplicate["modules"][1]["id"] = json!(1);
    bad.push(duplicate);
    let mut duplicate = good.clone();
    duplicate["cables"]
        .as_array_mut()
        .unwrap()
        .push(cable(3, 1, 0, 2, 0));
    bad.push(duplicate);
    let mut duplicate = good.clone();
    duplicate["cables"]
        .as_array_mut()
        .unwrap()
        .push(cable(4, 1, 1, 2, 1));
    bad.push(duplicate);
    for (field, value) in [
        ("inputModuleId", json!(99)),
        ("inputId", json!(-1)),
        ("outputId", json!(7)),
        ("inputId", json!(2)),
        ("color", json!("bad")),
        ("inputPlugOrder", json!(65536)),
    ] {
        let mut value_patch = good.clone();
        value_patch["cables"][0][field] = value;
        bad.push(value_patch);
    }
    for params in [
        json!({}),
        json!([{"id":0,"value":1e100}]),
        json!([{"id":0,"value":0},{"id":0,"value":1}]),
    ] {
        let mut value = good.clone();
        value["modules"][1]["params"] = params;
        bad.push(value);
    }
    for patch in bad {
        assert!(
            run(patch.clone()).is_err(),
            "accepted malformed patch: {patch}"
        );
    }
    assert!(run(good).is_ok());
}

#[test]
fn source_numeric_ids_stay_exact_and_separate_module_and_cable_namespaces() {
    let id = 9_007_199_254_740_993;
    let mut source = module(id, "VCA-1");
    source["pos"] = json!([0, 0]);
    let result = run(json!({"modules":[source],"cables":[cable(id,id,0,id,1)]})).unwrap();
    assert_eq!(
        result.document.graph.nodes[0].id(),
        "vcv_m_9007199254740993"
    );
    assert_eq!(
        result.document.graph.cables[0].route.id.as_deref(),
        Some("vcv_c_9007199254740993")
    );
}

/// Declared synthetic workflow cases, not a Rack numerical-equivalence oracle.
/// The native source is added through shared editing after real conversion.
#[test]
fn synthetic_polyphony_feedback_and_unknown_effect_collection_survives_shared_workflows() {
    use crate::control::{execute_shared, Session};
    use crate::project_document::{ExecutionDisposition, PlaybackMode};
    use openpatch_core::PortFrame;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    let workspace = std::env::temp_dir().join(format!(
        "openpatch-import-collection-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let drive: [f32; 16] = std::array::from_fn(|lane| (lane + 1) as f32 / 16.0);
    let asset_bytes = [0, 255, 128, 7];
    for (case, feedback, unknown_effect) in [
        ("polyphony", false, false),
        ("feedback", true, false),
        ("unsupported-effect", false, true),
    ] {
        let mut first = module(1, if feedback { "VCMixer" } else { "VCA-1" });
        first["params"] = if feedback {
            json!([{"id":0,"value":0.5},{"id":1,"value":1},{"id":2,"value":1}])
        } else {
            json!([{"id":0,"value":0.5},{"id":1,"value":1}])
        };
        let mut second = module(2, "VCMixer");
        second["params"] = json!([
            {"id":0,"value":if feedback {0.5} else {1.0}}, {"id":1,"value":1}
        ]);
        let mut modules = vec![first, second];
        let mut cables = vec![cable(10, 1, 0, 2, 1)];
        if feedback {
            cables.push(cable(11, 2, 0, 1, 2));
        }
        if unknown_effect {
            modules.push(
                json!({"id":3,"plugin":"SyntheticUnavailable","model":"Reverb",
                "version":"9.9.9","pos":[4,1],
                "data":{"impulse":"effects/impulse.bin","opaque":[1,{"future":true}]}}),
            );
            cables.extend([cable(12, 2, 0, 3, 0), cable(13, 3, 0, 1, 0)]);
        }
        let source = json!({"version":"2.6.4","synthetic_case":case,
            "modules":modules,"cables":cables,"future_global":{"retained":true}});
        let imported = import(&tar_patch(
            &source,
            &[("effects/impulse.bin", &asset_bytes)],
        ))
        .unwrap();
        let package_id = imported.report.package_id.clone();
        let assert_source = |document: &ProjectDocument| {
            let package = &document.sources[&package_id];
            assert_eq!(
                package.patch.as_ref(),
                &source,
                "{case}: source JSON changed"
            );
            assert_eq!(
                document.source_blobs[&package.assets["patch.json"]].bytes(),
                serde_json::to_vec(&source).unwrap()
            );
            assert_eq!(
                document.source_blobs[&package.assets["effects/impulse.bin"]].bytes(),
                asset_bytes
            );
        };
        assert_source(&imported.document);
        let shared = Session::new(&workspace).unwrap().into_shared();
        execute_shared(
            &shared,
            "replace_document",
            json!({"document":imported.document}),
        )
        .unwrap();
        let before_edit = shared.lock().unwrap().project_document().encode().unwrap();
        let mut params = serde_json::Map::from_iter([("channels".into(), json!(16))]);
        for (lane, value) in drive.iter().enumerate() {
            params.insert(format!("v{}", lane + 1), json!(value));
        }
        execute_shared(&shared, "apply_edits", json!({"edits":[
            {"tool":"add_module","arguments":{"id":"voices","kind":"poly_constant","params":params}},
            {"tool":"connect","arguments":{"from":"voices","to":"vcv_m_1",
                "input":if feedback {"in1"} else {"signal"},"cable_id":"native-drive"}},
            {"tool":"set_output","arguments":{"id":if feedback {"vcv_m_1"} else {"vcv_m_2"},"port":"mix"}}
        ]})).unwrap();
        let after_edit = shared.lock().unwrap().project_document().encode().unwrap();
        execute_shared(&shared, "undo", json!({})).unwrap();
        {
            let session = shared.lock().unwrap();
            assert_eq!(session.project_document().encode().unwrap(), before_edit);
            assert_source(session.project_document());
        }
        execute_shared(&shared, "redo", json!({})).unwrap();
        assert_eq!(
            shared.lock().unwrap().project_document().encode().unwrap(),
            after_edit
        );

        let mode = if unknown_effect {
            {
                let session = shared.lock().unwrap();
                assert_eq!(
                    session.execution_disposition(),
                    ExecutionDisposition::SuspendedUnresolved
                );
                assert!(session
                    .project_document()
                    .project(PlaybackMode::Exact)
                    .is_err());
                assert_source(session.project_document());
            }
            execute_shared(&shared, "set_document_playback", json!({"mode":"partial"})).unwrap();
            execute_shared(&shared, "undo", json!({})).unwrap();
            assert_eq!(
                shared.lock().unwrap().execution_disposition(),
                ExecutionDisposition::SuspendedUnresolved
            );
            execute_shared(&shared, "redo", json!({})).unwrap();
            assert_eq!(
                shared.lock().unwrap().execution_disposition(),
                ExecutionDisposition::RunnablePartial
            );
            PlaybackMode::Partial
        } else {
            assert_eq!(
                shared.lock().unwrap().execution_disposition(),
                ExecutionDisposition::RunnableExact
            );
            PlaybackMode::Exact
        };
        let projection = shared
            .lock()
            .unwrap()
            .project_document()
            .project(mode)
            .unwrap();
        assert_eq!(
            projection.patch.version, 2,
            "{case}: delayed engine2 scheduling required"
        );
        if feedback {
            for (id, from, to, input) in [
                ("vcv_c_10", "vcv_m_1", "vcv_m_2", "in1"),
                ("vcv_c_11", "vcv_m_2", "vcv_m_1", "in2"),
            ] {
                assert!(projection
                    .patch
                    .cables
                    .iter()
                    .any(|route| route.id.as_deref() == Some(id)
                        && route.from == from
                        && route.to == to
                        && route.input == input));
            }
        }
        if unknown_effect {
            assert_eq!(projection.unresolved_modules, ["vcv_m_3"]);
            for id in ["vcv_c_12", "vcv_c_13"] {
                assert!(projection
                    .omitted_routes
                    .iter()
                    .any(|route| route.cable_id == id));
            }
            assert!(!projection
                .patch
                .cables
                .iter()
                .any(|route| route.from == "vcv_m_3" || route.to == "vcv_m_3"));
        }
        let render = |patch: &openpatch_core::Patch| {
            let mut engine = patch.compile().unwrap();
            let mut frames = [PortFrame::default(); 128];
            engine.process_poly_block(&mut frames);
            assert!(
                frames
                    .iter()
                    .all(|frame| frame.values.iter().all(|v| v.is_finite())),
                "{case}: nonfinite native output"
            );
            frames
        };
        let frames = render(&projection.patch);
        assert_eq!(frames[0].values, [0.0; 16]);
        let settled_from = if feedback { 1 } else { 2 };
        assert!(
            frames[settled_from..]
                .iter()
                .all(|frame| frame.channels == 16),
            "{case}: lost a voice lane"
        );
        for (lane, value) in drive.iter().enumerate() {
            if feedback {
                // Each retained feedback cable contributes one sample of delay:
                // y[n] = .5*drive[n-1] + .25*y[n-2], initially zero.
                assert_eq!(frames[1].channel(lane), value * 0.5);
                assert_eq!(frames[2].channel(lane), value * 0.5);
                assert_eq!(frames[3].channel(lane), value * 0.625);
                assert!((frames[127].channel(lane) - value * (2.0 / 3.0)).abs() < 0.000001);
            } else {
                assert_eq!(frames[1].values, [0.0; 16]);
                assert!(
                    frames[2..]
                        .iter()
                        .all(|frame| frame.channel(lane) == value * 0.5),
                    "{case}: mapped VCA/mixer changed native lane {lane}"
                );
            }
        }
        let filename = format!("{case}.json");
        execute_shared(&shared, "save_document", json!({"filename":filename})).unwrap();
        let bytes = fs::read(workspace.join(&filename)).unwrap();
        let saved = ProjectDocument::decode(&bytes).unwrap();
        assert_source(&saved);
        execute_shared(&shared, "load_document", json!({"filename":filename})).unwrap();
        let reopened = {
            let session = shared.lock().unwrap();
            assert_eq!(
                session.playback_mode(),
                PlaybackMode::Exact,
                "partial permission must not persist"
            );
            assert_eq!(session.project_document().encode().unwrap(), bytes);
            assert_source(session.project_document());
            if unknown_effect {
                assert_eq!(
                    session.execution_disposition(),
                    ExecutionDisposition::SuspendedUnresolved
                );
                assert!(session
                    .project_document()
                    .project(PlaybackMode::Exact)
                    .is_err());
            }
            session.project_document().clone()
        };
        if unknown_effect {
            execute_shared(&shared, "set_document_playback", json!({"mode":"partial"})).unwrap();
        }
        let reopened_projection = reopened.project(mode).unwrap();
        let reopened_frames = render(&reopened_projection.patch);
        assert!(
            frames
                .iter()
                .zip(reopened_frames.iter())
                .all(|(before, after)| before.channels == after.channels
                    && before.values == after.values),
            "{case}: save/reload changed native projected processing"
        );
        assert_source(shared.lock().unwrap().project_document());
    }
    fs::remove_dir_all(workspace).unwrap();
}

#[test]
#[ignore = "requires OPENPATCH_VCV_IMPORT_FIXTURE pointing to the read-only user patch"]
fn actual_so_there_i_was_patch_preserves_complete_collection() {
    let path = std::env::var_os("OPENPATCH_VCV_IMPORT_FIXTURE").expect("fixture path required");
    let bytes = std::fs::read(path).unwrap();
    let archive = crate::vcv_archive::read(&bytes).unwrap();
    let source_patch: Value = serde_json::from_slice(&archive.patch_json).unwrap();
    let voice_counts = ["VCO", "VCF"].map(|model| {
        source_patch["modules"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|module| {
                module["plugin"] == "Fundamental"
                    && module["version"] == "2.6.4"
                    && module["model"] == model
            })
            .count()
    });
    assert_eq!(voice_counts, [1, 4]);
    let source_sequences = source_patch["modules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|module| {
            module["plugin"] == "RackMCP"
                && module["model"] == "Sequencer"
                && module["version"] == "2.0.0"
        })
        .count();
    assert_eq!(source_sequences, 2);
    let new_mappings: usize = voice_counts.into_iter().sum::<usize>() + source_sequences;
    let result = import(&bytes).unwrap();
    assert_eq!(
        (result.report.module_count, result.report.cable_count),
        (55, 83)
    );
    assert_eq!(
        (result.report.mapped_count, result.report.unresolved_count),
        (24 + new_mappings, 31 - new_mappings)
    );
    assert_eq!(result.report.asset_count, 1);
    let doc = result.document;
    assert_eq!(doc.graph.nodes.len(), 55);
    assert_eq!(doc.graph.cables.len(), 83);
    let sequences: Vec<_> = doc
        .graph
        .nodes
        .iter()
        .filter_map(|entry| match entry {
            DocumentNode::Native { node, .. } if node.kind == "rackmcp_sequencer" => {
                node.rackmcp_sequence.as_ref()
            }
            _ => None,
        })
        .collect();
    assert_eq!(sequences.len(), 2);
    assert_eq!(
        sequences
            .iter()
            .map(|state| state.data.lanes.len())
            .sum::<usize>(),
        13
    );
    assert_eq!(
        sequences
            .iter()
            .map(|state| state.targets.len())
            .sum::<usize>(),
        10
    );
    assert_eq!(doc.automation_omissions().unwrap().len(), 3);
    let source = &doc.sources[&result.report.package_id];
    assert_eq!(
        *source.patch,
        serde_json::from_slice::<Value>(&archive.patch_json).unwrap()
    );
    assert_eq!(
        doc.source_blobs[&source.assets["patch.json"]].bytes(),
        archive.patch_json
    );
    for (name, bytes) in &archive.assets {
        assert_eq!(doc.source_blobs[&source.assets[name]].bytes(), bytes);
    }
    assert!(doc
        .project(crate::project_document::PlaybackMode::Exact)
        .is_err());
    let partial = doc
        .project(crate::project_document::PlaybackMode::Partial)
        .unwrap();
    assert_eq!(partial.unresolved_modules.len(), 31 - new_mappings);
    assert_eq!(
        partial.omitted_routes.len(),
        result.report.omitted_route_count
    );
    let encoded = doc.encode().unwrap();
    let reopened = ProjectDocument::decode(&encoded).unwrap();
    assert_eq!(reopened.encode().unwrap(), encoded);
    assert_eq!(reopened.graph.cables.len(), 83);
}
