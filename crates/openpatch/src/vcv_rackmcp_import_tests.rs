use crate::project_document::{DocumentNode, MappingFidelity, ProjectDocument, SourceMapping};
use crate::vcv_import::{import, registered_source_mapping, source_automation_parameter};
use openpatch_core::rackmcp_sequence::{SequenceData, SequenceState};
use openpatch_core::Node;
use serde_json::{json, Value};

fn sequencer(id: u64) -> Value {
    json!({"id":id,"plugin":"RackMCP","model":"Sequencer","version":"2.0.0",
        "data":{"patterns":[{"name":"A","length":16}]}})
}

fn fundamental(id: u64, model: &str) -> Value {
    json!({"id":id,"plugin":"Fundamental","model":model,"version":"2.6.4"})
}

fn lane(module: u64, param: u16) -> Value {
    json!({"moduleId":module.to_string(),"paramId":param,"mode":"raw",
        "points":[{"bar":0,"value":0.375,"curve":"hold"},{"bar":1,"value":0.625}]})
}

fn native(document: &ProjectDocument, index: usize) -> &Node {
    match &document.graph.nodes[index] {
        DocumentNode::Native { node, .. } => node,
        _ => panic!("expected native node at index {index}"),
    }
}

#[test]
fn rackmcp_import_binds_only_qualified_targets_after_all_nodes_convert_and_retains_all_lanes() {
    let filter_id = 9_007_199_254_740_993_u64;
    let mut sequence = sequencer(314);
    sequence["params"] = json!([
        {"id":0,"value":128},{"id":1,"value":54.525020599365234},
        {"id":2,"value":1},{"id":3,"value":0}
    ]);
    sequence["data"] = json!({"version":1,"seed":1729,"selectedPattern":"B",
        "patterns":[{"name":"A","length":16},{"name":"B","length":4,
            "tracks":[{"voices":3,"glideMs":125,"steps":[
                {"step":0,"notes":[0,4,7],"vel":0.9,"accent":true},
                {"step":2,"notes":[],"ratchet":2,"slide":true}]}]}],
        "lanes":[lane(42,0),lane(filter_id,0),lane(42,1),lane(100,0),
            lane(123,0),lane(77,0),lane(78,0),lane(79,0)]});
    sequence["futureModuleMetadata"] = json!({"retain":[1,2,3]});
    let source_data: SequenceData = serde_json::from_value(sequence["data"].clone()).unwrap();
    let mut wrong_version = fundamental(77, "VCA-1");
    wrong_version["version"] = json!("2.6.5");
    let mut bad_filter = fundamental(78, "VCF");
    bad_filter["data"] = json!({"unsupported":true});
    let patch = json!({"version":"2.6.6","futurePatchMetadata":{"retain":true},
        "modules":[sequence,fundamental(42,"VCA-1"),fundamental(filter_id,"VCF"),
            {"id":100,"plugin":"Other","model":"VCA-1","version":"2.6.4"},
            wrong_version,bad_filter,fundamental(79,"ADSR")]});
    let bytes = serde_json::to_vec_pretty(&patch).unwrap();
    let result = import(&bytes).unwrap();
    assert_eq!(
        (result.report.mapped_count, result.report.unresolved_count),
        (4, 3)
    );
    let node = native(&result.document, 0);
    assert_eq!(node.kind, "rackmcp_sequencer");
    assert_eq!(node.params["bpm"], 128.0);
    assert_eq!(node.params["swing"], 54.525_02_f32);
    assert_eq!(node.params["running"], 1.0);
    assert_eq!(node.params["reset"], 0.0);
    let state = node.rackmcp_sequence.as_ref().unwrap();
    assert_eq!(state.data, source_data);
    assert_eq!(state.data.lanes.len(), 8);
    assert_eq!(state.targets.len(), 2);
    assert_eq!(state.targets[0].source_module_id, "42");
    assert_eq!(state.targets[0].source_param_id, 0);
    assert_eq!(state.targets[0].module, native(&result.document, 1).id);
    assert_eq!(state.targets[0].parameter, "level");
    assert_eq!(state.targets[1].source_module_id, filter_id.to_string());
    assert_eq!(state.targets[1].module, native(&result.document, 2).id);
    assert_eq!(state.targets[1].parameter, "frequency");
    let mapping = result.document.graph.nodes[0]
        .source()
        .unwrap()
        .mapping
        .as_ref()
        .unwrap();
    assert_eq!(mapping.id, "vcv/RackMCP/Sequencer/2.0.0");
    assert_eq!(mapping.version, 1);
    assert_eq!(mapping.fidelity, MappingFidelity::MusicalRole);
    assert!(result
        .report
        .warnings
        .iter()
        .any(|warning| warning.contains("MusicalRole")
            && warning.contains("self-unbinding")
            && warning.contains("scheduling")));
    assert!(result
        .report
        .warnings
        .iter()
        .any(|warning| warning.contains("6 RackMCP automation lane(s)")
            && warning.contains("without a binding")));
    let package = &result.document.sources[&result.report.package_id];
    assert_eq!(*package.patch, patch);
    assert_eq!(
        result.document.source_blobs[&package.assets["patch.json"]].bytes(),
        bytes
    );
    let reopened = ProjectDocument::decode(&result.document.encode().unwrap()).unwrap();
    assert_eq!(native(&reopened, 0).rackmcp_sequence, node.rackmcp_sequence);
}

