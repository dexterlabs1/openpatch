use super::*;
use crate::control::Session;

fn empty_session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute("replace_patch", json!({"patch": {"version": 2, "sample_rate": 48000, "nodes": [], "cables": []}})).unwrap();
    s
}

fn apply_all(edits: &[Value]) -> Result<Session, String> {
    let mut s = empty_session();
    for chunk in edits.chunks(256) {
        s.execute("apply_edits", json!({"edits": chunk}))?;
    }
    Ok(s)
}

fn flatten(steps: &[Value]) -> Vec<Value> {
    steps.iter().flat_map(|s| s["edits"].as_array().unwrap().clone()).collect()
}

#[test]
fn every_word_combination_builds_in_every_mode() {
    let mut fails = vec![];
    for g in GENRES {
        for m in MOODS {
            for t in TEXTURES {
                for seed in [1u64, 7, 4242] {
                    let r = Recipe::new(g, m, t, seed).unwrap();
                    let tag = format!("{g}|{m}|{t}|{seed}");
                    // plan caps used by start_construction
                    let (mods, steps) = r.reveal();
                    let show = r.showtime();
                    for (name, plan) in [("reveal", &steps), ("showtime", &show)] {
                        assert!(plan.len() <= 64, "{tag}: {name} has {} steps", plan.len());
                        for s in plan.iter() {
                            assert!(s["edits"].as_array().unwrap().len() <= 8, "{tag}: {name} step over 8 edits");
                        }
                        assert!(flatten(plan).len() <= 256, "{tag}: {name} over 256 edits");
                    }
                    for (name, edits) in [("instant", r.edits()), ("reveal", [mods.clone(), flatten(&steps)].concat()), ("showtime", flatten(&show))] {
                        if let Err(e) = apply_all(&edits) { fails.push(format!("{tag} {name}: {e}")); }
                    }
                }
            }
        }
        assert!(fails.is_empty(), "{} failures, first: {:?}", fails.len(), &fails[..fails.len().min(5)]);
    }
}

#[test]
fn final_graph_matches_the_offline_patch() {
    let r = Recipe::new("Deep House", "Dreamy", "Glitchy", 3).unwrap();
    let s = apply_all(&r.edits()).unwrap();
    let live = serde_json::to_value(s.patch()).unwrap();
    let want = r.patch();
    assert_eq!(live["nodes"].as_array().unwrap().len(), want["nodes"].as_array().unwrap().len());
    assert_eq!(live["cables"].as_array().unwrap().len(), want["cables"].as_array().unwrap().len());
    let route = |c: &Value| (c["from"].as_str().unwrap_or("").to_string(), c["to"].as_str().unwrap_or("").to_string(), c["input"].as_str().unwrap_or("").to_string());
    let mut a: Vec<_> = live["cables"].as_array().unwrap().iter().map(route).collect();
    let mut b: Vec<_> = want["cables"].as_array().unwrap().iter().map(route).collect();
    a.sort(); b.sort();
    assert_eq!(a, b);
    // the offline patch is a valid engine patch on its own
    let patch: openpatch_core::Patch = serde_json::from_value(want).unwrap();
    openpatch_core::render(&patch, 0.05).unwrap();
}

#[test]
fn seeds_compose_different_pieces_and_are_reproducible() {
    let a = Recipe::new("Techno", "Hypnotic", "Liquid", 11).unwrap();
    let b = Recipe::new("Techno", "Hypnotic", "Liquid", 11).unwrap();
    assert_eq!(a.patch(), b.patch());
    let mut seen = std::collections::HashSet::new();
    for seed in 1..=20u64 {
        seen.insert(serde_json::to_string(&Recipe::new("Techno", "Hypnotic", "Liquid", seed).unwrap().patch()).unwrap());
    }
    assert_eq!(seen.len(), 20);
}

#[test]
fn chord_voicings_never_stack_seconds_and_stay_in_range() {
    for m in MOODS {
        for seed in 1..=40u64 {
            let mut rng = Rng::new(&format!("v{m}{seed}"));
            let md = mood_def(m);
            let tonic = rng.pick(md.keys);
            let prog = md.progs[rng.idx(md.progs.len())];
            let (chords, roots) = voice_chords(&mut rng, tonic, prog);
            for v in chords {
                let mut v = v.clone();
                v.sort();
                assert!(v.windows(2).all(|p| p[1] - p[0] >= 3), "{m}: {v:?}");
                assert!(*v.last().unwrap() <= 18, "{m}: top note too high {v:?}");
            }
            assert!(roots.iter().all(|r| (-33..=-22).contains(r)));
        }
    }
}

#[test]
fn sentences_read_naturally() {
    let r = Recipe::new("Deep House", "Hypnotic", "Liquid", 1).unwrap();
    assert_eq!(r.sentence, "A hypnotic Deep House track that flows like liquid.");
    let r = Recipe::new("UK Garage", "Euphoric", "Pulsing", 1).unwrap();
    assert_eq!(r.sentence, "A euphoric UK Garage track that pulses and breathes.");
}
