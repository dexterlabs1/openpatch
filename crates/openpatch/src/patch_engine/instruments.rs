//! Instrument designs: each voice role (chords, pad layer, bass, lead) chooses among several sound designs drawn
//! from the whole catalog — macro-synthesis engines, the modal resonator, wavetables, the Fundamental 2.6.4 VCO/VCF
//! and the transistor ladder — next to the original subtractive voices. Designs carry character tags; the words
//! (genre · mood · texture) weight the tags, the seed picks. Expensive designs name a cheaper stand-in that light
//! (CPU-budgeted) builds use instead.
use super::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Role {
    Chords,
    Bass,
    Lead,
}

pub(super) struct Design {
    pub name: &'static str,
    pub role: Role,
    /// Phrase for the recipe description ("glassy FM electric piano").
    pub words: &'static str,
    pub tags: &'static [&'static str],
    /// Holds a note for as long as the gate (pads, organs, sustained leads). Percussive designs decay on their own.
    pub sustain: bool,
    /// Cheaper stand-in for light builds; `None` when the design is already inexpensive.
    pub light: Option<&'static str>,
}

const fn d(
    name: &'static str,
    role: Role,
    words: &'static str,
    tags: &'static [&'static str],
    sustain: bool,
    light: Option<&'static str>,
) -> Design {
    Design {
        name,
        role,
        words,
        tags,
        sustain,
        light,
    }
}

