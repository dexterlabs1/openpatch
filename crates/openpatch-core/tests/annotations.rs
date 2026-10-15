use openpatch_core::{
    catalog, validate_content, Cable, ModuleContent, ModuleParameterActions, Node, Patch,
    DEFAULT_BLANK_WIDTH_HP, MAX_BLANK_WIDTH_HP, MAX_NOTES_BYTES, MIN_BLANK_WIDTH_HP,
};
use serde_json::json;

fn node(id: &str, kind: &str, content: Option<ModuleContent>) -> Node {
    Node {
        content,
        score: None,
        id: id.into(),
        kind: kind.into(),
        bypassed: false,
        params: Default::default(),
    }
}
fn patch() -> Patch {
    Patch {
        nodes: vec![node("n", "notes", None), node("b", "blank", None)],
        ..Patch::new_v2()
    }
}
fn cable(from: &str, to: &str) -> Cable {
    serde_json::from_value(json!({"from":from,"to":to,"input":"signal"})).unwrap()
}

#[test]
fn defaults_and_old_json_remain_compact_and_compatible() {
    let original = json!({"id":"old","kind":"constant","params":{}});
    let old: Node = serde_json::from_value(original.clone()).unwrap();
    assert_eq!(serde_json::to_value(old).unwrap(), original);
    let p = patch();
    assert_eq!(p.nodes[0].effective_notes_text(), Some(""));
    assert_eq!(
        p.nodes[1].effective_blank_width_hp(),
        Some(DEFAULT_BLANK_WIDTH_HP)
    );
    assert_eq!(p.nodes[0].effective_blank_width_hp(), None);
    assert_eq!(p.nodes[1].effective_notes_text(), None);
    p.validate().unwrap();
    let mut e = p.compile().unwrap();
    let mut output = [1.0; 129];
    e.process_block(&mut output);
    assert_eq!(output, [0.0; 129]);
    assert!(!serde_json::to_string(&p).unwrap().contains("content"));
}

#[test]
fn unicode_utf8_byte_limit_and_typed_width_bounds_round_trip() {
    let text = "🎛\n\t".repeat(MAX_NOTES_BYTES / 6);
    let mut p = patch();
    p.nodes[0].content = Some(ModuleContent::Notes { text: text.clone() });
    p.nodes[1].content = Some(ModuleContent::Blank {
        width_hp: MAX_BLANK_WIDTH_HP,
    });
    let encoded = serde_json::to_string(&p).unwrap();
    let decoded: Patch = serde_json::from_str(&encoded).unwrap();
    decoded.validate().unwrap();
    assert_eq!(decoded.nodes[0].effective_notes_text(), Some(text.as_str()));
    assert_eq!(
        decoded.nodes[1].effective_blank_width_hp(),
        Some(MAX_BLANK_WIDTH_HP)
    );
    p.nodes[0].content = Some(ModuleContent::Notes {
        text: "é".repeat(MAX_NOTES_BYTES / 2),
    });
    p.validate().unwrap();
    if let Some(ModuleContent::Notes { text }) = &mut p.nodes[0].content {
        text.push('é');
    }
    assert!(p.validate().unwrap_err().contains("UTF-8 bytes"));
    p.nodes[0].content = None;
    for width_hp in [MIN_BLANK_WIDTH_HP, MAX_BLANK_WIDTH_HP] {
        p.nodes[1].content = Some(ModuleContent::Blank { width_hp });
        p.validate().unwrap();
    }
    for width_hp in [0, MIN_BLANK_WIDTH_HP - 1, MAX_BLANK_WIDTH_HP + 1, u16::MAX] {
        p.nodes[1].content = Some(ModuleContent::Blank { width_hp });
        assert!(validate_content(&p.nodes[1]).is_err());
        assert!(p.validate().is_err());
    }
}

#[test]
fn content_is_strict_and_requires_matching_version_two_kind() {
    for content in [
        json!({"type":"notes","text":"x","extra":1}),
        json!({"type":"notes"}),
        json!({"type":"blank","width_hp":10,"text":"x"}),
        json!({"type":"blank","width_hp":3.5}),
        json!({"type":"blank","width_hp":-1}),
        json!({"type":"blank","width_hp":65536}),
        json!({"type":"unknown","text":"x"}),
    ] {
        assert!(serde_json::from_value::<ModuleContent>(content).is_err());
    }
    let mut p = patch();
    p.nodes[0].content = Some(ModuleContent::Blank { width_hp: 10 });
    assert!(p.validate().is_err());
    p.nodes[0].kind = "constant".into();
    assert!(p.validate().is_err());
    p = patch();
    p.version = 1;
    assert!(p.validate().is_err());
    p.nodes = vec![node(
        "old",
        "constant",
        Some(ModuleContent::Notes { text: "x".into() }),
    )];
    assert!(p.validate().is_err());
    p = patch();
    p.nodes[0].params.insert("text".into(), 1.0);
    assert!(p.validate().is_err());
}

#[test]
fn annotations_have_no_audio_capture_or_cable_ports_even_when_bypassed() {
    for kind in ["notes", "blank"] {
        let mut p = Patch {
            nodes: vec![node("annotation", kind, None), node("dc", "constant", None)],
            ..Patch::new_v2()
        };
        for bypassed in [false, true] {
            p.nodes[0].bypassed = bypassed;
            let e = p.compile().unwrap();
            for port in ["out", "signal", "out1", ""] {
                assert!(e.resolve_tap("annotation", port, 0).is_err());
                p.output = Some("annotation".into());
                p.output_port = port.into();
                assert!(p.validate().is_err());
            }
            p.output = None;
            p.output_port = "out".into();
            p.cables = vec![cable("annotation", "dc")];
            assert!(p.validate().unwrap_err().contains("unknown output"));
            p.cables = vec![cable("dc", "annotation")];
            assert!(p.validate().unwrap_err().contains("unknown input"));
            p.cables.clear();
        }
    }
}

#[test]
fn catalog_and_initialize_describe_content_without_signal_controls() {
    let c = catalog();
    let actions = ModuleParameterActions::new(48_000);
    for (kind, content) in [
        (
            "notes",
            ModuleContent::Notes {
                text: "保存\nnotes".into(),
            },
        ),
        ("blank", ModuleContent::Blank { width_hp: 32 }),
    ] {
        let descriptor = c["modules"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == kind)
            .unwrap();
        assert_eq!(descriptor["inputs"], json!([]));
        assert_eq!(descriptor["outputs"], json!([]));
        assert_eq!(descriptor["params"], json!({}));
        assert_eq!(descriptor["min_patch_version"], 2);
        assert_eq!(descriptor["content_schema"]["additionalProperties"], false);
        assert_eq!(
            descriptor["content_schema"]["properties"]["type"]["const"],
            kind
        );
        let mut n = node("annotation", kind, Some(content));
        let saved = n.content.clone();
        assert!(actions.randomize(&mut n, 42).is_err());
        assert_eq!(n.content, saved);
        actions.initialize(&mut n).unwrap();
        assert_eq!(n.content, None);
        assert_eq!(n.effective_notes_text(), (kind == "notes").then_some(""));
        assert_eq!(
            n.effective_blank_width_hp(),
            (kind == "blank").then_some(DEFAULT_BLANK_WIDTH_HP)
        );
    }
}
