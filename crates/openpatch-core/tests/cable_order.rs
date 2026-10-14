use openpatch_core::{
    assign_new_cable_orders, cable_endpoint_order, promote_cable_endpoints, Cable, CableEndpoint,
    Patch, MAX_CABLES,
};
use serde_json::json;

fn cable(id: &str) -> Cable {
    serde_json::from_value(json!({"id":id,"from":"source","to":"sink","input":"signal"})).unwrap()
}

#[test]
fn old_missing_and_tied_ranks_resolve_deterministically_without_migration() {
    let mut cables = vec![cable("a"), cable("b"), cable("c"), cable("other")];
    cables[3].to = "elsewhere".into();
    cables[1].input_order = Some(0);
    cables[2].input_order = Some(65535);
    cables[0].output_order = Some(65535);
    cables[1].output_order = Some(4);
    cables[2].output_order = Some(4);
    let before = serde_json::to_value(&cables).unwrap();
    assert_eq!(
        cable_endpoint_order(&cables, CableEndpoint::Input, "sink", "signal"),
        [0, 1, 2]
    );
    assert_eq!(
        cable_endpoint_order(&cables, CableEndpoint::Output, "source", "out"),
        [3, 1, 2, 0]
    );
    assert!(cable_endpoint_order(&cables, CableEndpoint::Input, "missing", "signal").is_empty());
    assert_eq!(serde_json::to_value(&cables).unwrap(), before);
    let legacy = cable("legacy");
    let json = serde_json::to_value(&legacy).unwrap();
    assert!(json.get("input_order").is_none());
    assert!(json.get("output_order").is_none());
}

#[test]
fn strict_u16_fields_roundtrip_and_reject_invalid_wire_values() {
    for field in ["input_order", "output_order"] {
        for rank in [
            json!(-1),
            json!(65536),
            json!(0.5),
            json!(1.0),
            json!("1"),
            json!(true),
            json!([]),
            json!({}),
        ] {
            let mut value = serde_json::to_value(cable("route")).unwrap();
            value[field] = rank;
            assert!(
                serde_json::from_value::<Cable>(value.clone()).is_err(),
                "{value}"
            );
        }
    }
    for version in [1, 2] {
        let value = json!({"version":version,"nodes":[{"id":"source","kind":"constant"},{"id":"sink","kind":"gain"}],
            "cables":[{"id":"route","from":"source","to":"sink","input":"signal","input_order":65535,"output_order":0}]});
        let patch: Patch = serde_json::from_value(value).unwrap();
        patch.validate().unwrap();
        let restored: Patch = serde_json::from_slice(&serde_json::to_vec(&patch).unwrap()).unwrap();
        assert_eq!(restored.cables[0].input_order, Some(65535));
        assert_eq!(restored.cables[0].output_order, Some(0));
    }
}

#[test]
fn promotion_changes_only_one_end_and_effective_top_noop_never_normalizes() {
    let mut cables = vec![cable("a"), cable("b"), cable("c"), cable("unrelated")];
    cables[0].output_order = Some(65535);
    cables[1].output_order = Some(5);
    cables[2].output_order = None;
    cables[3].to = "elsewhere".into();
    cables[3].input_order = Some(65535);
    let opposite: Vec<_> = cables.iter().map(|c| c.output_order).collect();
    assert!(promote_cable_endpoints(&mut cables, CableEndpoint::Input, &[0]).unwrap());
    assert_eq!(
        cable_endpoint_order(&cables, CableEndpoint::Input, "sink", "signal"),
        [1, 2, 0]
    );
    assert_eq!(
        cables.iter().map(|c| c.output_order).collect::<Vec<_>>(),
        opposite
    );
    assert_eq!(
        cables
            .iter()
            .map(|c| c.id.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["a", "b", "c", "unrelated"]
    );
    let snapshot = serde_json::to_value(&cables).unwrap();
    assert!(!promote_cable_endpoints(&mut cables, CableEndpoint::Input, &[0]).unwrap());
    assert_eq!(serde_json::to_value(&cables).unwrap(), snapshot);
    // A top at an unrelated jack is a local no-op even below global ranks.
    assert!(!promote_cable_endpoints(&mut cables, CableEndpoint::Input, &[3]).unwrap());
    assert_eq!(serde_json::to_value(&cables).unwrap(), snapshot);
}

#[test]
fn fresh_cables_append_both_ends_and_overflow_preserves_existing_relations() {
    let mut cables = vec![cable("a"), cable("b"), cable("new")];
    cables[0].input_order = Some(65535);
    cables[1].input_order = Some(1);
    cables[0].output_order = Some(1);
    cables[1].output_order = Some(65535);
    assign_new_cable_orders(&mut cables, &[2]).unwrap();
    assert_eq!(
        cable_endpoint_order(&cables, CableEndpoint::Input, "sink", "signal"),
        [1, 0, 2]
    );
    assert_eq!(
        cable_endpoint_order(&cables, CableEndpoint::Output, "source", "out"),
        [0, 1, 2]
    );
    assert_eq!(cables[2].input_order, Some(3));
    assert_eq!(cables[2].output_order, Some(3));
    let mut next = cable("next");
    next.input_order = Some(65535); // Copied ranks must be ignored for new IDs.
    next.output_order = Some(0);
    cables.push(next);
    assign_new_cable_orders(&mut cables, &[3]).unwrap();
    assert_eq!(cables[3].input_order, Some(4));
    assert_eq!(cables[3].output_order, Some(4));
}

#[test]
fn full_limit_repeated_promotions_stay_bounded_and_invalid_selection_is_atomic() {
    let mut cables: Vec<_> = (0..MAX_CABLES)
        .map(|index| {
            let mut c = cable(&format!("route_{index}"));
            c.output_order = Some((MAX_CABLES - index) as u16);
            c
        })
        .collect();
    let opposite: Vec<_> = cables.iter().map(|c| c.output_order).collect();
    for index in 0..2050 {
        promote_cable_endpoints(&mut cables, CableEndpoint::Input, &[index % MAX_CABLES]).unwrap();
        assert!(cables.iter().all(|c| c
            .input_order
            .is_some_and(|rank| usize::from(rank) <= MAX_CABLES)));
    }
    assert_eq!(
        cables.iter().map(|c| c.output_order).collect::<Vec<_>>(),
        opposite
    );
    let before = serde_json::to_value(&cables).unwrap();
    for selected in [
        vec![],
        vec![0, 0],
        vec![MAX_CABLES],
        vec![0; MAX_CABLES + 1],
    ] {
        assert!(promote_cable_endpoints(&mut cables, CableEndpoint::Input, &selected).is_err());
        assert_eq!(serde_json::to_value(&cables).unwrap(), before);
    }
}