use Role::*;
pub(super) const DESIGNS: &[Design] = &[
    // chords · original subtractive voices
    d(
        "keys",
        Chords,
        "soft electric-piano chords (triangle + sine)",
        &["warm", "keys", "soft", "retro"],
        false,
        None,
    ),
    d(
        "piano",
        Chords,
        "soft piano broken chords",
        &["keys", "soft", "organic"],
        false,
        None,
    ),
    d(
        "pluck",
        Chords,
        "plucked saw chords",
        &["pluck", "bright", "digital"],
        false,
        None,
    ),
    d(
        "organ",
        Chords,
        "organ chords (stacked sines)",
        &["organ", "warm", "retro"],
        true,
        None,
    ),
    d(
        "pad_saw",
        Chords,
        "a detuned saw pad",
        &["pad", "huge", "analog"],
        true,
        None,
    ),
    d(
        "pad_square",
        Chords,
        "a soft square pad",
        &["pad", "soft", "hollow"],
        true,
        None,
    ),
    d(
        "stab",
        Chords,
        "dub chord stabs (saw + square)",
        &["stab", "dub", "dark"],
        false,
        None,
    ),
    // chords · catalog designs
    d(
        "fm_keys",
        Chords,
        "glassy FM electric-piano chords (macro voice · FM, low-pass gate)",
        &["keys", "glassy", "digital", "retro", "bright"],
        false,
        Some("keys"),
    ),
    d(
        "swarm_pad",
        Chords,
        "a supersaw swarm pad (macro voice · swarm), timbre on the sweep",
        &["pad", "huge", "lush", "bright", "prog"],
        true,
        Some("pad_saw"),
    ),
    d(
        "harmonic_organ",
        Chords,
        "additive drawbar chords (macro voice · harmonic)",
        &["organ", "warm", "hollow", "retro", "lush"],
        true,
        None,
    ),
    d(
        "fold_stab",
        Chords,
        "wavefolded chord stabs (macro voice · wavefold, low-pass gate)",
        &["stab", "gritty", "digital", "dark", "weird"],
        false,
        Some("stab"),
    ),
    d(
        "wt_pad",
        Chords,
        "an evolving wavetable pad, table position drifting on the motion LFO",
        &["pad", "evolving", "digital", "glassy", "lush"],
        true,
        None,
    ),
    d(
        "glass",
        Chords,
        "struck glass chords (modal resonator · stiff string)",
        &["glassy", "organic", "metallic", "keys", "pluck"],
        false,
        None,
    ),
    d(
        "analog_pluck",
        Chords,
        "analog plucks (Fundamental 2.6.4 VCO → ladder VCF with envelope sweep)",
        &["pluck", "analog", "bright", "prog", "warm"],
        false,
        None,
    ),
    // bass
    d(
        "ladder_saw",
        Bass,
        "saw through a four-pole low-pass (envelope + sweep) over a sine sub",
        &["warm", "analog"],
        false,
        None,
    ),
    d(
        "square",
        Bass,
        "square through a four-pole low-pass over a sine sub",
        &["hollow", "retro"],
        false,
        None,
    ),
    d(
        "round",
        Bass,
        "round triangle bass over a sine sub",
        &["soft", "warm", "organic"],
        false,
        None,
    ),
    d(
        "pwm",
        Bass,
        "slowly pulse-width-modulated square, low-passed",
        &["evolving", "retro", "hollow"],
        false,
        None,
    ),
    d(
        "sub",
        Bass,
        "pure sine sub",
        &["soft", "dub", "dark"],
        false,
        None,
    ),
    d(
        "analog_bass",
        Bass,
        "Fundamental 2.6.4 saw through its ladder VCF (envelope + sweep) over a sine sub",
        &["analog", "warm", "prog", "bright"],
        false,
        Some("ladder_saw"),
    ),
    d(
        "moog_bass",
        Bass,
        "capacitor-ramp saw through the circuit-modelled transistor ladder over a sine sub",
        &["analog", "warm", "huge", "gritty"],
        false,
        Some("ladder_saw"),
    ),
    d(
        "fm_bass",
        Bass,
        "punchy FM bass (macro voice · FM, low-pass gate) over a sine sub",
        &["digital", "dark", "metallic", "weird", "retro"],
        false,
        Some("square"),
    ),
    d(
        "fold_bass",
        Bass,
        "wavefolded growl bass (macro voice · wavefold) over a sine sub",
        &["gritty", "dark", "digital", "weird"],
        false,
        Some("square"),
    ),
    // lead
    d(
        "classic",
        Lead,
        "a subtractive oscillator voice",
        &["soft"],
        true,
        None,
    ),
    d(
        "fm_bell",
        Lead,
        "FM bell tones (macro voice · FM, low-pass gate)",
        &["glassy", "metallic", "digital", "bright", "retro"],
        false,
        Some("classic"),
    ),
    d(
        "modal_bell",
        Lead,
        "struck modal bells (modal resonator)",
        &["glassy", "organic", "metallic"],
        false,
        None,
    ),
    d(
        "fold_lead",
        Lead,
        "a wavefolded lead (macro voice · wavefold, low-pass gate)",
        &["gritty", "digital", "weird", "bright"],
        false,
        Some("classic"),
    ),
    d(
        "swarm_lead",
        Lead,
        "a supersaw swarm lead (macro voice · swarm)",
        &["huge", "bright", "prog", "lush"],
        true,
        Some("classic"),
    ),
    d(
        "harmonic_lead",
        Lead,
        "an additive flute-organ lead (macro voice · harmonic)",
        &["hollow", "soft", "organic", "warm"],
        true,
        None,
    ),
    d(
        "analog_lead",
        Lead,
        "an analog saw lead (Fundamental 2.6.4 VCO → ladder VCF)",
        &["analog", "warm", "retro", "prog"],
        true,
        Some("classic"),
    ),
    d(
        "wt_lead",
        Lead,
        "a wavetable lead, table position drifting on the motion LFO",
        &["evolving", "digital", "glassy"],
        true,
        None,
    ),
];

pub(super) fn design(name: &str) -> Option<&'static Design> {
    DESIGNS.iter().find(|x| x.name == name)
}

