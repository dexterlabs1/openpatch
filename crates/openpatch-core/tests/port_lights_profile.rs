//! Explicit offline release-only CPU measurement; root runs serially with
//! --release --test port_lights_profile -- --ignored --nocapture --test-threads=1.
//! No device/native-host/realtime qualification is implied by this measurement.
use openpatch_core::{Engine, Node, Patch, PortDirection, PortFrame, MAX_CABLES};
use serde_json::json;
use std::hint::black_box;
use std::time::Instant;

// Every physical input and output is distinct. A native 16-channel source feeds
// a chain through all eight independent rows of each 8VERT. The largest fixture
// uses exactly1024 cables,2048 unique endpoints,129 nodes, all measured at16 lanes.
fn dense(cables: usize) -> Patch {
    assert_eq!(cables % 8, 0);
    let mut p = Patch::new_v2();
    p.nodes.push(Node {
        id: "source".into(),
        kind: "poly_constant".into(),
        bypassed: false,
        content: None,
        score: None,
        params: std::iter::once(("channels".into(), 16.0))
            .chain((1..=16).map(|lane| (format!("v{lane}"), lane as f32 - 8.5)))
            .collect(),
    });
    for node in 0..cables / 8 {
        p.nodes.push(Node {
            id: format!("a{node}"),
            kind: "attenuverter8".into(),
            bypassed: false,
            content: None,
            score: None,
            params: (1..=8).map(|row| (format!("gain{row}"), 1.0)).collect(),
        });
    }
    for index in 0..cables {
        p.cables.push(
            serde_json::from_value(json!({
                "id":format!("c{index}"),
                "from":if index==0 {"source".into()} else {format!("a{}",(index-1)/8)},
                "output":if index==0 {"out".into()} else {format!("out{}",(index-1)%8+1)},
                "to":format!("a{}",index/8),"input":format!("in{}",index%8+1)
            }))
            .unwrap(),
        );
    }
    p.output = Some(format!("a{}", cables / 8 - 1));
    p.output_port = "out8".into();
    p
}
fn hash(frames: &[PortFrame]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for frame in frames {
        hash = (hash ^ u64::from(frame.channels)).wrapping_mul(0x100000001b3);
        for value in frame.values {
            hash = (hash ^ u64::from(value.to_bits())).wrapping_mul(0x100000001b3);
        }
    }
    hash
}
fn measure(engine: &mut Engine, output: &mut [PortFrame]) -> (f64, u64) {
    let start = Instant::now();
    engine.process_poly_block(black_box(output));
    black_box(&*output);
    let ns = start.elapsed().as_secs_f64() * 1e9 / output.len() as f64;
    (ns, hash(output))
}
fn percentile(values: &[f64], q: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[((sorted.len() as f64 * q).ceil() as usize).saturating_sub(1)]
}
#[test]
#[ignore = "manual release CPU profile; serial root build only"]
// A runtime guard rejects manual debug runs while allowing debug test discovery.
#[allow(clippy::assertions_on_constants)]
fn profile_dense_connected_polyphonic_observation() {
    assert!(
        !cfg!(debug_assertions),
        "run this measurement with --release"
    );
    let mut profiles = Vec::new();
    for cables in [8, 128, MAX_CABLES] {
        let p = dense(cables);
        let frames = (1_048_576 / cables).clamp(4096, 65536);
        let mut off = Vec::new();
        let mut on = Vec::new();
        let mut delta = Vec::new();
        let mut hashes = Vec::new();
        for trial in 0..11 {
            let mut disabled = p.compile().unwrap();
            let mut enabled = p.compile().unwrap();
            enabled.set_port_lights_enabled(true);
            let handles: Vec<_> = p
                .cables
                .iter()
                .flat_map(|c| {
                    [
                        enabled
                            .resolve_port_light(&c.from, PortDirection::Output, &c.output)
                            .unwrap(),
                        enabled
                            .resolve_port_light(&c.to, PortDirection::Input, &c.input)
                            .unwrap(),
                    ]
                })
                .collect();
            let mut warmup = vec![PortFrame::default(); 2048];
            disabled.process_poly_block(&mut warmup);
            enabled.process_poly_block(&mut warmup);
            // Validate actual workload width and signal; topology alone cannot
            // substantiate a claimed dense sixteen-lane detector measurement.
            for h in &handles {
                let s = enabled.port_light(*h).unwrap();
                assert!(s.connected && !s.invalid && s.channels == 16 && s.raw_rgb[2] > 0.0);
            }
            let mut a = vec![PortFrame::default(); frames];
            let mut b = vec![PortFrame::default(); frames];
            let (ta, tb) = if trial % 2 == 0 {
                let a = measure(&mut disabled, &mut a);
                let b = measure(&mut enabled, &mut b);
                (a, b)
            } else {
                let b = measure(&mut enabled, &mut b);
                let a = measure(&mut disabled, &mut a);
                (a, b)
            };
            assert_eq!(ta.1, tb.1, "enabled/disabled output hash differs");
            assert_eq!(a, b);
            off.push(ta.0);
            on.push(tb.0);
            delta.push(tb.0 - ta.0);
            hashes.push(format!("{:016x}", ta.1));
        }
        let report = json!({"cables":cables,"connected_unique_endpoints":2*cables,"nodes":p.nodes.len(),"channels_per_endpoint":16,
            "frames_per_trial":frames,"warmup_frames":2048,"trials":11,"alternating_order":true,
            "disabled_ns_per_frame":{"median":percentile(&off,0.5),"p95":percentile(&off,0.95),"trials":off},
            "enabled_ns_per_frame":{"median":percentile(&on,0.5),"p95":percentile(&on,0.95),"trials":on},
            "incremental_ns_per_frame":{"median":percentile(&delta,0.5),"p95":percentile(&delta,0.95),"trials":delta},
            "output_hashes_equal":true,"output_hashes_fnv1a64":hashes});
        println!("{}", serde_json::to_string(&report).unwrap());
        profiles.push(report);
    }
    if let Some(path) = std::env::var_os("OPENPATCH_PORT_LIGHT_PROFILE_PATH") {
        let report = json!({"schema":1,"kind":"offline core release CPU measurement","sample_rate":48000,
            "architecture":std::env::consts::ARCH,"os":std::env::consts::OS,"timing_scope":"process_poly_block + black_box; hashing/assertions outside timer",
            "detector_cadence_samples":7,"physical_or_native_host_qualification":false,
            "source_revision":std::env::var("OPENPATCH_PORT_LIGHT_PROFILE_REVISION").ok(),"profiles":profiles});
        std::fs::write(path, serde_json::to_string_pretty(&report).unwrap() + "\n").unwrap();
    }
}
