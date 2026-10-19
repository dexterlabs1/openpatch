use super::*;
use crate::control::Session;

fn empty_session() -> Session {
    let mut s = Session::new(&std::env::temp_dir()).unwrap();
    s.execute(
        "replace_patch",
        json!({"patch": {"version": 2, "sample_rate": 48000, "nodes": [], "cables": []}}),
    )
    .unwrap();
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
    steps
        .iter()
        .flat_map(|s| s["edits"].as_array().unwrap().clone())
        .collect()
}

#[test]
fn every_word_combination_builds_in_every_mode() {
    // Each recipe owns its Session. Bound independent genre workers so the
    // exhaustive coverage stays the same without serializing every graph on CI.
    let workers = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(4);
    std::thread::scope(|scope| {
        let handles: Vec<_> = GENRES
            .chunks(GENRES.len().div_ceil(workers))
            .map(|genres| {
                scope.spawn(move || {
                    let mut fails = vec![];
                    for &g in genres {
                        for m in MOODS {
                            for t in TEXTURES {
                                for seed in [1u64, 7, 4242] {
                                    let r = Recipe::new(g, m, t, seed).unwrap();
                                    let tag = format!("{g}|{m}|{t}|{seed}");
                                    // plan caps used by start_construction
                                    let (mods, steps) = r.reveal();
                                    let show = r.showtime();
                                    for (name, plan) in [("reveal", &steps), ("showtime", &show)] {
                                        assert!(
                                            plan.len() <= 64,
                                            "{tag}: {name} has {} steps",
                                            plan.len()
                                        );
                                        for s in plan.iter() {
                                            assert!(
                                                s["edits"].as_array().unwrap().len() <= 8,
                                                "{tag}: {name} step over 8 edits"
                                            );
                                        }
                                        assert!(
                                            flatten(plan).len() <= 256,
                                            "{tag}: {name} over 256 edits"
                                        );
                                    }
                                    for (name, edits) in [
                                        ("instant", r.edits()),
                                        ("reveal", [mods.clone(), flatten(&steps)].concat()),
                                        ("showtime", flatten(&show)),
                                    ] {
                                        if let Err(e) = apply_all(&edits) {
                                            fails.push(format!("{tag} {name}: {e}"));
                                        }
                                    }
                                }
                            }
                        }
                        assert!(
                            fails.is_empty(),
                            "{} failures, first: {:?}",
                            fails.len(),
                            &fails[..fails.len().min(5)]
                        );
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("keyword combination worker failed");
        }
    });
}

#[test]
fn final_graph_matches_the_offline_patch() {
    let r = Recipe::new("Deep House", "Dreamy", "Glitchy", 3).unwrap();
    let s = apply_all(&r.edits()).unwrap();
    let live = serde_json::to_value(s.patch()).unwrap();
    let want = r.patch();
    assert_eq!(
        live["nodes"].as_array().unwrap().len(),
        want["nodes"].as_array().unwrap().len()
    );
    assert_eq!(
        live["cables"].as_array().unwrap().len(),
        want["cables"].as_array().unwrap().len()
    );
    let route = |c: &Value| {
        (
            c["from"].as_str().unwrap_or("").to_string(),
            c["to"].as_str().unwrap_or("").to_string(),
            c["input"].as_str().unwrap_or("").to_string(),
        )
    };
    let mut a: Vec<_> = live["cables"]
        .as_array()
        .unwrap()
        .iter()
        .map(route)
        .collect();
    let mut b: Vec<_> = want["cables"]
        .as_array()
        .unwrap()
        .iter()
        .map(route)
        .collect();
    a.sort();
    b.sort();
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
        seen.insert(
            serde_json::to_string(
                &Recipe::new("Techno", "Hypnotic", "Liquid", seed)
                    .unwrap()
                    .patch(),
            )
            .unwrap(),
        );
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
    assert_eq!(
        r.sentence,
        "A hypnotic Deep House track that flows like liquid."
    );
    let r = Recipe::new("UK Garage", "Euphoric", "Pulsing", 1).unwrap();
    assert_eq!(
        r.sentence,
        "A euphoric UK Garage track that pulses and breathes."
    );
}

#[test]
fn minecraft_mode_builds_quietly_and_in_key() {
    for m in MOODS {
        for t in TEXTURES {
            let r = Recipe::new("Minecraft", m, t, 9).unwrap();
            assert!(r.sentence.contains("Minecraft-style piece"));
            apply_all(&r.edits()).unwrap_or_else(|e| panic!("Minecraft|{m}|{t}: {e}"));
            let p = r.patch();
            let drums = p["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["id"] == "seq_drums")
                .unwrap();
            assert!(
                drums["score"]["tracks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|tr| tr["steps"].as_array().unwrap().is_empty()),
                "no drums"
            );
        }
    }
}

/// Every kind, port and parameter a build emits exists in the module catalog, and every parameter value is
/// inside its catalog range. Sampled across words and seeds until every instrument design has been built.
#[test]
fn builds_use_catalog_kinds_ports_and_parameter_ranges() {
    let catalog = openpatch_core::catalog();
    let spec: HashMap<&str, &Value> = catalog["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["kind"].as_str().unwrap(), m))
        .collect();
    let names = |m: &Value, key: &str| -> Vec<String> {
        m[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| p.as_str().or(p["name"].as_str()).unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_else(|| vec!["out".into()])
    };
    let mut built: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut fails = vec![];
    let mut check = |r: &Recipe, tag: &str| {
        let p = r.patch();
        let mut kinds = HashMap::new();
        for n in p["nodes"].as_array().unwrap() {
            let kind = n["kind"].as_str().unwrap();
            kinds.insert(n["id"].as_str().unwrap().to_string(), kind.to_string());
            let Some(m) = spec.get(kind) else {
                fails.push(format!("{tag}: unknown kind {kind}"));
                continue;
            };
            for (k, v) in n["params"].as_object().unwrap() {
                let Some(range) = m["params"].get(k) else {
                    fails.push(format!("{tag}: {kind} has no parameter {k}"));
                    continue;
                };
                let x = v.as_f64().unwrap();
                let (lo, hi) = (range["min"].as_f64(), range["max"].as_f64());
                if lo.is_some_and(|lo| x < lo - 1e-6) || hi.is_some_and(|hi| x > hi + 1e-6) {
                    fails.push(format!("{tag}: {kind}.{k} = {x} outside {lo:?}..{hi:?}"));
                }
            }
        }
        for cab in p["cables"].as_array().unwrap() {
            let (f, o, t, i) = (
                cab["from"].as_str().unwrap(),
                cab["output"].as_str().unwrap(),
                cab["to"].as_str().unwrap(),
                cab["input"].as_str().unwrap(),
            );
            if let Some(m) = kinds.get(f).and_then(|k| spec.get(k.as_str())) {
                if !names(m, "outputs").iter().any(|x| x == o) {
                    fails.push(format!("{tag}: {} has no output {o}", kinds[f]));
                }
            }
            if let Some(m) = kinds.get(t).and_then(|k| spec.get(k.as_str())) {
                if !names(m, "inputs").iter().any(|x| x == i) {
                    fails.push(format!("{tag}: {} has no input {i}", kinds[t]));
                }
            }
        }
        for v in r.kinds.values() {
            built.insert(v.clone());
        }
    };
    for (n, g) in GENRES.iter().enumerate() {
        for (k, m) in MOODS.iter().enumerate() {
            for seed in 1..=6u64 {
                let t = TEXTURES[(n + k + seed as usize) % TEXTURES.len()];
                let tag = format!("{g}|{m}|{t}|{seed}");
                check(&Recipe::new(g, m, t, seed).unwrap(), &tag);
                check(
                    &Recipe::with_budget(g, m, t, seed, Some(18.0)).unwrap(),
                    &format!("{tag} light"),
                );
            }
        }
    }
    // rarely chosen designs: pin each one into builds until it appears (a lead needs a build that has a melody)
    for d in instruments::DESIGNS {
        PINNED.with(|p| *p.borrow_mut() = vec![d.name]);
        for seed in 1..=40u64 {
            let r = Recipe::new("Deep House", "Dreamy", "Liquid", seed).unwrap();
            let hit = r.kinds.values().any(|v| v == d.name);
            check(&r, &format!("pinned {} {seed}", d.name));
            if hit {
                break;
            }
        }
        PINNED.with(|p| p.borrow_mut().clear());
    }
    assert!(
        fails.is_empty(),
        "{} failures: {:?}",
        fails.len(),
        &fails[..fails.len().min(8)]
    );
    let missing: Vec<&str> = instruments::DESIGNS
        .iter()
        .map(|d| d.name)
        .filter(|n| !built.contains(*n) && *n != "piano")
        .collect();
    assert!(
        missing.is_empty(),
        "designs never built by the sample: {missing:?}"
    );
}

#[test]
fn light_builds_fit_their_budget_and_every_plan_fits_construction() {
    for g in GENRES {
        for m in MOODS {
            for (i, t) in TEXTURES.iter().enumerate() {
                let seed = 100 + i as u64;
                let full = Recipe::new(g, m, t, seed).unwrap();
                assert!(
                    full.ops.len() <= MAX_PLAN_EDITS,
                    "{g}|{m}|{t}: {} edits",
                    full.ops.len()
                );
                let light = Recipe::with_budget(g, m, t, seed, Some(18.0)).unwrap();
                assert!(light.ops.len() <= MAX_PLAN_EDITS);
                assert!(
                    light.cpu_estimate() <= 18.0
                        || light.notes.iter().any(|n| n.ends_with("open hat")),
                    "{g}|{m}|{t}: light build {}",
                    light.cpu_estimate()
                );
            }
        }
    }
}

#[test]
fn controls_shape_builds_and_the_centre_reproduces_every_seed() {
    let centre = Controls::default();
    for g in GENRES {
        let a = Recipe::new(g, "Dark", "Liquid", 3).unwrap();
        let b = Recipe::with_controls(g, "Dark", "Liquid", 3, None, centre).unwrap();
        assert_eq!(
            a.patch(),
            b.patch(),
            "{g}: centre controls must reproduce the seed"
        );
        for (lo, len) in [(0.0, 32), (1.0, 128)] {
            let ctl = Controls {
                energy: lo,
                complexity: lo,
                weirdness: lo,
                space: lo,
                length: len,
            };
            let r = Recipe::with_controls(g, "Dark", "Liquid", 3, Some(18.0), ctl).unwrap();
            assert!(r.ops.len() <= MAX_PLAN_EDITS, "{g} {len}: plan too large");
            for score in r.scores.values() {
                for lane in score["lanes"].as_array().into_iter().flatten() {
                    assert_eq!(lane["length_bars"], json!(len));
                    for p in lane["points"].as_array().unwrap() {
                        assert!(
                            p["bar"].as_f64().unwrap() <= len as f64,
                            "{g}: lane point past the form"
                        );
                    }
                }
            }
            assert!(
                r.drops.iter().all(|&d| d < len as f64),
                "{g}: drops {:?}",
                r.drops
            );
        }
    }
    let bad = Controls {
        length: 48,
        ..centre
    };
    assert!(Recipe::with_controls("Techno", "Dark", "Liquid", 1, None, bad).is_err());
}