/// The cheaper stand-in a light build uses for `name` (itself when none is needed).
pub(super) fn light_of(name: &'static str) -> &'static str {
    design(name).and_then(|x| x.light).unwrap_or(name)
}

fn genre_tags(genre: &str) -> &'static [(&'static str, f64)] {
    match genre {
        "Deep House" => &[
            ("warm", 2.0),
            ("keys", 2.0),
            ("organ", 1.5),
            ("soft", 1.0),
            ("lush", 1.0),
        ],
        "Dub Techno" => &[
            ("dub", 3.0),
            ("stab", 2.0),
            ("dark", 1.5),
            ("hollow", 1.0),
            ("metallic", 0.8),
        ],
        "Techno" => &[
            ("dark", 2.0),
            ("metallic", 1.5),
            ("stab", 1.5),
            ("gritty", 1.2),
            ("digital", 1.0),
        ],
        "Ambient" => &[
            ("pad", 2.0),
            ("glassy", 2.0),
            ("evolving", 2.0),
            ("lush", 1.5),
            ("organic", 1.0),
        ],
        "Lo-fi Hip Hop" => &[
            ("warm", 2.0),
            ("keys", 2.0),
            ("soft", 2.0),
            ("retro", 1.0),
            ("organic", 0.8),
        ],
        "Synthwave" => &[
            ("analog", 2.0),
            ("retro", 2.0),
            ("huge", 1.5),
            ("bright", 1.0),
            ("prog", 0.8),
        ],
        "Downtempo" => &[
            ("organic", 2.0),
            ("glassy", 1.5),
            ("warm", 1.5),
            ("keys", 1.0),
            ("evolving", 0.8),
        ],
        _ => &[
            ("organ", 2.0),
            ("digital", 1.5),
            ("stab", 1.5),
            ("bright", 1.0),
            ("pluck", 0.8),
        ],
    }
}
fn mood_tags(mood: &str) -> &'static [(&'static str, f64)] {
    match mood {
        "Dreamy" => &[
            ("glassy", 1.5),
            ("lush", 1.5),
            ("soft", 1.0),
            ("evolving", 0.8),
        ],
        "Dark" => &[("dark", 1.5), ("gritty", 1.2), ("metallic", 1.0)],
        "Euphoric" => &[("bright", 1.5), ("huge", 1.5), ("prog", 1.2), ("lush", 0.8)],
        "Melancholy" => &[
            ("soft", 1.2),
            ("hollow", 1.2),
            ("keys", 1.0),
            ("organic", 0.8),
        ],
        "Hypnotic" => &[
            ("evolving", 1.5),
            ("metallic", 1.0),
            ("dub", 1.0),
            ("pluck", 0.8),
        ],
        "Warm" => &[("warm", 1.5), ("analog", 1.2), ("organ", 0.8)],
        "Mysterious" => &[
            ("metallic", 1.2),
            ("glassy", 1.2),
            ("weird", 1.2),
            ("hollow", 1.0),
        ],
        _ => &[
            ("pluck", 1.5),
            ("bright", 1.2),
            ("digital", 1.0),
            ("retro", 1.0),
        ],
    }
}
fn texture_tags(texture: &str) -> &'static [(&'static str, f64)] {
    match texture {
        "Glitchy" => &[("digital", 1.5), ("weird", 1.5), ("gritty", 1.0)],
        "Dusty" => &[("warm", 1.5), ("retro", 1.2), ("soft", 1.0)],
        "Spacious" => &[("lush", 1.5), ("glassy", 1.2), ("pad", 1.0)],
        "Crisp" => &[("bright", 1.5), ("pluck", 1.2), ("digital", 0.8)],
        "Liquid" => &[("evolving", 1.5), ("glassy", 1.2), ("lush", 0.8)],
        "Minimal" => &[("hollow", 1.2), ("dub", 1.2), ("soft", 0.8)],
        "Echoing" => &[("pluck", 1.2), ("dub", 1.2), ("glassy", 1.0)],
        _ => &[("pad", 1.2), ("huge", 1.2), ("stab", 1.0)],
    }
}

/// Words-weighted affinity of a design (sum of matching tag weights).
pub(super) fn affinity(x: &Design, genre: &str, mood: &str, texture: &str) -> f64 {
    let sum = |tags: &[(&str, f64)]| -> f64 {
        tags.iter()
            .filter(|(t, _)| x.tags.contains(t))
            .map(|(_, w)| w)
            .sum()
    };
    sum(genre_tags(genre)) + 0.7 * sum(mood_tags(mood)) + 0.5 * sum(texture_tags(texture))
}

/// How often a design is chosen, from one-variable listening tests (2026-10-19: each design swapped into a patch
/// the listener liked). Great designs lead; ones the listener passed over stay possible but rare (and remain the
/// light-mode stand-ins); struck glass works only sparingly.
fn rating(name: &str) -> f64 {
    match name {
        "pluck" | "stab" | "swarm_pad" | "harmonic_lead" | "fold_bass" => 2.0,
        "fm_bass" | "analog_pluck" | "fm_keys" => 1.3,
        "organ" | "pad_saw" | "pad_square" | "piano" | "wt_pad" | "analog_lead" | "classic"
        | "fold_lead" => 1.0,
        "harmonic_organ" | "wt_lead" => 0.6,
        "glass" => 0.3,
        _ => 0.01,
    }
}

