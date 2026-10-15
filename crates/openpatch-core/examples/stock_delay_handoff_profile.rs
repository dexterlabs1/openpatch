//! Offline state-adoption microprofile; no device or callback deadline claim.
use openpatch_core::{Engine, Patch, PortFrame};
use serde_json::{json, Value};
use std::{hint::black_box, time::Instant};

fn patch(count: usize) -> Patch {
    let mut nodes = vec![json!({"id":"source","kind":"constant","params":{"value":0.5}})];
    let mut cables = Vec::new();
    for index in 0..count {
        let id = format!("delay{index}");
        nodes.push(json!({"id":id,"kind":"stock_delay","params":{"time":0.3}}));
        cables
            .push(json!({"id":format!("signal{index}"),"from":"source","to":id,"input":"signal"}));
    }
    serde_json::from_value(json!({"version":2,"sample_rate":48000,"nodes":nodes,"cables":cables,"output":"delay0","output_port":"mix"})).unwrap()
}
fn summarize(samples: &[f64]) -> Value {
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    json!({"observations_us_per_operation":samples,"median_us":sorted[sorted.len()/2],"maximum_us":sorted[sorted.len()-1]})
}
fn measure(count: usize) -> Value {
    let patch = patch(count);
    let mut donor: Engine = patch.compile().unwrap();
    let mut receiver = patch.compile().unwrap();
    let mut audio = [PortFrame::default(); 128];
    for _ in 0..32 {
        donor.process_poly_block(&mut audio);
    }
    let mut copies = Vec::with_capacity(33);
    let mut swaps = Vec::with_capacity(33);
    for trial in 0..33 {
        // Reverse measurement order on alternate trials. All graph construction,
        // allocations, observations and retirement stay outside timed calls.
        let mut copy = || {
            let start = Instant::now();
            receiver.adopt_state_from(black_box(&donor));
            start.elapsed().as_secs_f64() * 1e6
        };
        let copy_first = trial % 2 == 0;
        if copy_first {
            copies.push(copy());
        }
        let start = Instant::now();
        let mut all_moved = true;
        for _ in 0..128 {
            all_moved &= receiver.handoff_state_from(black_box(&mut donor));
            all_moved &= donor.handoff_state_from(black_box(&mut receiver));
        }
        let elapsed = start.elapsed().as_secs_f64() * 1e6 / 256.0;
        black_box(&receiver);
        black_box(&donor);
        assert!(all_moved);
        swaps.push(elapsed);
        if !copy_first {
            let start = Instant::now();
            receiver.adopt_state_from(black_box(&donor));
            copies.push(start.elapsed().as_secs_f64() * 1e6);
        }
    }
    json!({"delay_count":count,"history_bytes_per_delay":8388608,"trials":33,
        "immutable_independent_copy":summarize(&copies),"destructive_ownership_handoff":summarize(&swaps),
        "handoff_operations_per_trial":256,"copy_operations_per_trial":1,"patch":patch})
}
fn main() {
    if cfg!(debug_assertions) {
        panic!("use --release");
    }
    println!("{}",serde_json::to_string_pretty(&json!({
        "scope":"Offline Engine state-adoption microprofile. Includes graph bookkeeping, excludes graph compilation, rendering, device scheduling and donor retirement. Independent copies retain both engines; destructive swaps require retiring each donor before processing it again. This is not a Rack CPU comparison or a complete desktop callback deadline measurement.",
        "fixture":"Two prepared compatible graphs; 4096 warmup frames, alternating trial order, resident histories; handoffs batched to reduce timer overhead.",
        "profiles":[measure(1),measure(8)]
    })).unwrap());
}