#[test]
fn rackmcp_import_maps_all_source_port_indices_and_constructor_defaults() {
    let mut sequence = sequencer(12);
    sequence.as_object_mut().unwrap().remove("data");
    let mut cables = Vec::new();
    for output in 0..18 {
        cables.push(json!({"id":output,"outputModuleId":12,"outputId":output,
            "inputModuleId":999,"inputId":output}));
    }
    for input in 0..2 {
        cables.push(json!({"id":18+input,"outputModuleId":999,"outputId":input,
            "inputModuleId":12,"inputId":input}));
    }
    let patch = json!({"modules":[sequence,{"id":999,"plugin":"Unresolved","model":"Ports","version":"1"}],"cables":cables});
    let result = import(&serde_json::to_vec(&patch).unwrap()).unwrap();
    let node = native(&result.document, 0);
    let expected_params: std::collections::BTreeMap<String, f32> = [
        ("bpm".into(), 120.0),
        ("swing".into(), 50.0),
        ("running".into(), 0.0),
        ("reset".into(), 0.0),
    ]
    .into();
    assert_eq!(node.params, expected_params);
    assert_eq!(
        node.rackmcp_sequence.as_deref(),
        Some(&SequenceState::default())
    );
    let outputs = [
        "pitch1",
        "pitch2",
        "pitch3",
        "pitch4",
        "gate1",
        "gate2",
        "gate3",
        "gate4",
        "velocity1",
        "velocity2",
        "velocity3",
        "velocity4",
        "accent1",
        "accent2",
        "accent3",
        "accent4",
        "clock",
        "reset",
    ];
    for (cable, expected) in result.document.graph.cables.iter().zip(outputs) {
        assert_eq!(cable.route.output, expected);
    }
    assert_eq!(result.document.graph.cables[18].route.input, "clock");
    assert_eq!(result.document.graph.cables[19].route.input, "reset");
    assert_eq!(
        *result.document.sources[&result.report.package_id].patch,
        patch
    );
}

#[test]
fn rackmcp_unsupported_source_state_and_other_family_modules_stay_unresolved() {
    let valid_data = sequencer(1)["data"].clone();
    let mut unknown = valid_data.clone();
    unknown["unknown"] = json!(true);
    let mut long_pattern = valid_data.clone();
    long_pattern["patterns"][0]["length"] = json!(65);
    let mut bad_target = valid_data.clone();
    bad_target["lanes"] = json!([{"moduleId":"042","paramId":0,"points":[{"bar":0,"value":0}]}]);
    let mut too_many_lanes = valid_data.clone();
    too_many_lanes["lanes"] = Value::Array((0..9).map(|id| lane(id, 0)).collect());
    for bad_data in [
        Value::Null,
        json!({}),
        unknown,
        long_pattern,
        bad_target,
        too_many_lanes,
    ] {
        let mut module = sequencer(1);
        module["data"] = bad_data;
        let patch = json!({"modules":[module]});
        let result = import(&serde_json::to_vec(&patch).unwrap()).unwrap();
        assert!(
            matches!(&result.document.graph.nodes[0], DocumentNode::Unresolved { source, .. } if source.mapping.is_none())
        );
        assert_eq!(
            *result.document.sources[&result.report.package_id].patch,
            patch
        );
        assert!(result.report.modules[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("sequence data"));
    }
    let mut wrong_version = sequencer(1);
    wrong_version["version"] = json!("2.0.1");
    let mut bad_params = sequencer(2);
    bad_params["params"] = json!([{"id":0,"value":301}]);
    let mut bridge = sequencer(3);
    bridge["model"] = json!("Bridge");
    let mut listen = sequencer(4);
    listen["model"] = json!("Listen");
    let patch = json!({"modules":[wrong_version,bad_params,bridge,listen]});
    let result = import(&serde_json::to_vec(&patch).unwrap()).unwrap();
    assert_eq!(
        (result.report.mapped_count, result.report.unresolved_count),
        (0, 4)
    );
    assert_eq!(
        *result.document.sources[&result.report.package_id].patch,
        patch
    );
}

#[test]
fn source_automation_requires_exact_registered_source_mapping_and_live_parameter() {
    for (model, kind, parameter) in [
        ("VCA-1", "stock_vca", "level"),
        ("VCF", "rack264_vcf", "frequency"),
    ] {
        let module = fundamental(42, model);
        let mapping = registered_source_mapping(&module).unwrap();
        assert_eq!(mapping.id, format!("vcv/Fundamental/{model}/2.6.4"));
        assert_eq!(mapping.version, 1);
        assert_eq!(mapping.fidelity, MappingFidelity::SourceBehavior);
        assert_eq!(
            source_automation_parameter(&module, &mapping, kind, 0),
            Some(parameter)
        );
        assert_eq!(
            source_automation_parameter(&module, &mapping, kind, 1),
            None
        );
        assert_eq!(
            source_automation_parameter(&module, &mapping, "stock_vco", 0),
            None
        );
        for bad in [
            SourceMapping {
                id: "vcv/Other/VCA-1/2.6.4".into(),
                ..mapping.clone()
            },
            SourceMapping {
                version: 2,
                ..mapping.clone()
            },
            SourceMapping {
                fidelity: MappingFidelity::UserSubstitution,
                ..mapping.clone()
            },
        ] {
            assert_eq!(source_automation_parameter(&module, &bad, kind, 0), None);
        }
        for (field, value) in [
            ("version", "2.6.5"),
            ("plugin", "Other"),
            ("model", "Other"),
        ] {
            let mut forged = module.clone();
            forged[field] = json!(value);
            assert!(registered_source_mapping(&forged).is_none());
            assert_eq!(
                source_automation_parameter(&forged, &mapping, kind, 0),
                None
            );
        }
    }
    let module = sequencer(1);
    let mapping = registered_source_mapping(&module).unwrap();
    assert_eq!(
        source_automation_parameter(&module, &mapping, "rackmcp_sequencer", 0),
        None
    );
}