/// Seeded, words-weighted choice among the designs of `role` that pass `fit`. Designs in `native` (the genre's own
/// classic palette) get a bonus so a genre keeps its identity; designs in `avoid` (already used in this patch) are
/// strongly discouraged so layers contrast.
#[allow(clippy::too_many_arguments)]
pub(super) fn choose(
    rng: &mut Rng,
    role: Role,
    genre: &str,
    mood: &str,
    texture: &str,
    native: &[&str],
    avoid: &[&str],
    fit: impl Fn(&Design) -> bool,
) -> &'static str {
    let pool: Vec<(&'static str, f64)> = DESIGNS
        .iter()
        .filter(|x| x.role == role && fit(x))
        .map(|x| {
            let a = affinity(x, genre, mood, texture)
                + if native.contains(&x.name) { 1.5 } else { 0.0 };
            let w = (0.5 + a).powi(2)
                * rating(x.name)
                * if avoid.contains(&x.name) { 0.1 } else { 1.0 };
            (x.name, w)
        })
        .collect();
    rng.weighted(&pool)
}

const C4: f64 = 261.6256;

/// `harmonics` value that selects a macro-voice FM ratio. The engine reads `RATIOS[round(h × 15)]`; only whole-number
/// ratios (and ½ for basses) keep every sideband on the harmonic series, so designs pick ratios, never raw values.
fn fm_ratio(ratio: f64) -> f64 {
    const RATIOS: [f64; 16] = [
        0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 5.0, 6.0, 8.0, 10.0, 12.0, 16.0,
    ];
    let i = RATIOS
        .iter()
        .position(|r| *r == ratio)
        .expect("macro-voice FM ratio");
    r3(i as f64 / 15.0)
}

impl Recipe {
    /// Catalog chord designs (poly, on the 4-voice chord pitch bus). Returns the poly output before summing, or
    /// `None` for the original subtractive voices that `chord_voice` builds itself.
    pub(super) fn chord_design(
        &mut self,
        c: &mut Ctx,
        id: &str,
        kind: &str,
        pitch: &(String, String),
        gate: (&str, &str),
        fcv: &(String, String),
    ) -> Option<(String, String)> {
        let row = 4;
        let b = c.bright;
        let v = format!("{id}_voice");
        let env = format!("{id}_env");
        let pad_env = |c: &mut Ctx| json!({"attack": r3(c.rng.uni(0.3, 1.0)), "decay": 0.8, "sustain": 0.85, "release": r3(c.rng.uni(1.0, 2.0))});
        match kind {
            "fm_keys" | "fold_stab" => {
                // one macro voice per chord note; its own low-pass gate is the envelope
                let (engine, label, p) = match kind {
                    "fm_keys" => (
                        2,
                        "FM keys",
                        json!({"harmonics": fm_ratio(c.rng.pick(&[1.0, 1.0, 2.0, 3.0, 4.0])), "timbre": r3((0.24 + 0.06 * b + c.rng.uni(-0.04, 0.05)).clamp(0.15, 0.35)),
                                                       "morph": r3(c.rng.uni(0.15, 0.35)), "decay": r3(c.rng.uni(0.55, 0.75))}),
                    ),
                    "fold_stab" => (
                        1,
                        "Wavefold stabs",
                        json!({"harmonics": r3(c.rng.uni(0.05, 0.2)), "timbre": r3((0.07 + 0.04 * b + c.rng.uni(-0.02, 0.03)).clamp(0.02, 0.12)),
                                                              "morph": r3(c.rng.uni(0.0, 0.2)), "decay": r3(c.rng.uni(0.35, 0.5))}),
                    ),
                    _ => unreachable!(),
                };
                let mut p = p;
                p["engine"] = json!(engine);
                p["frequency"] = json!(C4);
                p["level"] = json!(0.8);
                self.module(
                    &v,
                    "macro_voice",
                    row,
                    p,
                    format!("{label} (macro voice ×4)"),
                );
                self.con(&pitch.0, &pitch.1, &v, "pitch");
                self.con(gate.0, gate.1, &v, "trigger");
                // FM index follows the sweep; the folder's timbre is too sensitive, so the sweep tilts its symmetry
                self.con(
                    &fcv.0,
                    &fcv.1,
                    &v,
                    if kind == "fm_keys" { "timbre" } else { "morph" },
                );
                Some((v, "out".into()))
            }
            "swarm_pad" | "harmonic_organ" => {
                let swarm = kind == "swarm_pad";
                let p = if swarm {
                    json!({"engine": 6, "frequency": C4, "harmonics": r3(c.rng.uni(0.3, 0.55)), "timbre": r3((0.18 + 0.06 * b).clamp(0.12, 0.25)), "morph": 0.5, "level": 0.8})
                } else {
                    json!({"engine": 3, "frequency": C4, "harmonics": r3(c.rng.uni(0.05, 0.3)), "timbre": r3((0.06 + 0.04 * b).clamp(0.02, 0.12)), "morph": r3(c.rng.uni(0.25, 0.45)), "level": 0.8})
                };
                self.module(
                    &v,
                    "macro_voice",
                    row,
                    p,
                    if swarm {
                        "Swarm (macro voice ×4)"
                    } else {
                        "Harmonic drawbars (macro voice ×4)"
                    },
                );
                self.con(&pitch.0, &pitch.1, &v, "pitch");
                let e = if swarm {
                    pad_env(c)
                } else {
                    json!({"attack": 0.01, "decay": 0.3, "sustain": 1.0, "release": 0.15})
                };
                self.module(&env, "adsr", row, e, "Envelope → level");
                self.con(gate.0, gate.1, &env, "gate");
                self.con(&env, "out", &v, "level");
                // the sweep moves the spectral slope; timbre (centroid) would push these up into whistle range
                self.con(&fcv.0, &fcv.1, &v, "morph");
                if !swarm {
                    return Some((v, "out".into()));
                }
                // the swarm's energy sits high above the fundamental: a four-pole low-pass empties it, so only a
                // gentle one-pole takes the fizz off
                let lp = format!("{id}_lp");
                self.module(
                    &lp,
                    "lowpass",
                    row,
                    json!({"cutoff": Self::cut(b, 3200.0 * c.rng.uni(0.8, 1.25), 0.5)}),
                    "Low-pass (one-pole)",
                );
                self.con(&v, "out", &lp, "signal");
                Some((lp, "out".into()))
            }
            "wt_pad" => {
                self.module(&v, "wavetable_vco", row, json!({"tune": 0.0, "position": r3(c.rng.uni(0.1, 0.6)), "position_cv_amount": r3(c.rng.uni(0.25, 0.5))}), "Wavetable ×4");
                self.con(&pitch.0, &pitch.1, &v, "pitch");
                self.con("motion", "triangle", &v, "position_cv");
                let lp = format!("{id}_lp");
                self.module(
                    &lp,
                    "lowpass",
                    row,
                    json!({"cutoff": Self::cut(b, 1800.0 * c.rng.uni(0.8, 1.25), 0.6)}),
                    "Low-pass (one-pole)",
                );
                self.con(&v, "out", &lp, "signal");
                let e = pad_env(c);
                self.module(&env, "adsr", row, e, "Envelope");
                self.con(gate.0, gate.1, &env, "gate");
                let vca = format!("{id}_vca");
                self.module(&vca, "stock_vca", row, json!({"level": 1.0}), "VCA");
                self.con(&lp, "out", &vca, "signal");
                self.con(&env, "out", &vca, "cv");
                Some((vca, "out".into()))
            }
            "glass" => {
                self.module(&v, "modal_resonator", row, json!({"frequency": C4, "stiffness": 0.0, "decay": r3(c.rng.uni(1.5, 4.0)),
                    "damping": r3(c.rng.uni(0.2, 0.5) - 0.1 * b), "strike_position": r3(c.rng.uni(0.1, 0.3)), "pickup_position": r3(c.rng.uni(0.5, 0.7)), "strike_strength": 1.0, "level": 1.0}),
                    "Struck glass (modal resonator ×4)");
                self.con(&pitch.0, &pitch.1, &v, "pitch");
                self.con(gate.0, gate.1, &v, "strike");
                Some((v, "out".into()))
            }
            "analog_pluck" => {
                let osc = format!("{id}_osc");
                self.module(
                    &osc,
                    "rack264_vco",
                    row,
                    json!({"frequency": 0.0}),
                    "VCO 2.6.4 ×4 (saw)",
                );
                self.con(&pitch.0, &pitch.1, &osc, "pitch");
                self.module(&env, "adsr", row, json!({"attack": 0.002, "decay": r3(c.rng.uni(0.2, 0.5)), "sustain": 0.0, "release": 0.25}), "Envelope");
                self.con(gate.0, gate.1, &env, "gate");
                let fm = format!("{id}_fcv");
                self.module(
                    &fm,
                    "cv_mix",
                    row,
                    json!({"level1": 1.0, "level2": r3(c.rng.uni(0.2, 0.4)), "level3": 0.0}),
                    "Filter CV: sweep + envelope",
                );
                self.con(&fcv.0, &fcv.1, &fm, "in1");
                self.con(&env, "out", &fm, "in2");
                let vcf = format!("{id}_vcf");
                let cutoff = Self::cut(b, 650.0 * c.rng.uni(0.8, 1.25), 0.6);
                self.module(&vcf, "rack264_vcf", row, json!({"frequency": vcf_knob(cutoff).min(0.99), "resonance": r3(c.rng.uni(0.15, 0.35)), "cutoff_cv_amount": 1.0}), "Ladder VCF 2.6.4");
                self.con(&osc, "saw", &vcf, "signal");
                self.con(&fm, "out", &vcf, "cutoff_cv");
                let vca = format!("{id}_vca");
                self.module(&vca, "stock_vca", row, json!({"level": 1.0}), "VCA");
                self.con(&vcf, "lowpass", &vca, "signal");
                self.con(&env, "out", &vca, "cv");
                Some((vca, "out".into()))
            }
            _ => None,
        }
    }

    /// Catalog bass designs. The sine sub (`bass_osc`), envelope (`bass_env`) and envelope × lane VCA (`bass_gate`)
    /// already exist. Returns the main bass signal, or `None` for the original multi-VCO voices.
    pub(super) fn bass_design(
        &mut self,
        c: &mut Ctx,
        kind: &str,
        pitch: &(String, String),
    ) -> Option<(String, String)> {
        let row = 3;
        let b = c.bright;
        let bold = matches!(c.genre, "Techno" | "UK Garage" | "Synthwave" | "Dub Techno");
        match kind {
            "fm_bass" | "fold_bass" => {
                let fm = kind == "fm_bass";
                let p = if fm {
                    json!({"engine": 2, "frequency": C4, "harmonics": fm_ratio(c.rng.pick(&[0.5, 1.0, 1.0, 2.0])), "timbre": r3((0.22 + 0.06 * b + c.rng.uni(-0.04, 0.05)).clamp(0.12, 0.32)),
                           "morph": r3(c.rng.uni(0.05, 0.3)), "decay": r3(c.rng.uni(0.35, 0.55)), "level": 0.9})
                } else {
                    json!({"engine": 1, "frequency": C4, "harmonics": r3(c.rng.uni(0.05, 0.25)), "timbre": r3((0.08 + 0.04 * b).clamp(0.03, 0.14)),
                           "morph": r3(c.rng.uni(0.1, 0.4)), "decay": r3(c.rng.uni(0.4, 0.6)), "level": 0.9})
                };
                self.module(
                    "bass_voice",
                    "macro_voice",
                    row,
                    p,
                    if fm {
                        "FM bass (macro voice)"
                    } else {
                        "Wavefold bass (macro voice)"
                    },
                );
                self.con(&pitch.0, &pitch.1, "bass_voice", "pitch");
                self.con("seq_music", "gate2", "bass_voice", "trigger");
                let bl = self.lane_out("bass");
                self.con("lanes", &bl, "bass_voice", "level");
                let cl = self.lane_out("cut");
                self.con(
                    "lanes",
                    &cl,
                    "bass_voice",
                    if fm { "timbre" } else { "morph" },
                );
                Some(("bass_voice".into(), "out".into()))
            }
            "analog_bass" | "moog_bass" => {
                let analog = kind == "analog_bass";
                let mut wave = "out";
                if analog {
                    wave = c.rng.pick(&["saw", "saw", "square"]);
                    self.module(
                        "bass_voice",
                        "rack264_vco",
                        row,
                        json!({"frequency": 0.0, "pulse_width": 0.45}),
                        format!("VCO 2.6.4 ({wave})"),
                    );
                    self.con(&pitch.0, &pitch.1, "bass_voice", "pitch");
                } else {
                    self.module(
                        "bass_voice",
                        "analog_vco",
                        row,
                        json!({"frequency": C4, "waveform": 0}),
                        "Capacitor-ramp saw",
                    );
                    self.con(&pitch.0, &pitch.1, "bass_voice", "pitch");
                }
                self.module(
                    "bass_fcv",
                    "cv_mix",
                    row,
                    json!({"level1": 0.5, "level2": r3(c.rng.uni(0.15, 0.26)), "level3": 0.0}),
                    "Filter CV: sweep + envelope",
                );
                let cl = self.lane_out("cut");
                self.con("lanes", &cl, "bass_fcv", "in1");
                self.con("bass_env", "out", "bass_fcv", "in2");
                let cutoff = Self::cut(b, 260.0 * c.rng.uni(0.8, 1.25), 0.5);
                let (res, drive) = if bold {
                    (c.rng.uni(0.25, 0.4), c.rng.uni(0.2, 0.4))
                } else {
                    (c.rng.uni(0.15, 0.3), 0.0)
                };
                let out = if analog {
                    self.module("bass_vcf", "rack264_vcf", row, json!({"frequency": vcf_knob(cutoff).min(0.99), "resonance": r3(res), "drive": r3(drive), "cutoff_cv_amount": 1.0}), "Ladder VCF 2.6.4");
                    self.con("bass_voice", wave, "bass_vcf", "signal");
                    "lowpass"
                } else {
                    // the circuit ladder's -12 dB pole sits lower than the four-pole knob: open it a little
                    self.module("bass_vcf", "ladder_vcf", row, json!({"cutoff": r3(cutoff * 1.4), "resonance": r3(res * 1.6), "drive": r3(1.0 + 2.0 * drive)}), "Transistor ladder (circuit model)");
                    self.con("bass_voice", wave, "bass_vcf", "signal");
                    "out"
                };
                self.con("bass_fcv", "out", "bass_vcf", "cutoff_cv");
                self.module(
                    "bass_vca",
                    "stock_vca",
                    row,
                    json!({"level": 1.0}),
                    "Bass VCA",
                );
                self.con("bass_vcf", out, "bass_vca", "signal");
                self.con("bass_gate", "out", "bass_vca", "cv");
                Some(("bass_vca".into(), "out".into()))
            }
            _ => None,
        }
    }

    /// Catalog lead designs on seq_music pitch3/gate3, opened by the lead lane. Returns the lead output, or `None`
    /// for the original oscillator voice built by `mono_voice`.
    pub(super) fn lead_design(
        &mut self,
        c: &mut Ctx,
        kind: &str,
        env: &Value,
        lane_port: &str,
    ) -> Option<(String, String)> {
        let row = 5;
        let b = c.bright;
        if design(kind).is_none() || kind == "classic" {
            return None;
        }
        let (pp, po) = self.key_src(c, "pitch3");
        // shared envelope × lane VCA for the sustained designs
        let gated = |r: &mut Recipe, env: &Value| {
            r.module("lead_env", "adsr", row, env.clone(), "Envelope");
            r.con("seq_music", "gate3", "lead_env", "gate");
            r.module(
                "lead_gate",
                "stock_vca",
                row,
                json!({"level": 1.0}),
                "Envelope × song-form lane",
            );
            r.con("lead_env", "out", "lead_gate", "signal");
            r.con("lanes", lane_port, "lead_gate", "cv");
        };
        match kind {
            "fm_bell" | "fold_lead" => {
                let p = if kind == "fm_bell" {
                    json!({"engine": 2, "frequency": C4, "harmonics": fm_ratio(c.rng.pick(&[2.0, 3.0, 4.0, 5.0, 6.0])), "timbre": r3((0.25 + 0.06 * b + c.rng.uni(-0.03, 0.06)).clamp(0.15, 0.38)),
                           "morph": r3(c.rng.uni(0.1, 0.3)), "decay": r3(c.rng.uni(0.6, 0.8)), "level": 0.8})
                } else {
                    json!({"engine": 1, "frequency": C4, "harmonics": r3(c.rng.uni(0.05, 0.25)), "timbre": r3((0.08 + 0.05 * b).clamp(0.03, 0.15)),
                           "morph": r3(c.rng.uni(0.2, 0.5)), "decay": r3(c.rng.uni(0.45, 0.65)), "level": 0.8})
                };
                self.module(
                    "lead_voice",
                    "macro_voice",
                    row,
                    p,
                    if kind == "fm_bell" {
                        "FM bells (macro voice)"
                    } else {
                        "Wavefold lead (macro voice)"
                    },
                );
                self.con(&pp, &po, "lead_voice", "pitch");
                self.con("seq_music", "gate3", "lead_voice", "trigger");
                self.con("lanes", lane_port, "lead_voice", "level");
                self.con("motion", "sine", "lead_voice", "morph");
                Some(("lead_voice".into(), "out".into()))
            }
            "modal_bell" => {
                self.module("lead_voice", "modal_resonator", row, json!({"frequency": C4, "stiffness": 0.0, "decay": r3(c.rng.uni(2.0, 5.0)), "damping": r3(c.rng.uni(0.15, 0.4)),
                    "strike_position": r3(c.rng.uni(0.1, 0.25)), "pickup_position": r3(c.rng.uni(0.55, 0.75)), "strike_strength": 1.0, "level": 1.0}), "Modal bells");
                self.con(&pp, &po, "lead_voice", "pitch");
                self.con("seq_music", "gate3", "lead_voice", "strike");
                self.module(
                    "lead_vca",
                    "stock_vca",
                    row,
                    json!({"level": 1.0}),
                    "Song-form lane VCA",
                );
                self.con("lead_voice", "out", "lead_vca", "signal");
                self.con("lanes", lane_port, "lead_vca", "cv");
                Some(("lead_vca".into(), "out".into()))
            }
            "swarm_lead" | "harmonic_lead" => {
                let swarm = kind == "swarm_lead";
                let p = if swarm {
                    json!({"engine": 6, "frequency": C4, "harmonics": r3(c.rng.uni(0.25, 0.5)), "timbre": r3((0.2 + 0.06 * b).clamp(0.12, 0.28)), "morph": 0.5, "level": 0.8})
                } else {
                    json!({"engine": 3, "frequency": C4, "harmonics": r3(c.rng.uni(0.05, 0.3)), "timbre": r3((0.07 + 0.04 * b).clamp(0.02, 0.13)), "morph": r3(c.rng.uni(0.25, 0.45)), "level": 0.8})
                };
                self.module(
                    "lead_voice",
                    "macro_voice",
                    row,
                    p,
                    if swarm {
                        "Swarm lead (macro voice)"
                    } else {
                        "Harmonic lead (macro voice)"
                    },
                );
                self.con(&pp, &po, "lead_voice", "pitch");
                gated(self, env);
                self.con("lead_gate", "out", "lead_voice", "level");
                self.con("motion", "sine", "lead_voice", "morph");
                if !swarm {
                    return Some(("lead_voice".into(), "out".into()));
                }
                self.module(
                    "lead_vcf",
                    "lowpass",
                    row,
                    json!({"cutoff": Self::cut(b, 3500.0 * c.rng.uni(0.8, 1.2), 0.5)}),
                    "Low-pass (one-pole)",
                );
                self.con("lead_voice", "out", "lead_vcf", "signal");
                Some(("lead_vcf".into(), "out".into()))
            }
            "analog_lead" | "wt_lead" => {
                if kind == "analog_lead" {
                    self.module(
                        "lead_osc",
                        "rack264_vco",
                        row,
                        json!({"frequency": 0.0}),
                        "VCO 2.6.4 (saw)",
                    );
                    self.con(&pp, &po, "lead_osc", "pitch");
                    let cutoff = Self::cut(b, 1300.0 * c.rng.uni(0.8, 1.2), 0.6);
                    self.module("lead_vcf", "rack264_vcf", row, json!({"frequency": vcf_knob(cutoff).min(0.99), "resonance": r3(c.rng.uni(0.15, 0.35)), "cutoff_cv_amount": 1.0}), "Ladder VCF 2.6.4");
                    self.con("lead_osc", "saw", "lead_vcf", "signal");
                    self.con("motion", "sine", "lead_vcf", "cutoff_cv");
                } else {
                    self.module("lead_osc", "wavetable_vco", row, json!({"tune": 0.0, "position": r3(c.rng.uni(0.2, 0.8)), "position_cv_amount": r3(c.rng.uni(0.2, 0.45))}), "Wavetable");
                    self.con(&pp, &po, "lead_osc", "pitch");
                    self.con("motion", "sine", "lead_osc", "position_cv");
                    self.module(
                        "lead_vcf",
                        "lowpass",
                        row,
                        json!({"cutoff": Self::cut(b, 2400.0, 0.6)}),
                        "Low-pass (one-pole)",
                    );
                    self.con("lead_osc", "out", "lead_vcf", "signal");
                }
                gated(self, env);
                self.module("lead_vca", "stock_vca", row, json!({"level": 1.0}), "VCA");
                let port = if kind == "analog_lead" {
                    "lowpass"
                } else {
                    "out"
                };
                self.con("lead_vcf", port, "lead_vca", "signal");
                self.con("lead_gate", "out", "lead_vca", "cv");
                Some(("lead_vca".into(), "out".into()))
            }
            _ => None,
        }
    }
}
