//! Patch Engine: three words (genre · mood · texture) plus a seed compose a complete, laid-out patch.
//!
//! The words set the style; the seed composes a new piece inside it (key, progression, voicing, drum,
//! bass, chord and melody patterns, optional layers, sound design, song form). Output is the same edit
//! vocabulary `apply_edits` and `start_construction` accept, so a build reuses preflight, pacing, undo
//! and construction motion. Voices come from the instrument designs in `instruments`: macro-synthesis engines,
//! modal resonators, wavetables and analog models next to the original subtractive voices.
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

mod instruments;
use instruments::{choose, light_of, Role};

#[cfg(test)]
thread_local! {
    /// Designs a test pins (one per role), so rarely chosen designs still get built and validated.
    static PINNED: std::cell::RefCell<Vec<&'static str>> = const { std::cell::RefCell::new(vec![]) };
}

#[cfg(test)]
#[path = "patch_engine_tests.rs"]
mod tests;

pub const GENRES: [&str; 13] = [
    "Deep House",
    "Dub Techno",
    "Techno",
    "Ambient",
    "Lo-fi Hip Hop",
    "Synthwave",
    "Downtempo",
    "UK Garage",
    "Progressive House",
    "Hypnagogic",
    "Berlin School",
    "IDM",
    "Melodic Techno",
];

/// Performance controls (0–1, centre 0.5 = the genre's own character; the centre reproduces every seed exactly).
/// Energy: tempo and layer density. Complexity: ghost notes, counter-lines, variation. Weirdness: collage forms,
/// generative arps, rarer sounds. Space: reverb and echo depth. Length: bars in the song form (32, 64 or 128).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    pub energy: f64,
    pub complexity: f64,
    pub weirdness: f64,
    pub space: f64,
    pub length: u32,
}
impl Default for Controls {
    fn default() -> Self {
        Self {
            energy: 0.5,
            complexity: 0.5,
            weirdness: 0.5,
            space: 0.5,
            length: 64,
        }
    }
}
impl Controls {
    fn validate(&self) -> Result<(), String> {
        for (name, v) in [
            ("energy", self.energy),
            ("complexity", self.complexity),
            ("weirdness", self.weirdness),
            ("space", self.space),
        ] {
            if !(0.0..=1.0).contains(&v) {
                return Err(format!("{name} must be in 0..=1"));
            }
        }
        if ![32, 64, 128].contains(&self.length) {
            return Err("length must be 32, 64 or 128 bars".into());
        }
        Ok(())
    }
    /// Weight multiplier for weird choices: 0.2 at 0, 1 at the centre, 1.8 at 1.
    fn weird(&self) -> f64 {
        0.2 + 1.6 * self.weirdness
    }
}

/// Everything that defines a Patch Engine piece: the three words, the controls and three independent seeds — the
/// song (tempo, key, harmony, melody, rhythm, form), the sound (instrument designs) and the detail (envelopes, mix,
/// effects settings). A plain seed uses it for all three. Shareable as text:
/// `Genre|Mood|Texture|song.sound.detail[|energy.complexity.weirdness.space.length]` (controls in percent).
#[derive(Clone, Debug, PartialEq)]
pub struct Genome {
    pub genre: String,
    pub mood: String,
    pub texture: String,
    pub song: u64,
    pub sound: u64,
    pub detail: u64,
    pub controls: Controls,
}
impl Genome {
    pub fn new(genre: &str, mood: &str, texture: &str, seed: u64, controls: Controls) -> Self {
        Self {
            genre: genre.into(),
            mood: mood.into(),
            texture: texture.into(),
            song: seed,
            sound: seed,
            detail: seed,
            controls,
        }
    }

    /// A related genome. `sound`: new instruments and details, same song. `song`: new song, same instruments.
    /// `evolve`: new details and a small drift of the controls. `entropy` makes repeated mutations differ.
    pub fn mutate(&self, kind: &str, entropy: u64) -> Result<Genome, String> {
        let mut rng = Rng::new(&format!("mutate|{kind}|{entropy}|{self}"));
        let mut g = self.clone();
        match kind {
            "sound" => {
                g.sound = rng.int(1, 99_999) as u64;
                g.detail = rng.int(1, 99_999) as u64;
            }
            "song" => g.song = rng.int(1, 99_999) as u64,
            "evolve" => {
                g.detail = rng.int(1, 99_999) as u64;
                let c = &mut g.controls;
                for v in [
                    &mut c.energy,
                    &mut c.complexity,
                    &mut c.weirdness,
                    &mut c.space,
                ] {
                    *v = r3((*v + rng.uni(-0.12, 0.12)).clamp(0.0, 1.0));
                }
            }
            _ => return Err("mutation must be sound, song or evolve".into()),
        }
        Ok(g)
    }

    /// Cross two genomes: the words and song of `self`, the instruments and details of `other`, the controls
    /// in between.
    pub fn breed(&self, other: &Genome) -> Genome {
        let (a, b) = (self.controls, other.controls);
        let mid = |x: f64, y: f64| r3((x + y) / 2.0);
        Genome {
            sound: other.sound,
            detail: other.detail,
            controls: Controls {
                energy: mid(a.energy, b.energy),
                complexity: mid(a.complexity, b.complexity),
                weirdness: mid(a.weirdness, b.weirdness),
                space: mid(a.space, b.space),
                length: a.length,
            },
            ..self.clone()
        }
    }
}
impl std::fmt::Display for Genome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}|{}|{}|{}.{}.{}",
            self.genre, self.mood, self.texture, self.song, self.sound, self.detail
        )?;
        let c = self.controls;
        if c != Controls::default() {
            let pct = |v: f64| (v * 100.0).round() as i64;
            write!(
                f,
                "|{}.{}.{}.{}.{}",
                pct(c.energy),
                pct(c.complexity),
                pct(c.weirdness),
                pct(c.space),
                c.length
            )?;
        }
        Ok(())
    }
}
impl std::str::FromStr for Genome {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, String> {
        let parts: Vec<&str> = text.trim().split('|').collect();
        if !(4..=5).contains(&parts.len()) {
            return Err("genome must be Genre|Mood|Texture|song.sound.detail[|controls]".into());
        }
        let seeds: Vec<u64> = parts[3]
            .split('.')
            .map(|x| x.parse::<u64>().map_err(|_| format!("bad seed {x}")))
            .collect::<Result<_, _>>()?;
        let [song, sound, detail] = seeds[..] else {
            return Err("genome needs three seeds: song.sound.detail".into());
        };
        let mut controls = Controls::default();
        if let Some(c) = parts.get(4) {
            let v: Vec<f64> = c
                .split('.')
                .map(|x| x.parse::<f64>().map_err(|_| format!("bad control {x}")))
                .collect::<Result<_, _>>()?;
            let [e, cx, w, sp, len] = v[..] else {
                return Err("genome controls need energy.complexity.weirdness.space.length".into());
            };
            controls = Controls {
                energy: e / 100.0,
                complexity: cx / 100.0,
                weirdness: w / 100.0,
                space: sp / 100.0,
                length: len as u32,
            };
            controls.validate()?;
        }
        Ok(Genome {
            genre: parts[0].into(),
            mood: parts[1].into(),
            texture: parts[2].into(),
            song,
            sound,
            detail,
            controls,
        })
    }
}

/// The original genre whose drum sounds, chord colour and mix details a newer genre borrows; each newer genre
/// keeps its own tempo, grooves, instruments, melodies and song forms.
fn parent(genre: &str) -> &str {
    match genre {
        "Progressive House" => "Deep House",
        "Hypnagogic" => "Ambient",
        "Berlin School" => "Synthwave",
        "IDM" => "Downtempo",
        "Melodic Techno" => "Techno",
        g => g,
    }
}
pub const MOODS: [&str; 8] = [
    "Dreamy",
    "Dark",
    "Euphoric",
    "Melancholy",
    "Hypnotic",
    "Warm",
    "Mysterious",
    "Playful",
];
pub const TEXTURES: [&str; 8] = [
    "Glitchy", "Dusty", "Spacious", "Crisp", "Liquid", "Minimal", "Echoing", "Pulsing",
];

/// Noun used in the sentence "A dreamy Deep House track …".
pub fn genre_noun(genre: &str) -> &'static str {
    match genre {
        "Deep House" => "Deep House track",
        "Dub Techno" => "Dub Techno groove",
        "Techno" => "Techno track",
        "Ambient" => "Ambient piece",
        "Lo-fi Hip Hop" => "Lo-fi Hip Hop beat",
        "Synthwave" => "Synthwave tune",
        "Downtempo" => "Downtempo groove",
        "Minecraft" => "Minecraft-style piece",
        "Progressive House" => "Progressive House track",
        "Hypnagogic" => "Hypnagogic piece",
        "Berlin School" => "Berlin School sequence",
        "IDM" => "IDM piece",
        "Melodic Techno" => "Melodic Techno track",
        _ => "UK Garage track",
    }
}

/// Phrase that completes the sentence for a texture.
pub fn texture_phrase(texture: &str) -> &'static str {
    match texture {
        "Glitchy" => "full of glitches",
        "Dusty" => "with a dusty tape feel",
        "Spacious" => "with endless space",
        "Crisp" => "with a crisp edge",
        "Liquid" => "that flows like liquid",
        "Minimal" => "stripped right back",
        "Echoing" => "drenched in echoes",
        _ => "that pulses and breathes",
    }
}

pub fn texture_text(texture: &str) -> &'static str {
    match texture {
        "Glitchy" => "stutter and bit-crush bus between the mixer and the master, sequenced on the drum clock",
        "Dusty" => "tape hiss bed, darker filters and a lazier swing",
        "Spacious" => "long hall, longer echo and a slow wind band sweeping underneath",
        "Crisp" => "tighter drums, brighter filters, drier reverb",
        "Liquid" => "slow filter tide on the chords, smeared ping-pong echo, gliding lead",
        "Minimal" => "stripped back: fewer layers, more echo on what is left",
        "Echoing" => "dub echo throws on the chords and lead, long feedback",
        _ => "rhythmic gate chopping the chords on the grid, deeper kick pump",
    }
}

// ---------------------------------------------------------------- random
struct Rng(u64);
impl Rng {
    fn new(text: &str) -> Self {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for b in text.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        Rng(h)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn f(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn uni(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.f()
    }
    fn int(&mut self, a: i64, b: i64) -> i64 {
        a + (self.next() % ((b - a + 1) as u64)) as i64
    }
    fn idx(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[self.idx(v.len())]
    }
    fn chance(&mut self, p: f64) -> bool {
        self.f() < p
    }
    fn weighted<T: Copy>(&mut self, v: &[(T, f64)]) -> T {
        let total: f64 = v.iter().map(|x| x.1).sum();
        let mut r = self.f() * total;
        for &(item, w) in v {
            r -= w;
            if r <= 0.0 {
                return item;
            }
        }
        v[v.len() - 1].0
    }
    fn sample(&mut self, n: usize, k: usize) -> Vec<usize> {
        let mut all: Vec<usize> = (0..n).collect();
        for i in 0..k.min(n) {
            let j = i + self.idx(n - i);
            all.swap(i, j);
        }
        all.truncate(k.min(n));
        all
    }
}

fn r3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// Stock VCF frequency knob for a cutoff in Hz (C4 · 2^(10·f − 5)). The stock four-pole VCF costs ~1/40 of the
/// analog-modelled ladder and ~1/7 of the SVF per voice, which keeps every Patch Engine build inside a Steam Deck core.
fn vcf_knob(hz: f64) -> f64 {
    r3(((hz.clamp(20.0, 18000.0) / 261.6256).log2() + 5.0) / 10.0)
}

// ---------------------------------------------------------------- score helpers
fn st(step: i64, notes: &[i64], vel: f64, gate: f64, prob: f64, slide: bool) -> Value {
    let mut s = json!({"step": step, "notes": notes, "velocity": r3(vel), "gate": r3(gate)});
    if prob < 1.0 {
        s["probability"] = json!(r3(prob));
    }
    if slide {
        s["slide"] = json!(true);
    }
    s
}
fn hit(step: i64, vel: f64) -> Value {
    st(step, &[0], vel, 0.4, 1.0, false)
}
fn held(start: i64, len: i64, notes: &[i64], vel: f64, last_gate: f64) -> Vec<Value> {
    (0..len)
        .map(|k| {
            st(
                start + k,
                notes,
                vel,
                if k < len - 1 { 1.0 } else { last_gate },
                1.0,
                k < len - 1,
            )
        })
        .collect()
}
fn trk(steps: Vec<Value>, voices: u32, glide: u32) -> Value {
    let mut by: BTreeMap<i64, Value> = BTreeMap::new();
    for s in steps {
        let k = s["step"].as_i64().unwrap_or(-1);
        if (0..64).contains(&k) {
            by.insert(k, s);
        }
    }
    json!({"voices": voices, "glide_ms": glide, "steps": by.into_values().collect::<Vec<_>>()})
}
fn lane(points: &[(f64, f64, &str)]) -> Value {
    json!({"length_bars": 64, "looped": true,
           "points": points.iter().map(|(b, v, c)| json!({"bar": b, "value": r3(*v), "curve": c})).collect::<Vec<_>>()})
}
fn euclid(k: i64, n: i64, rot: i64) -> Vec<i64> {
    let mut v: Vec<i64> = (0..k).map(|i| (i * n / k + rot) % n).collect();
    v.sort();
    v.dedup();
    v
}

// ---------------------------------------------------------------- harmony
fn quality(q: &str) -> &'static [i64] {
    match q {
        "maj9" => &[4, 7, 11, 14],
        "m9" => &[3, 7, 10, 14],
        "m7" => &[0, 3, 7, 10],
        "maj7" => &[0, 4, 7, 11],
        "dom9" => &[4, 7, 10, 14],
        "m11" => &[3, 7, 10, 17],
        "min_open" => &[0, 7, 12, 15], // root · fifth · octave · minor tenth
        "m7sh" => &[0, 3, 10, 14],     // shell: root · ♭3 · ♭7 · 9
        "maj7sh" => &[0, 4, 11, 14],   // shell: root · 3 · 7 · 9
        "add9" => &[0, 4, 7, 14],
        "maj_open" => &[0, 7, 12, 16],
        _ => &[4, 7, 11, 18], // maj7#11
    }
}
/// Scale steps of a mood's mode, from the tonic.
fn mode_steps(mode: &str) -> [i64; 7] {
    match mode {
        "minor" => [0, 2, 3, 5, 7, 8, 10],
        "dorian" => [0, 2, 3, 5, 7, 9, 10],
        "phrygian" => [0, 1, 3, 5, 7, 8, 10],
        "lydian" => [0, 2, 4, 6, 7, 9, 11],
        "mixolydian" => [0, 2, 4, 5, 7, 9, 10],
        _ => [0, 2, 4, 5, 7, 9, 11],
    }
}
fn qname(q: &str) -> &'static str {
    match q {
        "maj9" => "maj9",
        "m9" => "m9",
        "m7" => "m7",
        "maj7" => "maj7",
        "dom9" => "9",
        "m11" => "m11",
        "min_open" => "m",
        "maj_open" => "",
        "m7sh" => "m7",
        "maj7sh" => "maj7",
        "add9" => "add9",
        _ => "maj7♯11",
    }
}
const NOTE: [&str; 12] = [
    "C", "D♭", "D", "E♭", "E", "F", "F♯", "G", "A♭", "A", "B♭", "B",
];

type Prog = &'static [(i64, &'static str)];
struct MoodDef {
    keys: &'static [i64],
    mode: &'static str,
    bright: f64,
    space: f64,
    bpm: i64,
    progs: &'static [Prog],
}

fn mood_def(m: &str) -> MoodDef {
    match m {
        "Dreamy" => MoodDef {
            keys: &[2, 7, 4],
            mode: "lydian",
            bright: 0.0,
            space: 0.8,
            bpm: -1,
            progs: &[
                &[(0, "maj9"), (9, "m9"), (5, "maj9"), (2, "m9")],
                &[(0, "maj9"), (2, "dom9"), (5, "maj9"), (0, "maj9")],
                &[(5, "maj9"), (0, "maj9"), (5, "maj9"), (4, "m7")],
                &[(0, "maj9"), (4, "m9"), (9, "m9"), (5, "maj9")],
                &[(0, "maj7s11"), (2, "dom9"), (0, "maj7s11"), (7, "maj9")],
                &[(0, "maj9"), (7, "maj9"), (9, "m9"), (7, "maj9")],
            ],
        },
        "Dark" => MoodDef {
            keys: &[5, 0, 7],
            mode: "minor",
            bright: -0.7,
            space: 0.5,
            bpm: 0,
            progs: &[
                &[(0, "m9"), (8, "maj7"), (5, "m7"), (7, "m7")],
                &[(0, "m11"), (0, "m11"), (10, "dom9"), (8, "maj7")],
                &[(0, "m9"), (5, "m9"), (0, "m9"), (5, "m9")],
                &[(0, "m9"), (1, "maj7"), (0, "m9"), (10, "dom9")],
                &[(0, "m9"), (3, "maj7"), (8, "maj7"), (7, "m7")],
                &[(0, "m11"), (5, "m7"), (8, "maj7"), (10, "dom9")],
            ],
        },
        "Euphoric" => MoodDef {
            keys: &[5, 0, 9],
            mode: "major",
            bright: 0.4,
            space: 0.6,
            bpm: 2,
            progs: &[
                &[(9, "m9"), (5, "maj9"), (0, "maj9"), (7, "dom9")],
                &[(0, "maj9"), (7, "dom9"), (9, "m9"), (5, "maj9")],
                &[(5, "maj9"), (7, "dom9"), (4, "m7"), (9, "m9")],
                &[(0, "maj9"), (4, "m9"), (5, "maj9"), (7, "dom9")],
                &[(9, "m9"), (7, "dom9"), (5, "maj9"), (7, "dom9")],
                &[(0, "maj9"), (9, "m9"), (5, "maj9"), (7, "dom9")],
            ],
        },
        "Melancholy" => MoodDef {
            keys: &[9, 4, 11],
            mode: "minor",
            bright: -0.3,
            space: 0.7,
            bpm: -2,
            progs: &[
                &[(0, "m9"), (5, "m9"), (10, "dom9"), (3, "maj9")],
                &[(0, "m9"), (8, "maj9"), (3, "maj9"), (10, "dom9")],
                &[(5, "m9"), (7, "m9"), (0, "m9"), (0, "m9")],
                &[(0, "m9"), (10, "dom9"), (8, "maj7"), (7, "m7")],
                &[(0, "m9"), (3, "maj9"), (10, "dom9"), (5, "m9")],
                &[(8, "maj7"), (10, "dom9"), (0, "m9"), (0, "m9")],
            ],
        },
        "Hypnotic" => MoodDef {
            keys: &[2, 9, 7],
            mode: "dorian",
            bright: -0.2,
            space: 0.5,
            bpm: 0,
            progs: &[
                &[(0, "m11"), (0, "m11"), (5, "m9"), (0, "m11")],
                &[(0, "m11"), (10, "dom9"), (0, "m11"), (10, "dom9")],
                &[(0, "m11"), (0, "m11"), (0, "m11"), (0, "m11")],
                &[(0, "m9"), (8, "maj7"), (0, "m9"), (8, "maj7")],
                &[(0, "m9"), (5, "dom9"), (0, "m9"), (5, "dom9")],
                &[(0, "m11"), (2, "m7"), (0, "m11"), (10, "maj7")],
            ],
        },
        "Warm" => MoodDef {
            keys: &[3, 8, 10],
            mode: "major",
            bright: -0.2,
            space: 0.55,
            bpm: -1,
            progs: &[
                &[(0, "maj9"), (2, "m9"), (4, "m9"), (5, "maj9")],
                &[(0, "maj9"), (9, "m9"), (2, "m9"), (7, "dom9")],
                &[(5, "maj9"), (4, "m9"), (2, "m9"), (0, "maj9")],
                &[(0, "maj9"), (5, "maj9"), (0, "maj9"), (5, "maj9")],
                &[(0, "maj9"), (4, "m9"), (5, "maj9"), (7, "dom9")],
                &[(5, "maj9"), (7, "dom9"), (9, "m9"), (0, "maj9")],
            ],
        },
        "Mysterious" => MoodDef {
            keys: &[4, 11, 6],
            mode: "phrygian",
            bright: -0.4,
            space: 0.75,
            bpm: -1,
            progs: &[
                &[(0, "m11"), (1, "maj7"), (0, "m11"), (1, "maj7")],
                &[(0, "m7"), (10, "m7"), (8, "maj7"), (1, "maj7")],
                &[(0, "m11"), (8, "maj7"), (1, "maj7"), (0, "m11")],
                &[(0, "m7"), (3, "dom9"), (1, "maj7"), (0, "m7")],
                &[(0, "m11"), (0, "m11"), (1, "maj7"), (10, "m7")],
                &[(0, "m7"), (5, "m7"), (1, "maj7"), (0, "m7")],
            ],
        },
        _ => MoodDef {
            keys: &[0, 5, 7],
            mode: "mixolydian",
            bright: 0.3,
            space: 0.45,
            bpm: 2,
            progs: &[
                &[(0, "maj9"), (10, "maj7"), (5, "maj9"), (0, "maj9")],
                &[(0, "maj9"), (9, "m7"), (2, "m9"), (7, "dom9")],
                &[(5, "maj9"), (7, "dom9"), (0, "maj9"), (0, "maj9")],
                &[(0, "dom9"), (5, "dom9"), (0, "dom9"), (7, "dom9")],
                &[(0, "maj9"), (2, "m9"), (5, "maj9"), (7, "dom9")],
                &[(0, "maj9"), (5, "maj9"), (9, "m7"), (7, "dom9")],
            ],
        },
    }
}

struct GenreDef {
    bpm: (i64, i64),
    swing: (i64, i64),
    div: i64,
    drums: &'static str,
    bass: &'static [&'static str],
    voices: &'static [&'static str],
    rhythm: &'static [&'static str],
    leads: &'static [(&'static str, f64)],
    counter: f64,
    perc: f64,
    perc2: f64,
    pad: f64,
    pump: bool,
    kick: (f64, f64, f64, f64),
    hats: &'static [&'static str],
    ohat: &'static [&'static str],
}
fn genre_def(g: &str) -> GenreDef {
    match g {
        // Pryda-style: rolling bass, plucks and saw pads under long filter builds
        "Progressive House" => GenreDef {
            bpm: (123, 126),
            swing: (50, 52),
            div: 1,
            drums: "four",
            bass: &["roll", "octave", "pulse"],
            voices: &["pluck", "stab", "pad_saw"],
            rhythm: &["syncop", "stabs", "pulse8"],
            leads: &[("arp", 4.0), ("hook", 2.0), ("drift", 1.0), ("none", 1.0)],
            counter: 0.2,
            perc: 0.5,
            perc2: 0.2,
            pad: 0.6,
            pump: true,
            kick: (50.0, 0.42, 0.2, 0.3),
            hats: &["16", "off8"],
            ohat: &["off", "off", "half"],
        },
        // Oneohtrix Point Never-style: sparse pulse, drones, generative arps, collage cuts
        "Hypnagogic" => GenreDef {
            bpm: (84, 100),
            swing: (50, 54),
            div: 2,
            drums: "ambient",
            bass: &["drone", "sub"],
            voices: &["keys", "organ", "pad_saw"],
            rhythm: &["held", "restrike"],
            leads: &[("drift", 5.0), ("bells", 2.0), ("hook", 1.0), ("none", 1.0)],
            counter: 0.2,
            perc: 0.2,
            perc2: 0.0,
            pad: 0.6,
            pump: false,
            kick: (46.0, 0.5, 0.1, 0.3),
            hats: &["euclid"],
            ohat: &["none"],
        },
        // sequencer music: a 16th ostinato bass, generative arps, analog pads, no drums
        "Berlin School" => GenreDef {
            bpm: (100, 116),
            swing: (50, 50),
            div: 1,
            drums: "none",
            bass: &["roll", "pulse"],
            voices: &["pad_saw", "organ", "pad_square"],
            rhythm: &["held"],
            leads: &[("drift", 4.0), ("arp", 4.0), ("none", 1.0)],
            counter: 0.3,
            perc: 0.0,
            perc2: 0.0,
            pad: 0.5,
            pump: false,
            kick: (48.0, 0.4, 0.2, 0.3),
            hats: &["none", "none", "euclid"],
            ohat: &["none"],
        },
        "IDM" => GenreDef {
            bpm: (100, 130),
            swing: (50, 56),
            div: 1,
            drums: "broken",
            bass: &["gen", "broken"],
            voices: &["pluck", "keys", "stab"],
            rhythm: &["syncop", "stabs"],
            leads: &[("drift", 3.0), ("riff", 2.0), ("bells", 2.0), ("arp", 1.0)],
            counter: 0.2,
            perc: 0.7,
            perc2: 0.5,
            pad: 0.3,
            pump: false,
            kick: (52.0, 0.3, 0.4, 0.4),
            hats: &["euclid", "shuffle"],
            ohat: &["none", "half"],
        },
        "Melodic Techno" => GenreDef {
            bpm: (120, 124),
            swing: (50, 50),
            div: 1,
            drums: "techno",
            bass: &["roll", "pulse", "gen"],
            voices: &["pad_saw", "stab", "pluck"],
            rhythm: &["pulse8", "syncop", "held"],
            leads: &[("arp", 4.0), ("hook", 2.0), ("drift", 2.0)],
            counter: 0.2,
            perc: 0.4,
            perc2: 0.2,
            pad: 0.7,
            pump: true,
            kick: (48.0, 0.4, 0.35, 0.35),
            hats: &["16", "off8"],
            ohat: &["off", "half"],
        },
        "Deep House" => GenreDef {
            bpm: (120, 124),
            swing: (54, 58),
            div: 1,
            drums: "four",
            bass: &["house", "gen", "sub"],
            voices: &["keys", "pluck", "organ", "pad_saw"],
            rhythm: &["stabs", "stabs", "syncop", "held"],
            leads: &[("hook", 2.0), ("arp", 2.0), ("bells", 2.0), ("none", 3.0)],
            counter: 0.3,
            perc: 0.75,
            perc2: 0.3,
            pad: 0.35,
            pump: true,
            kick: (50.0, 0.45, 0.15, 0.3),
            hats: &["16", "16", "euclid", "shuffle"],
            ohat: &["off", "off", "half"],
        },
        "Dub Techno" => GenreDef {
            bpm: (117, 123),
            swing: (52, 56),
            div: 2,
            drums: "dub",
            bass: &["pulse", "gen", "sub"],
            voices: &["stab", "pluck", "pad_square"],
            rhythm: &["stabs", "stabs", "syncop"],
            leads: &[("none", 5.0), ("hook", 2.0), ("bells", 2.0), ("drift", 1.5)],
            counter: 0.2,
            perc: 0.6,
            perc2: 0.2,
            pad: 0.5,
            pump: true,
            kick: (46.0, 0.55, 0.2, 0.3),
            hats: &["16", "euclid", "shuffle"],
            ohat: &["off", "half", "none"],
        },
        "Techno" => GenreDef {
            bpm: (126, 134),
            swing: (50, 52),
            div: 1,
            drums: "techno",
            bass: &["roll", "gen", "sub"],
            voices: &["stab", "pluck", "pad_square"],
            rhythm: &["stabs", "syncop", "pulse8"],
            leads: &[("arp", 4.0), ("riff", 4.0), ("drift", 2.0), ("none", 2.0)],
            counter: 0.15,
            perc: 0.5,
            perc2: 0.25,
            pad: 0.3,
            pump: true,
            kick: (48.0, 0.38, 0.4, 0.35),
            hats: &["16", "off8", "euclid"],
            ohat: &["off", "off", "half"],
        },
        "Ambient" => GenreDef {
            bpm: (70, 90),
            swing: (50, 52),
            div: 2,
            drums: "ambient",
            bass: &["drone"],
            voices: &["pad_saw", "pad_square", "organ", "keys"],
            rhythm: &["held", "held", "restrike"],
            leads: &[
                ("drift", 3.0),
                ("bells", 5.0),
                ("hook", 2.0),
                ("counter", 3.0),
                ("arp", 1.0),
            ],
            counter: 0.4,
            perc: 0.35,
            perc2: 0.1,
            pad: 0.0,
            pump: false,
            kick: (44.0, 0.6, 0.0, 0.2),
            hats: &["sparse", "none"],
            ohat: &["none"],
        },
        "Lo-fi Hip Hop" => GenreDef {
            bpm: (76, 90),
            swing: (58, 66),
            div: 1,
            drums: "boombap",
            bass: &["lofi", "gen", "sub"],
            voices: &["keys", "organ", "pluck", "pad_square"],
            rhythm: &["stabs", "held", "restrike"],
            leads: &[
                ("hook", 2.0),
                ("bells", 3.0),
                ("counter", 1.0),
                ("none", 3.0),
            ],
            counter: 0.25,
            perc: 0.5,
            perc2: 0.25,
            pad: 0.3,
            pump: false,
            kick: (52.0, 0.35, 0.25, 0.25),
            hats: &["8", "8", "16", "shuffle"],
            ohat: &["sparse", "none"],
        },
        "Synthwave" => GenreDef {
            bpm: (96, 112),
            swing: (50, 50),
            div: 1,
            drums: "rock",
            bass: &["octave", "gen"],
            voices: &["pad_saw", "pluck", "pad_square"],
            rhythm: &["held", "pulse8", "restrike"],
            leads: &[("arp", 5.0), ("hook", 3.0), ("riff", 1.0), ("drift", 1.5)],
            counter: 0.3,
            perc: 0.4,
            perc2: 0.1,
            pad: 0.3,
            pump: true,
            kick: (54.0, 0.32, 0.3, 0.4),
            hats: &["8", "16"],
            ohat: &["none", "none", "half"],
        },
        "Downtempo" => GenreDef {
            bpm: (90, 104),
            swing: (54, 60),
            div: 1,
            drums: "broken",
            bass: &["broken", "gen", "sub"],
            voices: &["keys", "pad_saw", "pluck", "organ"],
            rhythm: &["held", "stabs", "syncop", "restrike"],
            leads: &[
                ("drift", 2.0),
                ("hook", 2.0),
                ("bells", 3.0),
                ("counter", 2.0),
                ("arp", 1.0),
                ("none", 1.0),
            ],
            counter: 0.35,
            perc: 0.6,
            perc2: 0.35,
            pad: 0.35,
            pump: false,
            kick: (52.0, 0.42, 0.2, 0.3),
            hats: &["16", "shuffle", "euclid", "8"],
            ohat: &["half", "sparse", "none"],
        },
        // C418-style mode: original music in that spirit (soft piano figures, slow drifting harmony, space)
        "Minecraft" => GenreDef {
            bpm: (64, 80),
            swing: (50, 50),
            div: 2,
            drums: "none",
            bass: &["drone"],
            voices: &["piano"],
            rhythm: &["broken"],
            leads: &[
                ("bells", 4.0),
                ("hook", 3.0),
                ("counter", 2.0),
                ("none", 1.0),
            ],
            counter: 0.3,
            perc: 0.0,
            perc2: 0.0,
            pad: 0.6,
            pump: false,
            kick: (44.0, 0.5, 0.0, 0.2),
            hats: &["none"],
            ohat: &["none"],
        },
        _ => GenreDef {
            bpm: (128, 134),
            swing: (60, 66),
            div: 1,
            drums: "twostep",
            bass: &["garage", "gen", "sub"],
            voices: &["organ", "stab", "keys", "pluck"],
            rhythm: &["stabs", "syncop"],
            leads: &[("hook", 3.0), ("none", 4.0), ("arp", 1.0)],
            counter: 0.2,
            perc: 0.5,
            perc2: 0.3,
            pad: 0.25,
            pump: true,
            kick: (52.0, 0.4, 0.25, 0.3),
            hats: &["shuffle", "16", "euclid"],
            ohat: &["off", "half"],
        },
    }
}

/// Genre chord colour: bold genres use open triads instead of jazz extensions; dub keeps its minor-seventh stabs.
fn colorize(genre: &str, q: &'static str) -> &'static str {
    let minor = matches!(q, "m9" | "m7" | "m11");
    match parent(genre) {
        "Techno" | "Synthwave" => {
            if minor {
                "min_open"
            } else {
                "maj_open"
            }
        }
        "Dub Techno" => match q {
            "m11" | "m7" => "m7sh",
            "maj9" | "maj7s11" => "maj7",
            "dom9" => "maj_open",
            _ => q,
        },
        "Deep House" => match q {
            "maj9" | "maj7s11" | "dom9" => "maj7sh",
            "m7" | "m11" => "m7sh",
            _ => q,
        },
        "UK Garage" => match q {
            "maj9" | "maj7s11" | "dom9" => "maj7sh",
            "m9" | "m11" | "m7" => "m7sh",
            _ => q,
        },
        "Ambient" => match q {
            "maj9" | "maj7s11" => "add9",
            "dom9" => "maj7sh",
            _ => q,
        },
        "Minecraft" => match q {
            "dom9" => "maj7sh",
            "m11" | "m7" => "m9",
            _ => q,
        },
        "Downtempo" => match q {
            "maj9" | "maj7s11" => "maj7sh",
            _ => q,
        },
        _ => q,
    }
}

/// Harmonic rhythm: which progression chord sounds in each of the four slots (a slot is one bar, two for slow genres).
/// Club genres hold a chord or vamp between two; song-like genres walk the full progression.
fn harmonic_rhythms(genre: &str) -> &'static [[usize; 4]] {
    match genre {
        "Techno" | "Dub Techno" => &[[0, 0, 0, 0]],
        "Deep House" | "UK Garage" => &[[0, 0, 1, 1], [0, 1, 0, 1]],
        "Ambient" => &[[0, 0, 1, 1]],
        "Minecraft" => &[[0, 0, 1, 1], [0, 1, 2, 3]],
        "Downtempo" => &[[0, 0, 1, 1], [0, 1, 2, 3]],
        "Synthwave" => &[[0, 1, 2, 3]],
        "Progressive House" => &[[0, 0, 1, 1], [0, 1, 2, 3]],
        "Melodic Techno" => &[[0, 0, 1, 1], [0, 1, 0, 1]],
        "Berlin School" => &[[0, 0, 0, 0], [0, 0, 1, 1]],
        "Hypnagogic" | "IDM" => &[[0, 1, 2, 3], [0, 0, 1, 1]],
        _ => &[[0, 1, 2, 3]],
    }
}

/// Mixer strip trims (dB) per voice, from offline stem measurements (56 random builds, all layers on) so each
/// voice lands at its target level relative to the kick: chords ≈ 0…−3 dB, sub −2, bass −7, lead −3…−5.
fn trim_db(name: &str) -> f64 {
    match name {
        "keys" => -3.9,
        "pluck" => 8.2,
        "organ" => -14.8,
        "pad_saw" => 14.0,
        "pad_square" => 5.5,
        "stab" => 5.1,
        "bass:ladder_saw" => -2.6,
        "bass:pwm" => -9.1,
        "bass:round" => -10.8,
        "bass:square" => -7.9,
        "lead:hook" => -6.7,
        "lead:arp" => -5.2,
        "lead:bells" => -12.7,
        "lead:riff" => 2.4,
        "lead:counter" => -13.0,
        // catalog designs (stems run of 160 builds, 2026-10-19): chords → 0 dB, pads → −2, pad layer → −3,
        // bass → −4.5, lead → −5 relative to the kick; swarm confirmed on 14 swarm builds
        "fm_keys" => -7.3,
        "fold_stab" => -3.7,
        "harmonic_organ" => -6.3,
        "wt_pad" => -14.6,
        "swarm_pad" => 2.4,
        "glass" => -1.4,
        "analog_pluck" => 6.0,
        "pad:wt_pad" => -3.8,
        "pad:swarm_pad" => 11.7,
        "bass:analog_bass" => -2.8,
        "bass:fm_bass" => -9.4,
        "bass:fold_bass" => -7.9,
        "bass:moog_bass" => -0.8,
        "lead:analog_lead" => -1.5,
        "lead:fm_bell" => -11.2,
        "lead:fold_lead" => -8.0,
        "lead:harmonic_lead" => -6.8,
        "lead:modal_bell" => -13.6,
        "lead:wt_lead" => -9.6,
        "lead:swarm_lead" => 7.6,
        _ => 0.0,
    }
}

/// Final out gain by genre: fitted on random full-mix renders of the drops (bars 18 and 42) so peaks stay ≤ ~0.88 (the limiter misses
/// fast kick transients), which also evens loudness between genres.
fn out_gain(genre: &str) -> f64 {
    match genre {
        "Deep House" => 1.05,
        "Dub Techno" => 1.2,
        "Techno" => 1.2,
        "Ambient" => 1.6,
        "Lo-fi Hip Hop" => 1.3,
        "Synthwave" => 0.95,
        "Downtempo" => 1.25,
        "Minecraft" => 0.85,
        "Progressive House" => 1.05,
        "Hypnagogic" => 1.25,
        "Berlin School" => 1.15,
        "IDM" => 1.15,
        "Melodic Techno" => 1.15,
        _ => 1.15,
    }
}

/// Offline DSP cost per module (% of one Apple M-series core at 48 kHz; mono, 4-voice), measured with every
/// instance live. A Steam Deck core is roughly 3–3.5× slower, so a budget of ~18 keeps builds under ~65% there.
fn module_cost(kind: &str, params: &Value, poly: bool) -> f64 {
    let (mono, four) = match kind {
        // macro voice cost depends on the engine: Analog, Wavefold, FM, Harmonic, Wavetable, Chord, Swarm, Metallic
        "macro_voice" => match params["engine"].as_i64().unwrap_or(0) {
            1 => (1.58, 5.19),
            2 => (2.41, 8.56),
            3 => (0.84, 2.17),
            4 => (0.83, 2.16),
            6 => (0.86, 2.42),
            5 => (0.61, 1.44),
            7 => (0.59, 1.47),
            _ => (0.58, 1.34),
        },
        "modal_resonator" => (0.5, 1.26),
        "wavetable_vco" => (0.49, 0.79),
        "rack264_vco" => (0.6, 0.67),
        "rack264_vcf" => (1.32, 1.32),
        "stock_delay" => (0.57, 0.63),
        "tidal_functions" => (0.31, 0.32),
        "mid_side" => (0.34, 0.4),
        "multi_vco" => (0.21, 0.78),
        "analog_vco" => (0.03, 0.23),
        "stock_vcf" => (1.46, 1.59),
        "svf_filter" => (1.35, 4.6),
        "ladder_vcf" => (7.5, 22.3),
        "stock_vca" => (0.05, 0.2),
        "analog_vca" => (3.2, 5.4),
        "adsr" => (0.07, 0.07),
        "adsr_eg" => (0.17, 0.16),
        "cv_mix" => (0.11, 0.2),
        "fade" => (0.16, 0.23),
        "sidechain_compressor" => (0.38, 0.38),
        "drum_voice" => (0.47, 0.47),
        "lowpass" => (0.04, 0.19),
        "stereo_reverb" => (0.41, 0.41),
        "stereo_delay" => (0.45, 0.45),
        "studio_mixer" => (1.12, 1.12),
        "score_sequencer" => (0.31, 0.31),
        "poly_split" => (0.2, 0.2),
        "poly_merge" => (0.1, 0.1),
        "poly_sum" => (0.07, 0.2),
        "gates" => (0.51, 0.51),
        "quantizer" => (0.18, 0.18),
        "multi_lfo" => (0.13, 0.13),
        "recall_random" => (0.31, 0.31),
        "colored_noise" => (0.49, 0.49),
        "wavetable_lfo" => (0.44, 0.44),
        "audio_stereo" => (0.05, 0.05),
        _ => (0.0, 0.0),
    };
    if poly {
        four
    } else {
        mono
    }
}

/// `start_construction` accepts at most this many edits per plan (showtime sends every edit as a step).
const MAX_PLAN_EDITS: usize = 256;

/// Optional layers dropped, in order, when a build exceeds its CPU budget.
const DROP_ORDER: [&str; 6] = [
    "glue compressor",
    "second percussion",
    "counter-line",
    "pad layer",
    "percussion",
    "open hat",
];

// ---------------------------------------------------------------- graph model
struct Module {
    kind: &'static str,
    row: u8,
    params: Value,
    label: String,
    content: Option<Value>,
}
#[derive(Clone)]
enum Op {
    Add(String),
    Con(String, String, String, String, String, bool),
    Out(String),
    Move(String, String, String),
}
struct Strip {
    name: String,
    level: f64,
    pan: f64,
    hpf: f64,
    send_a: f64,
    send_b: f64,
    high_db: f64,
}

/// A composed piece: graph, scores and the human-readable recipe.
pub struct Recipe {
    pub genre: String,
    pub mood: String,
    pub texture: String,
    pub seed: u64,
    pub title: String,
    pub sentence: String,
    pub description: String,
    pub notes: Vec<String>,
    pub bpm: i64,
    pub swing: i64,
    pub key_name: String,
    pub chord_names: Vec<String>,
    modules: HashMap<String, Module>,
    ops: Vec<Op>,
    cid: usize,
    strips: Vec<Strip>,
    lanes: Vec<&'static str>,
    scores: HashMap<String, Value>,
    kinds: BTreeMap<&'static str, String>,
    /// Bars where the song's main drops start (for listening checks and renders).
    drops: Vec<f64>,
    pub controls: Controls,
    pub genome: Genome,
}

struct Ctx {
    /// The song stream during composition; the detail stream while the graph is assembled.
    rng: Rng,
    /// The sound stream (instrument designs).
    srng: Rng,
    g: GenreDef,
    m: MoodDef,
    genre: &'static str,
    texture: &'static str,
    tonic: i64,
    chords: Vec<Vec<i64>>,
    roots: Vec<i64>,
    bright: f64,
    space: f64,
    six: f64,
    lead: &'static str,
    counter: bool,
    clap: bool,
    perc: Option<&'static str>,
    perc2: Option<&'static str>,
    pad: bool,
    voice: &'static str,
    bass_style: &'static str,
    form: &'static str,
    form_kind: &'static str,
    ohat: bool,
    break_shift: i64,
    drop2_shift: i64,
    glue: bool,
    light: bool,
    bass_design: &'static str,
    lead_design: &'static str,
    ctl: Controls,
}

fn find(list: &[&'static str], s: &str) -> Option<&'static str> {
    list.iter().copied().find(|x| *x == s)
}

fn voice_chords(
    rng: &mut Rng,
    tonic: i64,
    prog: &[(i64, &'static str)],
) -> (Vec<Vec<i64>>, Vec<i64>) {
    let target = rng.pick(&[-3.0, -1.0, 0.0, 2.0]);
    let spread = rng.chance(0.35);
    let mut out: Vec<Vec<i64>> = Vec::new();
    for &(root, q) in prog {
        let base: Vec<i64> = quality(q).iter().map(|i| tonic + root + i).collect();
        let centre = |v: &Vec<i64>| v.iter().sum::<i64>() as f64 / v.len() as f64;
        let want = out.last().map(centre).unwrap_or(target);
        let mut best: Option<Vec<i64>> = None;
        for o in -3..3 {
            let mut v: Vec<i64> = base.iter().map(|n| n + 12 * o).collect();
            if spread {
                // open voicing: lift the second voice an octave when that keeps every interval ≥ a minor third
                let mut w = v.clone();
                w[1] += 12;
                w.sort();
                if w.windows(2).all(|p| p[1] - p[0] >= 3) && *w.last().unwrap() <= 14 {
                    v = w;
                }
            }
            let lo = *v.iter().min().unwrap();
            if !(-12..=0).contains(&lo) || *v.iter().max().unwrap() > 14 {
                continue;
            }
            if best
                .as_ref()
                .is_none_or(|b| (centre(&v) - want).abs() < (centre(b) - want).abs())
            {
                best = Some(v);
            }
        }
        out.push(best.unwrap_or(base));
    }
    let roots = prog
        .iter()
        .map(|&(root, _)| {
            let mut r = (tonic + root).rem_euclid(12) - 36;
            while r < -33 {
                r += 12;
            }
            r
        })
        .collect();
    (out, roots)
}

impl Recipe {
    pub fn new(genre: &str, mood: &str, texture: &str, seed: u64) -> Result<Recipe, String> {
        Self::with_budget(genre, mood, texture, seed, None)
    }

    /// Compose within a CPU budget (see `module_cost`): expensive instrument designs first swap to their light
    /// stand-ins, then optional layers are dropped in `DROP_ORDER` until the estimate fits. `None` means no CPU
    /// limit. Every build also stays inside one construction plan (`MAX_PLAN_EDITS`). Deterministic for a given
    /// seed and budget: each attempt recomposes the same piece from scratch.
    pub fn with_budget(
        genre: &str,
        mood: &str,
        texture: &str,
        seed: u64,
        budget: Option<f64>,
    ) -> Result<Recipe, String> {
        Self::with_controls(genre, mood, texture, seed, budget, Controls::default())
    }

    /// As `with_budget`, shaped by the performance controls.
    pub fn with_controls(
        genre: &str,
        mood: &str,
        texture: &str,
        seed: u64,
        budget: Option<f64>,
        ctl: Controls,
    ) -> Result<Recipe, String> {
        Self::from_genome(&Genome::new(genre, mood, texture, seed, ctl), budget)
    }

    /// Build the piece a genome describes (see `Genome`), within an optional CPU budget.
    pub fn from_genome(genome: &Genome, budget: Option<f64>) -> Result<Recipe, String> {
        genome.controls.validate()?;
        let over_cpu = |r: &Recipe| budget.is_some_and(|b| r.cpu_estimate() > b);
        let mut r = Self::build(genome, false, 0)?;
        let light = over_cpu(&r);
        if light {
            r = Self::build(genome, true, 0)?;
        }
        for drop in 1..=DROP_ORDER.len() {
            if !over_cpu(&r) && r.ops.len() <= MAX_PLAN_EDITS {
                break;
            }
            r = Self::build(genome, light, drop)?;
            r.notes.push(format!(
                "light mode: left out {}",
                DROP_ORDER[..drop].join(", ")
            ));
        }
        Ok(r)
    }

    /// Estimated DSP load (% of one Apple M-series core).
    pub fn cpu_estimate(&self) -> f64 {
        let poly = |id: &str| {
            (id.starts_with("chord")
                || id.starts_with("pad")
                || id == "wow"
                || id == "tide"
                || id == "chop")
                && id != "pad_hold"
        };
        let modules: f64 = self
            .modules
            .iter()
            .map(|(id, m)| module_cost(m.kind, &m.params, poly(id)))
            .sum();
        r3(1.2 * (1.0 + modules + 0.01 * self.cid as f64))
    }

    fn build(genome: &Genome, light: bool, drop: usize) -> Result<Recipe, String> {
        let (genre, mood, texture, seed, ctl) = (
            genome.genre.as_str(),
            genome.mood.as_str(),
            genome.texture.as_str(),
            genome.song,
            genome.controls,
        );
        let genre = if genre == "Minecraft" {
            "Minecraft"
        } else {
            find(&GENRES, genre).ok_or("unknown genre")?
        };
        let mood_name = find(&MOODS, mood).ok_or("unknown mood")?;
        let texture = find(&TEXTURES, texture).ok_or("unknown texture")?;
        let mut rng = Rng::new(&format!("{genre}|{mood_name}|{texture}|{seed}"));
        // instrument designs and details draw from their own streams, so a song keeps its notes when its sounds
        // mutate (and the other way round)
        let mut srng = Rng::new(&format!(
            "sound|{genre}|{mood_name}|{texture}|{}",
            genome.sound
        ));
        let drng = Rng::new(&format!(
            "detail|{genre}|{mood_name}|{texture}|{}",
            genome.detail
        ));
        let g = genre_def(genre);
        let m = mood_def(mood_name);
        let tonic = rng.pick(m.keys);
        let colored: Vec<(i64, &'static str)> = m.progs[rng.idx(m.progs.len())]
            .iter()
            .map(|&(r, q)| (r, colorize(genre, q)))
            .collect();
        let rhythm = rng.pick(harmonic_rhythms(genre));
        // the four slots of the loop, in order (a vamp repeats chords)
        let prog: Vec<(i64, &'static str)> = rhythm.iter().map(|&i| colored[i]).collect();
        let (chords, roots) = voice_chords(&mut rng, tonic, &prog);
        let bpm = (rng.int(g.bpm.0, g.bpm.1) + m.bpm + ((ctl.energy - 0.5) * 10.0).round() as i64)
            .max(60);
        let swing =
            (rng.int(g.swing.0, g.swing.1) + if texture == "Dusty" { 4 } else { 0 }).clamp(50, 70);
        let bright = (m.bright
            + match texture {
                "Crisp" => 0.35,
                "Dusty" => -0.35,
                _ => 0.0,
            }
            + rng.uni(-0.15, 0.15))
        .clamp(-1.0, 1.0);
        let space = (m.space
            + if genre == "Minecraft" { 0.25 } else { 0.0 }
            + match texture {
                "Spacious" => 0.3,
                "Crisp" => -0.25,
                "Minimal" | "Echoing" => 0.1,
                _ => 0.0,
            }
            + rng.uni(-0.1, 0.1)
            + (ctl.space - 0.5) * 0.6)
            .clamp(0.2, 1.0);
        let minimal = texture == "Minimal";
        let lead = if minimal && rng.chance(0.6) {
            "none"
        } else {
            let leads: Vec<(&str, f64)> = g
                .leads
                .iter()
                .map(|&(l, w)| (l, if l == "drift" { w * ctl.weird() } else { w }))
                .collect();
            rng.weighted(&leads)
        };
        let counter = !minimal
            && lead != "counter"
            && rng.chance((g.counter * 2.0 * ctl.complexity).min(0.95));
        let kinds = ["conga", "tom", "wood", "shaker", "rim"];
        let busy = 0.4 + 1.2 * ctl.energy;
        let perc = rng
            .chance((g.perc * busy).min(1.0))
            .then(|| rng.pick(&kinds));
        let perc2 = (perc.is_some() && !minimal && rng.chance((g.perc2 * busy).min(1.0)))
            .then(|| rng.pick(&kinds))
            .filter(|k| Some(*k) != perc);
        // instrument designs: the genre's classic voices plus every catalog design, weighted by the three words
        let minecraft = genre == "Minecraft";
        let voice = if minecraft {
            srng.pick(g.voices)
        } else {
            choose(
                &mut srng,
                ctl.weirdness,
                Role::Chords,
                genre,
                mood_name,
                texture,
                g.voices,
                &[],
                |_| true,
            )
        };
        let voice_pad = instruments::design(voice).is_some_and(|x| x.tags.contains(&"pad"));
        let pad_roll = srng.chance((g.pad * (1.5 - ctl.energy)).min(1.0));
        let pad = !voice_pad && !minimal && pad_roll;
        let bass_style = rng.pick(g.bass);
        let bass_design = if minecraft || bass_style == "drone" {
            "sine"
        } else {
            choose(
                &mut srng,
                ctl.weirdness,
                Role::Bass,
                genre,
                mood_name,
                texture,
                &["ladder_saw", "ladder_saw", "square"],
                &[],
                |_| true,
            )
        };
        // percussive designs (low-pass gates, struck resonators) suit arps, riffs and bells; hooks need a held note
        let sustained_lead = matches!(lead, "hook" | "counter");
        let lead_design = if minecraft || lead == "none" {
            "classic"
        } else {
            choose(
                &mut srng,
                ctl.weirdness,
                Role::Lead,
                genre,
                mood_name,
                texture,
                &["classic"],
                &[],
                |x| x.sustain || !sustained_lead,
            )
        };
        let (voice, bass_design, lead_design) = if light {
            (
                light_of(voice),
                light_of(bass_design),
                light_of(lead_design),
            )
        } else {
            (voice, bass_design, lead_design)
        };
        #[cfg(test)]
        let (voice, bass_design, lead_design) = PINNED.with(|p| {
            let mut out = (voice, bass_design, lead_design);
            for &n in p.borrow().iter() {
                match instruments::design(n).map(|x| x.role) {
                    Some(Role::Chords) => out.0 = n,
                    Some(Role::Bass) => out.1 = n,
                    Some(Role::Lead) => out.2 = n,
                    None => {}
                }
            }
            out
        });
        // song form: the breakdown (and maybe the second drop) moves to a related key
        let shifts: &[i64] = match m.mode {
            "major" | "lydian" | "mixolydian" => &[5, -3, 0],
            "phrygian" => &[3, 0],
            _ => &[3, -4, 5, 0],
        };
        let song_like = matches!(genre, "Synthwave" | "Lo-fi Hip Hop" | "Downtempo");
        // song form: each seed picks a form the genre suits (the classic or 16-bar layering forms, a long
        // progressive filter build, or a collage of abrupt cuts)
        let forms: &[(&str, f64)] = match genre {
            "Minecraft" => &[("long", 1.0)],
            "Techno" => &[("long", 3.0), ("collage", 0.5), ("progressive", 1.5)],
            "Dub Techno" | "Ambient" => &[("long", 3.0), ("collage", 1.5)],
            "Deep House" | "Synthwave" | "UK Garage" => {
                &[("classic", 3.0), ("progressive", 2.0), ("collage", 0.3)]
            }
            "Progressive House" => &[("progressive", 5.0), ("classic", 1.0)],
            "Hypnagogic" => &[("collage", 4.0), ("long", 1.0)],
            "Berlin School" => &[("long", 3.0), ("progressive", 1.0)],
            "IDM" => &[("collage", 2.0), ("classic", 1.5)],
            "Melodic Techno" => &[("progressive", 3.0), ("long", 1.5)],
            _ => &[("classic", 3.0), ("progressive", 0.5), ("collage", 1.5)],
        };
        let forms: Vec<(&str, f64)> = forms
            .iter()
            .map(|&(f, w)| (f, if f == "collage" { w * ctl.weird() } else { w }))
            .collect();
        let form_kind = rng.weighted(&forms);
        let break_shift = if song_like {
            rng.pick(shifts)
        } else if form_kind == "collage" && rng.chance(0.6) {
            rng.pick(&shifts[..shifts.len() - 1])
        } else {
            0
        };
        let drop2_shift = if rng.chance(0.4) { break_shift } else { 0 };
        let form = match form_kind {
            "long" => "four 16-bar sections, layers arrive one by one",
            "progressive" => "a 32-bar filter build into the drop, a short breakdown, a last drop",
            "collage" => "a collage of abrupt cuts between scenes",
            _ => "intro · build · drop · breakdown · drop · outro",
        };
        let dropped = |layer: &str| DROP_ORDER[..drop].contains(&layer);
        let perc2 = if dropped("second percussion") {
            None
        } else {
            perc2
        };
        let counter = counter && !dropped("counter-line");
        let pad = pad && !dropped("pad layer");
        let (perc, perc2) = if dropped("percussion") {
            (None, None)
        } else {
            (perc, perc2)
        };
        let mut c = Ctx {
            rng,
            srng,
            genre,
            texture,
            tonic,
            chords,
            roots,
            bright,
            space,
            six: 60.0 / bpm as f64 / 4.0,
            lead,
            counter,
            clap: !minimal && genre != "Ambient" && genre != "Minecraft",
            perc,
            perc2,
            pad,
            voice,
            bass_style,
            form,
            form_kind,
            ctl,
            g,
            m,
            ohat: !dropped("open hat"),
            break_shift,
            drop2_shift,
            glue: !dropped("glue compressor"),
            light,
            bass_design,
            lead_design,
        };
        let key_name = format!("{} {}", NOTE[tonic as usize], c.m.mode);
        let mut chord_names: Vec<String> = Vec::new();
        for &i in rhythm.iter() {
            let (r, q) = colored[i];
            let name = format!("{}{}", NOTE[(tonic + r).rem_euclid(12) as usize], qname(q));
            if chord_names.last() != Some(&name) {
                chord_names.push(name);
            }
        }
        let mut r = Recipe {
            genre: genre.into(),
            mood: mood_name.into(),
            texture: texture.into(),
            seed,
            title: format!("{mood_name} {genre} · {texture}"),
            sentence: format!(
                "A {} {} {}.",
                mood_name.to_lowercase(),
                genre_noun(genre),
                texture_phrase(texture)
            ),
            description: String::new(),
            notes: Vec::new(),
            bpm,
            swing,
            key_name,
            chord_names,
            modules: HashMap::new(),
            ops: Vec::new(),
            cid: 0,
            strips: Vec::new(),
            lanes: Vec::new(),
            scores: HashMap::new(),
            kinds: BTreeMap::new(),
            drops: vec![],
            controls: ctl,
            genome: genome.clone(),
        };
        r.compose(&mut c);
        r.stretch_form(ctl.length);
        c.rng = drng;
        r.assemble(&mut c);
        Ok(r)
    }

    /// Stretch the 64-bar song form (every automation lane and the drop bars) to `bars`.
    fn stretch_form(&mut self, bars: u32) {
        if bars == 64 {
            return;
        }
        let k = bars as f64 / 64.0;
        for score in self.scores.values_mut() {
            let Some(lanes) = score.get_mut("lanes").and_then(|l| l.as_array_mut()) else {
                continue;
            };
            for lane in lanes {
                lane["length_bars"] = json!(bars);
                for p in lane["points"].as_array_mut().into_iter().flatten() {
                    let b = p["bar"].as_f64().unwrap_or(0.0);
                    p["bar"] = json!(r3(b * k));
                }
            }
        }
        for d in &mut self.drops {
            *d = r3(*d * k);
        }
        self.note(format!("song form stretched to {bars} bars"));
    }

    // ------------------------------------------------------------ builder helpers
    fn cut(bright: f64, hz: f64, amt: f64) -> f64 {
        r3((hz * 2f64.powf(bright * amt)).clamp(60.0, 9000.0))
    }
    fn module(
        &mut self,
        id: &str,
        kind: &'static str,
        row: u8,
        params: Value,
        label: impl Into<String>,
    ) {
        self.modules.insert(
            id.into(),
            Module {
                kind,
                row,
                params,
                label: label.into(),
                content: None,
            },
        );
        self.ops.push(Op::Add(id.into()));
    }
    fn label(&mut self, row: u8, text: String) {
        let id = format!("label_{row}");
        self.modules.insert(
            id.clone(),
            Module {
                kind: "notes",
                row,
                params: json!({}),
                label: format!("Row {row} label"),
                content: Some(json!({"type": "notes", "text": text})),
            },
        );
        self.ops.push(Op::Add(id));
    }
    fn con(&mut self, from: &str, out: &str, to: &str, input: &str) {
        self.con_mode(from, out, to, input, false)
    }
    fn con_mode(&mut self, from: &str, out: &str, to: &str, input: &str, stack: bool) {
        self.cid += 1;
        self.ops.push(Op::Con(
            format!("c{}", self.cid),
            from.into(),
            out.into(),
            to.into(),
            input.into(),
            stack,
        ));
    }
    #[allow(clippy::too_many_arguments)]
    fn strip(
        &mut self,
        name: &str,
        node: &str,
        port: &str,
        level: f64,
        pan: f64,
        hpf: f64,
        send_a: f64,
        send_b: f64,
        high_db: f64,
    ) {
        let n = self.strips.len() + 1;
        self.strips.push(Strip {
            name: name.into(),
            level,
            pan,
            hpf,
            send_a,
            send_b,
            high_db,
        });
        self.con(node, port, "mixer", &format!("in{n}"));
    }
    /// Pitch source for a seq_music track, transposed by the song-form key lane when the arrangement changes key.
    fn key_src(&mut self, c: &Ctx, port: &str) -> (String, String) {
        if port == "pitch3" && c.lead == "drift" {
            return self.drift_src(c, "pitch");
        }
        if c.break_shift == 0 && c.drop2_shift == 0 {
            return ("seq_music".into(), port.into());
        }
        let id = format!("key_{port}");
        if !self.modules.contains_key(&id) {
            self.module(
                &id,
                "cv_mix",
                1,
                json!({"level1": 1.0, "level2": 1.0, "level3": 0.0}),
                format!("Key change · {port}"),
            );
            self.con("seq_music", port, &id, "in1");
            let kl = self.lane_out("key");
            self.con("lanes", &kl, &id, "in2");
        }
        (id, "out".into())
    }
    /// Gate source for a seq_music track (the generative arp replaces the melody track's gate).
    fn gate_src(&mut self, c: &Ctx, port: &str) -> (String, String) {
        if port == "gate3" && c.lead == "drift" {
            return self.drift_src(c, "gate");
        }
        ("seq_music".into(), port.into())
    }

    /// Generative arp: recall_random draws looping random notes that slowly mutate (déjà vu) on random 16ths of
    /// the master clock; a quantizer keeps them on the key's pentatonic notes, the key lane follows key changes.
    fn drift_src(&mut self, c: &Ctx, what: &str) -> (String, String) {
        if !self.modules.contains_key("drift") {
            let h = self.seed;
            let length = [5.0, 8.0, 8.0, 12.0, 16.0][(h / 7 % 5) as usize];
            self.module(
                "drift",
                "recall_random",
                5,
                json!({"seed": h % 65_534 + 1, "deja_vu": r3((0.6 + (h % 26) as f64 / 100.0 - (c.ctl.weirdness - 0.5) * 0.4).clamp(0.2, 0.95)),
                       "length": length,
                       "t_repeat": 1, "x_repeat": 1, "t_model": 0, "t_bias": r3(0.4 + (h / 11 % 25) as f64 / 100.0),
                       "x_range": 0, "x_spread": 0.3, "x_bias": 0.5, "x_steps": 0}),
                "Generative arp (looping random notes that slowly mutate)",
            );
            self.con("seq_drums", "clock", "drift", "t_clock");
            let steps = mode_steps(c.m.mode);
            let penta: [usize; 5] = if steps[2] == 4 {
                [0, 1, 2, 4, 5]
            } else {
                [0, 2, 3, 4, 6]
            };
            let mask: i64 = penta
                .iter()
                .map(|&i| 1i64 << (c.tonic + steps[i]).rem_euclid(12))
                .sum();
            self.module(
                "drift_q",
                "quantizer",
                5,
                json!({"scale_mask": mask, "offset": -0.5}),
                "Generative arp in key (pentatonic)",
            );
            self.con("drift", "x1", "drift_q", "pitch");
        }
        if what == "gate" {
            return ("drift".into(), "t1".into());
        }
        if c.break_shift == 0 && c.drop2_shift == 0 {
            return ("drift_q".into(), "out".into());
        }
        if !self.modules.contains_key("key_drift") {
            self.module(
                "key_drift",
                "cv_mix",
                5,
                json!({"level1": 1.0, "level2": 1.0, "level3": 0.0}),
                "Key change · generative arp",
            );
            self.con("drift_q", "out", "key_drift", "in1");
            let kl = self.lane_out("key");
            self.con("lanes", &kl, "key_drift", "in2");
        }
        ("key_drift".into(), "out".into())
    }

    fn lane_out(&self, name: &str) -> String {
        format!(
            "out{}",
            self.lanes.iter().position(|l| *l == name).unwrap() + 1
        )
    }
    fn note(&mut self, s: impl Into<String>) {
        self.notes.push(s.into());
    }

    // ------------------------------------------------------------ scores
    fn compose(&mut self, c: &mut Ctx) {
        let div = c.g.div;
        let spb = 16 / div; // seq_music steps per bar
        let bpc = if div == 1 { 1 } else { 2 }; // bars per chord
        let chord_at = |s: i64| ((s / spb) / bpc % 4) as usize;
        let rng = &mut c.rng;

        // ---- drums (seq_drums, 4 bars of 16ths)
        let (mut kick, mut clap, mut hats, mut ohat) = (vec![], vec![], vec![], vec![]);
        match c.g.drums {
            "boombap" | "broken" | "twostep" => {
                let pats: &[&[i64]] = match c.g.drums {
                    "boombap" => &[
                        &[0, 10],
                        &[0, 7, 10],
                        &[0, 3, 10],
                        &[0, 10, 13],
                        &[0, 8, 10],
                    ],
                    "broken" => &[
                        &[0, 10],
                        &[0, 6, 10],
                        &[0, 3, 8, 11],
                        &[0, 7, 13],
                        &[0, 10, 14],
                    ],
                    _ => &[&[0, 10], &[0, 7, 10], &[0, 11], &[0, 6, 10], &[0, 3, 10]],
                };
                let (a, b) = (rng.pick(pats), rng.pick(pats));
                for bar in 0..4 {
                    for &s in if bar % 2 == 0 { a } else { b } {
                        kick.push(hit(bar * 16 + s, if s == 0 { 1.0 } else { 0.8 }));
                    }
                }
                self.note(format!(
                    "{} kick",
                    match c.g.drums {
                        "boombap" => "boom-bap",
                        "broken" => "broken-beat",
                        _ => "2-step",
                    }
                ));
            }
            "none" => {}
            "ambient" => {
                let style = rng.pick(&["heartbeat", "pulse", "lone"]);
                for bar in 0..4 {
                    match style {
                        "heartbeat" if bar % 2 == 0 => {
                            kick.push(hit(bar * 16, 0.7));
                            kick.push(hit(bar * 16 + 3, 0.45));
                        }
                        "pulse" => kick.push(hit(bar * 16, 0.6)),
                        "lone" if bar == 0 => kick.push(hit(0, 0.55)),
                        _ => {}
                    }
                }
                self.note(format!("{style} kick"));
            }
            _ => {
                let rock = c.g.drums == "rock" && rng.chance(0.4);
                for bar in 0..4 {
                    for s in if rock {
                        vec![0, 8, 10]
                    } else {
                        vec![0, 4, 8, 12]
                    } {
                        kick.push(hit(bar * 16 + s, if s == 10 { 0.75 } else { 1.0 }));
                    }
                }
                if rock {
                    self.note("rock kick");
                }
                if rng.chance(0.4) {
                    kick.retain(|k| k["step"] != 60);
                    kick.push(hit(62, 0.6));
                    kick.push(st(63, &[0], 0.5, 0.4, 0.6, false));
                }
                if c.g.drums == "techno" && rng.chance(0.5) {
                    kick.push(hit(30, 0.45));
                    kick.push(hit(62, 0.45));
                }
            }
        }
        if c.g.drums == "dub" {
            let rims: &[&[i64]] = &[
                &[6, 14],
                &[3, 10, 14],
                &[6, 11, 14],
                &[2, 8, 14],
                &[5, 10, 13],
            ];
            let rim = rng.pick(rims);
            for bar in 0..4 {
                for (k, &s) in rim.iter().enumerate() {
                    clap.push(st(
                        bar * 16 + s,
                        &[0],
                        rng.pick(&[0.5, 0.6, 0.7]),
                        0.3,
                        if k == 0 { 1.0 } else { 0.85 },
                        false,
                    ));
                }
            }
        } else if c.g.drums != "ambient" && c.g.drums != "none" {
            // backbeat by genre: techno keeps it to a quiet rim (or nothing), house a softer clap
            let backbeat = match parent(c.genre) {
                "Techno" => {
                    if rng.chance(0.5) {
                        0.0
                    } else {
                        0.5
                    }
                }
                "Deep House" => 0.8,
                _ => 0.95,
            };
            if backbeat > 0.0 {
                for bar in 0..4 {
                    clap.push(hit(bar * 16 + 4, backbeat));
                    clap.push(hit(bar * 16 + 12, backbeat));
                }
            }
            if rng.chance(0.5) {
                clap.push(st(63, &[0], 0.45, 0.3, 0.5, false));
            }
            if rng.chance(0.35) {
                for bar in [1, 3] {
                    clap.push(st(bar * 16 + 7, &[0], 0.35, 0.3, 0.5, false));
                }
            }
            if c.g.drums == "twostep" || c.g.drums == "broken" {
                for bar in 0..4 {
                    clap.push(st(
                        bar * 16 + rng.pick(&[9, 14, 15]),
                        &[0],
                        0.35,
                        0.3,
                        0.6,
                        false,
                    ));
                }
            }
        }
        let hstyle = rng.pick(c.g.hats);
        let acc = rng.pick(&[
            [0.35, 0.2, 0.75, 0.25],
            [0.6, 0.25, 0.45, 0.25],
            [0.3, 0.3, 0.8, 0.3],
            [0.5, 0.2, 0.6, 0.35],
            [0.7, 0.3, 0.5, 0.3],
        ]);
        let hname = match hstyle {
            "16" => {
                for s in 0..64 {
                    hats.push(st(
                        s,
                        &[0],
                        acc[(s % 4) as usize],
                        0.3,
                        if s % 4 == 1 { 0.7 } else { 1.0 },
                        false,
                    ));
                }
                "16ths".to_string()
            }
            "8" => {
                for s in (0..64).step_by(2) {
                    hats.push(hit(s, if s % 4 == 2 { 0.8 } else { 0.5 }));
                }
                "8ths".into()
            }
            "off8" => {
                for s in (2..64).step_by(4) {
                    hats.push(hit(s, 0.8));
                }
                for s in (1..64).step_by(4) {
                    hats.push(st(s, &[0], 0.35, 0.3, 0.5, false));
                }
                "offbeats".into()
            }
            "shuffle" => {
                for b in 0..8 {
                    for (k, s) in [0, 3, 6].iter().enumerate() {
                        hats.push(hit(b * 8 + s, [0.7, 0.45, 0.55][k]));
                    }
                }
                "3-3-2 shuffle".into()
            }
            "sparse" => {
                for s in (2..64).step_by(4) {
                    hats.push(st(s, &[0], 0.4, 0.3, 0.35, false));
                }
                "sparse".into()
            }
            "none" => "none".into(),
            _ => {
                let k = rng.pick(&[5, 7, 9, 11]);
                let rot = rng.int(0, 15);
                for b in 0..4 {
                    for (i, s) in euclid(k, 16, rot).into_iter().enumerate() {
                        hats.push(hit(b * 16 + s, if i % 2 == 0 { 0.75 } else { 0.4 }));
                    }
                }
                format!("E({k},16)")
            }
        };
        self.note(format!("hats {hname}"));
        match rng.pick(c.g.ohat) {
            "off" => {
                for s in (2..64).step_by(4) {
                    ohat.push(hit(s, 0.85));
                }
            }
            "half" => {
                for b in 0..4 {
                    ohat.push(hit(b * 16 + 6, 0.85));
                    ohat.push(hit(b * 16 + 14, 0.85));
                }
            }
            "sparse" => {
                for b in [1, 3] {
                    ohat.push(hit(b * 16 + 14, 0.6));
                }
            }
            _ => {}
        }
        if !c.clap {
            clap.clear();
        }
        if !c.ohat {
            ohat.clear();
        }

        // ---- chords
        let ch = &c.chords;
        let mut chords = vec![];
        let rhythm = rng.pick(c.g.rhythm);
        // percussive designs (low-pass gates, struck resonators, plucks) die away inside a held chord: strike each bar
        let rhythm = if rhythm == "held" && instruments::design(c.voice).is_some_and(|x| !x.sustain)
        {
            "restrike"
        } else {
            rhythm
        };
        match rhythm {
            "held" | "restrike" => {
                for (ci, chord) in ch.iter().enumerate() {
                    let o = ci as i64 * spb * bpc;
                    if rhythm == "restrike" {
                        for bar in 0..bpc {
                            chords.extend(held(o + bar * spb, spb, chord, 0.9, 0.85));
                        }
                    } else {
                        chords.extend(held(o, spb * bpc, chord, 1.0, 0.95));
                    }
                }
                self.note(if rhythm == "held" {
                    "held chords"
                } else {
                    "chords re-strike every bar"
                });
            }
            "broken" => {
                // broken chords, one note at a time across each two-bar chord (8th-note grid), piano-like
                let figs: [&[(i64, usize)]; 4] = [
                    &[(0, 0), (2, 1), (4, 2), (6, 3), (9, 2), (12, 1)],
                    &[(0, 0), (3, 2), (6, 1), (8, 3), (11, 2)],
                    &[(0, 1), (2, 2), (5, 3), (8, 0), (10, 2), (13, 1)],
                    &[(0, 0), (4, 1), (6, 2), (8, 3), (12, 2)],
                ];
                let fig = figs[rng.idx(figs.len())];
                let span = spb * bpc;
                for ci in 0..4 {
                    for &(s, v) in fig {
                        chords.push(st(
                            ci as i64 * span + s * span / 16,
                            &[ch[ci][v.min(ch[ci].len() - 1)]],
                            rng.pick(&[0.55, 0.7, 0.85]),
                            0.9,
                            1.0,
                            false,
                        ));
                    }
                }
                self.note("broken-chord piano figure");
            }
            "pulse8" => {
                for s in (0..64).step_by(2) {
                    chords.push(st(
                        s,
                        &ch[chord_at(s)],
                        if s % 4 == 0 { 0.9 } else { 0.65 },
                        0.45,
                        1.0,
                        false,
                    ));
                }
                self.note("chords pulse in 8ths");
            }
            _ => {
                let pool: Vec<Vec<(i64, f64)>> = if rhythm == "syncop" {
                    (0..3)
                        .map(|_| {
                            let n = rng.int(2, 4) as usize;
                            let weights = [
                                0.2, 0.3, 1.0, 0.8, 0.3, 0.6, 1.0, 0.7, 0.2, 0.5, 1.0, 0.8, 0.3,
                                0.6, 0.9, 0.5,
                            ];
                            let mut on: Vec<i64> = vec![];
                            while on.len() < n {
                                let s = rng.weighted(
                                    &(0..16).map(|i| (i as i64, weights[i])).collect::<Vec<_>>(),
                                ) * spb
                                    / 16;
                                if !on.contains(&s) {
                                    on.push(s);
                                }
                            }
                            on.sort();
                            on.into_iter()
                                .map(|s| (s, rng.pick(&[0.6, 0.75, 0.9])))
                                .collect()
                        })
                        .collect()
                } else if div == 2 {
                    vec![
                        vec![(1, 1.0), (4, 0.6), (6, 0.8)],
                        vec![(1, 1.0), (3, 0.6), (6, 0.8)],
                        vec![(2, 1.0), (5, 0.7)],
                        vec![(1, 1.0), (6, 0.7), (7, 0.5)],
                    ]
                } else {
                    vec![
                        vec![(2, 0.9), (5, 0.6), (10, 0.85), (13, 0.55)],
                        vec![(3, 0.9), (6, 0.6), (11, 0.8), (14, 0.6)],
                        vec![(0, 0.85), (3, 0.6), (6, 0.8), (10, 0.6)],
                        vec![(2, 0.9), (7, 0.7), (10, 0.85)],
                        vec![(3, 0.85), (10, 0.6)],
                        vec![(0, 0.9), (7, 0.55), (10, 0.7)],
                        vec![(4, 0.8), (11, 0.7), (14, 0.6)],
                    ]
                };
                let (a, b) = (
                    pool[rng.idx(pool.len())].clone(),
                    pool[rng.idx(pool.len())].clone(),
                );
                let sparse = parent(c.genre) == "Techno" && rng.chance(0.3);
                for bar in 0..(64 / spb) {
                    if sparse && bar % 2 == 1 {
                        continue;
                    }
                    for &(s, v) in if bar % 2 == 0 { &a } else { &b } {
                        let step = bar * spb + s;
                        chords.push(st(step, &ch[chord_at(step)], v, 0.3, 1.0, false));
                    }
                }
                let pat: String = (0..spb)
                    .map(|k| if a.iter().any(|x| x.0 == k) { 'x' } else { '.' })
                    .collect();
                self.note(format!(
                    "{}chord rhythm {pat}",
                    if sparse { "sparse " } else { "" }
                ));
            }
        }

        // ---- bass
        let roots = &c.roots;
        let mut bass = vec![];
        let bstyle = c.bass_style;
        if bstyle == "drone" {
            let split = rng.chance(0.5);
            for (ci, root) in roots.iter().enumerate() {
                let (o, span) = (ci as i64 * spb * bpc, spb * bpc);
                if split {
                    bass.extend(held(o, span / 2, &[*root], 0.9, 0.95));
                    bass.extend(held(
                        o + span / 2,
                        span / 2,
                        &[*root + rng.pick(&[7, 12, 0])],
                        0.8,
                        0.95,
                    ));
                } else {
                    bass.extend(held(o, span, &[*root], 0.9, 0.95));
                }
            }
            self.note(if split {
                "drone moves within the chord"
            } else {
                "held drone"
            });
        } else {
            let templates: Vec<Vec<(i64, i64, f64, f64)>> = match bstyle {
                "house" => vec![
                    vec![
                        (2, 0, 0.95, 0.55),
                        (5, 0, 0.6, 0.3),
                        (7, 12, 0.7, 0.25),
                        (10, 0, 0.9, 0.5),
                        (13, 7, 0.65, 0.3),
                        (14, 0, 0.75, 0.4),
                    ],
                    vec![
                        (3, 0, 0.9, 0.5),
                        (6, 0, 0.6, 0.3),
                        (10, 12, 0.8, 0.3),
                        (11, 0, 0.7, 0.4),
                        (14, 7, 0.7, 0.3),
                    ],
                    vec![
                        (2, 0, 0.95, 0.7),
                        (6, 0, 0.7, 0.4),
                        (9, 7, 0.6, 0.3),
                        (10, 0, 0.9, 0.5),
                        (14, 12, 0.6, 0.25),
                    ],
                    vec![
                        (2, 0, 0.9, 0.6),
                        (6, 0, 0.9, 0.6),
                        (10, 0, 0.9, 0.6),
                        (14, 12, 0.8, 0.4),
                    ],
                ],
                "pulse" => vec![
                    vec![
                        (1, 0, 0.9, 0.6),
                        (3, 0, 0.7, 0.6),
                        (5, 0, 0.9, 0.6),
                        (7, 0, 0.7, 0.6),
                    ],
                    vec![
                        (1, 0, 0.9, 0.6),
                        (3, 0, 0.7, 0.5),
                        (5, 0, 0.9, 0.6),
                        (6, 7, 0.6, 0.4),
                        (7, 0, 0.7, 0.5),
                    ],
                    vec![(1, 0, 0.9, 0.9), (5, 0, 0.9, 0.9)],
                    vec![
                        (1, 0, 0.9, 0.6),
                        (4, 0, 0.6, 0.4),
                        (5, 0, 0.9, 0.6),
                        (7, 12, 0.6, 0.3),
                    ],
                ],
                "roll" => vec![
                    (0..16)
                        .filter(|s| s % 4 != 0)
                        .map(|s| (s, 0, if s % 4 == 2 { 0.9 } else { 0.65 }, 0.4))
                        .collect(),
                    (0..16)
                        .filter(|s| s % 4 >= 2)
                        .map(|s| (s, 0, 0.85, 0.4))
                        .collect(),
                    (0..16)
                        .filter(|s| s % 4 != 0)
                        .map(|s| (s, if s % 4 == 3 { 12 } else { 0 }, 0.8, 0.35))
                        .collect(),
                    (0..16)
                        .filter(|s| s % 2 == 1)
                        .map(|s| (s, 0, 0.85, 0.45))
                        .collect(),
                ],
                "lofi" => vec![
                    vec![(0, 0, 0.9, 0.9), (10, 0, 0.7, 0.5), (14, 7, 0.6, 0.4)],
                    vec![(0, 0, 0.9, 0.6), (6, 0, 0.6, 0.4), (10, 7, 0.7, 0.5)],
                    vec![(0, 0, 0.9, 0.8), (3, 12, 0.5, 0.25), (10, 0, 0.8, 0.6)],
                    vec![
                        (0, 0, 0.9, 0.5),
                        (8, 7, 0.6, 0.4),
                        (10, 0, 0.8, 0.5),
                        (14, 12, 0.5, 0.3),
                    ],
                ],
                "octave" => vec![
                    (0..16)
                        .step_by(2)
                        .map(|s| {
                            (
                                s,
                                if s % 4 == 2 { 12 } else { 0 },
                                if s % 4 == 0 { 0.85 } else { 0.65 },
                                0.45,
                            )
                        })
                        .collect(),
                    (0..16)
                        .filter(|s| s % 4 != 1)
                        .map(|s| (s, 0, if s % 4 == 0 { 0.85 } else { 0.6 }, 0.4))
                        .collect(),
                    (0..16)
                        .step_by(2)
                        .map(|s| (s, if s % 8 == 6 { 12 } else { 0 }, 0.8, 0.45))
                        .collect(),
                ],
                "broken" => vec![
                    vec![
                        (0, 0, 0.9, 0.7),
                        (6, 0, 0.7, 0.4),
                        (10, 7, 0.7, 0.4),
                        (11, 0, 0.8, 0.5),
                    ],
                    vec![
                        (0, 0, 0.9, 0.5),
                        (3, 0, 0.6, 0.3),
                        (8, 12, 0.6, 0.3),
                        (10, 0, 0.8, 0.6),
                    ],
                    vec![(0, 0, 0.9, 1.0), (7, 7, 0.6, 0.4), (14, 0, 0.7, 0.3)],
                ],
                "garage" => vec![
                    vec![
                        (0, 0, 0.9, 0.4),
                        (3, 0, 0.7, 0.3),
                        (6, 12, 0.6, 0.25),
                        (10, 0, 0.85, 0.4),
                        (13, 7, 0.6, 0.3),
                    ],
                    vec![
                        (0, 0, 0.9, 0.6),
                        (7, 0, 0.7, 0.3),
                        (10, 0, 0.8, 0.5),
                        (14, 12, 0.6, 0.25),
                    ],
                    vec![
                        (2, 0, 0.9, 0.4),
                        (6, 0, 0.7, 0.3),
                        (9, 7, 0.7, 0.3),
                        (12, 0, 0.8, 0.4),
                    ],
                ],
                "sub" => vec![
                    vec![(0, 0, 0.9, 0.9)],
                    vec![(0, 0, 0.9, 0.5), (10, 0, 0.8, 0.5)],
                    vec![(2, 0, 0.9, 0.8), (10, 0, 0.8, 0.6)],
                ],
                _ => {
                    // generative: weighted onsets that avoid the strongest kick positions, root/fifth/octave notes
                    (0..2)
                        .map(|_| {
                            let w = [
                                0.15, 0.3, 1.0, 0.6, 0.2, 0.5, 0.9, 0.7, 0.15, 0.45, 1.0, 0.6, 0.2,
                                0.55, 0.9, 0.6,
                            ];
                            let n = rng.int(3, 6) as usize;
                            let mut on: Vec<i64> = vec![];
                            while on.len() < n {
                                let s = rng.weighted(
                                    &(0..16).map(|i| (i as i64, w[i])).collect::<Vec<_>>(),
                                );
                                if !on.contains(&s) {
                                    on.push(s);
                                }
                            }
                            on.sort();
                            on.iter()
                                .map(|&s| {
                                    (
                                        s,
                                        rng.weighted(&[(0, 8.0), (12, 0.8), (7, 1.2)]),
                                        rng.pick(&[0.65, 0.8, 0.95]),
                                        rng.pick(&[0.3, 0.45, 0.6]),
                                    )
                                })
                                .collect()
                        })
                        .collect()
                }
            };
            let ti = rng.idx(templates.len());
            let (a, b) = (
                templates[ti].clone(),
                templates[rng.idx(templates.len())].clone(),
            );
            let slides = matches!(bstyle, "house" | "lofi" | "gen" | "garage" | "broken");
            let steps = mode_steps(c.m.mode);
            let in_key = |x: i64| steps.contains(&(x - c.tonic).rem_euclid(12));
            for bar in 0..(64 / spb) {
                let pat = if bar % 2 == 0 { &a } else { &b };
                let mut octaves = 0;
                for (j, &(s, iv, v, gt)) in pat.iter().enumerate() {
                    if s >= spb {
                        continue;
                    }
                    let step = bar * spb + s;
                    let ci = chord_at(step);
                    // one octave jump per bar at most; the odd fifth stays off the downbeats
                    let iv = if iv == 12 {
                        octaves += 1;
                        if octaves > 1 {
                            0
                        } else {
                            12
                        }
                    } else if iv == 0 && s % 4 != 0 && rng.chance(0.08) {
                        7
                    } else {
                        iv
                    };
                    // the last note before a chord change can step into the next root from a scale tone
                    let next = chord_at((bar + 1) * spb);
                    let last = pat[j + 1..].iter().all(|x| x.0 >= spb);
                    let note = if last && next != ci && s >= spb / 2 && rng.chance(0.4) {
                        let to = roots[next];
                        [to - 1, to + 1, to - 2, to + 2]
                            .into_iter()
                            .find(|&x| in_key(x) && x != roots[ci])
                            .unwrap_or(roots[ci] + iv)
                    } else {
                        roots[ci] + iv
                    };
                    // a note directly followed by another sometimes slides into it
                    let glide_on = slides && pat.iter().any(|x| x.0 == s + 1) && rng.chance(0.3);
                    bass.push(st(
                        step,
                        &[note],
                        v,
                        if glide_on { 1.0 } else { gt },
                        1.0,
                        glide_on,
                    ));
                }
            }
            self.note(if bstyle == "gen" {
                "generated bass line".to_string()
            } else {
                format!("{bstyle} bass {}", (b'A' + ti as u8) as char)
            });
        }

        // ---- melody (lead) and counter-line, all from chord tones
        let tones = |ci: usize, lo: i64, hi: i64| -> Vec<i64> {
            let mut t: Vec<i64> = ch[ci]
                .iter()
                .flat_map(|n| (-2..3).map(move |k| n + 12 * k))
                .filter(|n| (lo..=hi).contains(n))
                .collect();
            t.sort();
            t.dedup();
            t
        };
        let mut lead = vec![];
        match c.lead {
            "arp" => {
                let orders: [&[usize]; 6] = [
                    &[0, 1, 2, 3],
                    &[0, 1, 2, 3, 2, 1],
                    &[0, 2, 1, 3],
                    &[3, 2, 1, 0],
                    &[0, 1, 2, 3, 4, 3, 2, 1],
                    &[0, 2, 4, 1, 3],
                ];
                let order = orders[rng.idx(orders.len())];
                let rate = if div == 1 { rng.pick(&[1, 1, 2]) } else { 1 };
                let (lo, hi) = rng.pick(&[(-3, 12), (0, 14), (-5, 9)]);
                let gate = rng.pick(&[0.3, 0.45, 0.6]);
                let rest = rng.pick(&[0.0, 0.1, 0.2]);
                for (k, s) in (0..64).step_by(rate as usize).enumerate() {
                    if rng.chance(rest) {
                        continue;
                    }
                    let t = tones(chord_at(s), lo, hi);
                    lead.push(st(
                        s,
                        &[t[order[k % order.len()] % t.len()]],
                        if s % 4 == 0 { 0.9 } else { 0.6 },
                        gate,
                        1.0,
                        false,
                    ));
                }
                self.note(format!(
                    "arp {} in {}",
                    order
                        .iter()
                        .map(|x| x.to_string())
                        .collect::<Vec<_>>()
                        .join("-"),
                    if rate == 1 && div == 1 {
                        "16ths"
                    } else {
                        "8ths"
                    }
                ));
            }
            "riff" => {
                let n = rng.int(4, 7) as usize;
                let mut on: Vec<i64> = rng.sample(16, n).into_iter().map(|x| x as i64).collect();
                on.sort();
                let shape: Vec<usize> = (0..n).map(|_| rng.idx(4)).collect();
                for bar in 0..4 {
                    for (k, &s) in on.iter().enumerate() {
                        if bar == 3 && k == n - 1 {
                            continue;
                        }
                        let step = bar * 16 + s;
                        let t = tones(chord_at(step), -2, 10);
                        lead.push(st(
                            step,
                            &[t[shape[k] % t.len()]],
                            if k == 0 { 0.9 } else { 0.7 },
                            0.35,
                            1.0,
                            false,
                        ));
                    }
                }
                self.note(format!("{n}-note riff"));
            }
            "hook" => {
                // a 3–5 note motif over two chords, developed over the next two: the same rhythm carried onto the new
                // harmony, inverted, or thinned out. Pitches come from the mode bent to each chord; strong beats and
                // phrase ends land on chord tones (3rd, 5th, 7th or 9th more often than the root), weak beats may pass
                // through scale tones, and no note sits a semitone above a chord tone.
                let span = 2 * spb * bpc;
                let beat = 4 / div;
                let n = rng.weighted(&[(3usize, 3.0), (4, 3.0), (5, 1.0)]);
                let mut on: Vec<i64> = vec![];
                for _ in 0..64 {
                    if on.len() == n {
                        break;
                    }
                    let slots: Vec<(i64, f64)> = (0..span * 3 / 4)
                        .map(|t| {
                            let w = if t == 0 {
                                0.5
                            } else if t % beat == 0 {
                                0.7
                            } else if t % 2 == 0 {
                                1.2
                            } else {
                                0.5
                            };
                            (t, w)
                        })
                        .collect();
                    let t = rng.weighted(&slots);
                    if on.iter().all(|&o| (o - t).abs() >= 2) {
                        on.push(t);
                    }
                }
                on.sort();
                let n = on.len();
                let lens: Vec<i64> = (0..n)
                    .map(|k| match on.get(k + 1) {
                        Some(&nx) => (nx - on[k] - rng.pick(&[0, 0, 1])).max(1),
                        None => rng.int(beat, 2 * beat).min(span - on[k] - 1).max(1),
                    })
                    .collect();
                let mut offs = vec![0i64];
                for _ in 1..n {
                    let d = rng.weighted(&[
                        (1, 3.0),
                        (-1, 3.0),
                        (2, 1.5),
                        (-2, 1.5),
                        (0, 1.0),
                        (3, 0.4),
                        (-3, 0.4),
                    ]);
                    offs.push((offs.last().unwrap() + d).clamp(-4, 4));
                }
                let steps = mode_steps(c.m.mode);
                let tonic = c.tonic;
                // the mode bent to chord `ci` (a chromatic chord tone replaces its scale neighbours), and the chord's
                // pitch classes
                let chord_scale = |ci: usize| -> (Vec<i64>, Vec<i64>) {
                    let cpc: Vec<i64> = ch[ci].iter().map(|x| x.rem_euclid(12)).collect();
                    let mut pcs: Vec<i64> =
                        steps.iter().map(|x| (tonic + x).rem_euclid(12)).collect();
                    for &q in &cpc {
                        if !pcs.contains(&q) {
                            pcs.retain(|&p| {
                                (p - q).rem_euclid(12) != 1 && (q - p).rem_euclid(12) != 1
                            });
                            pcs.push(q);
                        }
                    }
                    let sc = (-12..=26)
                        .filter(|x: &i64| pcs.contains(&x.rem_euclid(12)))
                        .collect();
                    (sc, cpc)
                };
                let home = rng.int(4, 9);
                let anchor = |rng: &mut Rng, ci: usize, home: i64| -> i64 {
                    let (sc, cpc) = chord_scale(ci);
                    let root = roots[ci].rem_euclid(12);
                    let pool: Vec<(i64, f64)> = sc
                        .iter()
                        .filter(|x| cpc.contains(&x.rem_euclid(12)) && (*x - home).abs() <= 5)
                        .map(|&x| {
                            let w = 1.0 / (1.0 + (x - home).abs() as f64).powi(2);
                            (x, if x.rem_euclid(12) == root { w * 0.3 } else { w })
                        })
                        .collect();
                    rng.weighted(&pool)
                };
                let answer = rng.weighted(&[("carried", 3.0), ("inverted", 1.5), ("thinned", 1.5)]);
                let liquid = c.texture == "Liquid";
                for phrase in 0..2i64 {
                    let base = phrase * span;
                    let ci0 = chord_at(base);
                    let (sc0, _) = chord_scale(ci0);
                    let lift = if phrase == 1 {
                        rng.pick(&[0, 0, 2, -2])
                    } else {
                        0
                    };
                    let a = anchor(rng, ci0, home + lift);
                    let a0 = sc0.iter().position(|&x| x == a).unwrap_or(0) as i64;
                    let skip = if phrase == 1 && answer == "thinned" && n > 2 {
                        Some(rng.int(1, n as i64 - 1) as usize)
                    } else {
                        None
                    };
                    let mut prev: Option<(usize, i64)> = None;
                    let mut k = 0;
                    while k < n {
                        if Some(k) == skip {
                            k += 1;
                            continue;
                        }
                        let t = base + on[k];
                        // a thinned note's time goes to the note before it
                        let len = if skip == Some(k + 1) && k + 2 <= n {
                            on.get(k + 2)
                                .map_or(lens[k + 1] + on[k + 1] - on[k], |&x| x - on[k] - 1)
                        } else {
                            lens[k]
                        };
                        let off = if phrase == 1 && answer == "inverted" {
                            -offs[k]
                        } else {
                            offs[k]
                        };
                        let p = sc0[(a0 + off).clamp(0, sc0.len() as i64 - 1) as usize];
                        let ci = chord_at(t);
                        let (sc, cpc) = chord_scale(ci);
                        let p = *sc.iter().min_by_key(|&&x| ((x - p).abs(), x)).unwrap();
                        let last = (k + 1..n).all(|j| Some(j) == skip);
                        let strong = on[k] % beat == 0 || last;
                        let root = roots[ci].rem_euclid(12);
                        let note = if strong {
                            let mut tones: Vec<i64> = sc
                                .iter()
                                .copied()
                                .filter(|x| cpc.contains(&x.rem_euclid(12)))
                                .collect();
                            tones.sort_by_key(|&x| ((x - p).abs(), x));
                            match tones.as_slice() {
                                [r, alt, ..] if r.rem_euclid(12) == root && rng.chance(0.6) => *alt,
                                [x, ..] => *x,
                                [] => p,
                            }
                        } else {
                            let avoid = |x: i64| {
                                !cpc.contains(&x.rem_euclid(12))
                                    && cpc.iter().any(|&q| (x - q).rem_euclid(12) == 1)
                            };
                            if avoid(p) {
                                sc.iter()
                                    .rev()
                                    .copied()
                                    .find(|&x| x < p && !avoid(x))
                                    .unwrap_or(p)
                            } else {
                                p
                            }
                        };
                        let vel = if strong { 0.85 } else { 0.68 } + rng.uni(-0.06, 0.06);
                        if let (true, Some((i, q))) = (liquid, prev) {
                            if (note - q).abs() <= 2 && rng.chance(0.5) {
                                if let Some(x) = lead.get_mut(i) {
                                    x["slide"] = json!(true);
                                    x["gate"] = json!(1.0);
                                }
                            }
                        }
                        lead.extend(held(t, len, &[note], vel, 0.85));
                        prev = Some((lead.len() - 1, note));
                        k += 1;
                    }
                }
                self.note(format!("hook: {n}-note motif, answer {answer}"));
            }
            "bells" => {
                // one three-note figure (fifth → ninth → third of each chord), repeated and carried through the changes
                let order = rng.pick(&[[2usize, 3, 1], [1, 2, 0], [3, 1, 2]]);
                for ci in 0..4 {
                    let t = tones(ci, 2, 14);
                    for (k, &o) in order.iter().enumerate() {
                        lead.extend(held(
                            ci as i64 * 16 + k as i64 * 3,
                            2,
                            &[t[o % t.len()]],
                            [0.8, 0.6, 0.7][k],
                            0.9,
                        ));
                    }
                }
                self.note("bell figure");
            }
            "counter" => {} // the counter-line below takes the melody slot
            _ => {}
        }
        // counter-line: long guide tones (3rd/7th) moving smoothly, one or two per chord
        let mut counter = vec![];
        if c.counter || c.lead == "counter" {
            let per = rng.int(1, 2);
            let mut prev = 4;
            for ci in 0..4 {
                let span = spb * bpc;
                let t = tones(ci, -3, 9);
                for k in 0..per {
                    let n = *t.iter().min_by_key(|&&n| ((n - prev).abs(), n)).unwrap();
                    let n = if k == 1 {
                        *t.iter()
                            .filter(|&&x| x != n)
                            .min_by_key(|&&x| (x - n).abs())
                            .unwrap_or(&n)
                    } else {
                        n
                    };
                    let len = span / per;
                    counter.extend(held(ci as i64 * span + k * len, len, &[n], 0.8, 0.95));
                    prev = n;
                }
            }
            self.note("counter-line on guide tones");
        }
        let mut melody_track = if c.lead == "counter" {
            std::mem::take(&mut counter)
        } else {
            lead
        };

        // ---- percussion (seq_perc, 16ths) + Pulsing chop gates
        let mut percs: Vec<Vec<Value>> = vec![vec![], vec![], vec![]];
        for (slot, kind) in [c.perc, c.perc2].into_iter().enumerate() {
            let Some(kind) = kind else { continue };
            let k = rng.pick(&[3, 5, 7]);
            let rot = rng.int(0, 15);
            for b in 0..4 {
                for (i, s) in euclid(k, 16, rot).into_iter().enumerate() {
                    percs[slot].push(st(
                        b * 16 + s,
                        &[0],
                        rng.pick(&[0.5, 0.7, 0.9]),
                        0.3,
                        if i == 0 {
                            1.0
                        } else {
                            rng.pick(&[1.0, 0.8, 0.6])
                        },
                        false,
                    ));
                }
            }
            self.note(format!("{kind} E({k},16)"));
        }
        if c.texture == "Pulsing" {
            let pats: [&[i64]; 5] = [
                &[0, 2, 4, 6, 8, 10, 12, 14],
                &[0, 3, 6, 8, 11, 14],
                &[0, 2, 3, 6, 8, 10, 11, 14],
                &[0, 1, 4, 5, 8, 9, 12, 13],
                &[0, 3, 4, 7, 8, 11, 12, 15],
            ];
            let p = pats[rng.idx(pats.len())];
            for b in 0..4 {
                for &s in p {
                    percs[2].push(st(b * 16 + s, &[0], 1.0, 0.5, 1.0, false));
                }
            }
            self.note("trance-gate pattern");
        }

        // ---- variation inside the 4-bar loop: the sequencers re-roll step probability on every pass, so optional
        // notes make each pass a little different (ghost bass notes, an arp that breathes, hat rolls)
        let used: Vec<i64> = bass.iter().filter_map(|x| x["step"].as_i64()).collect();
        let held_into = |s: i64| {
            bass.iter().any(|x| {
                x["step"].as_i64() == Some(s - 1)
                    && (x["slide"] == json!(true) || x["gate"].as_f64().unwrap_or(0.5) >= 0.9)
            })
        };
        if !matches!(c.bass_style, "drone" | "sub") && !bass.is_empty() {
            let mut ghosts = vec![];
            for st_ in 0..64i64 {
                let pos = st_ % spb;
                let slot = if div == 1 {
                    matches!(pos, 3 | 7 | 11 | 15)
                } else {
                    matches!(pos, 3 | 7)
                };
                if slot
                    && !used.contains(&st_)
                    && !used.contains(&(st_ + 1))
                    && !held_into(st_)
                    && rng.chance((0.7 * c.ctl.complexity).min(0.9))
                {
                    let up = if rng.chance(0.3) { 12 } else { 0 };
                    let mut g = st(st_, &[roots[chord_at(st_)] + up], 0.45, 0.25, 1.0, false);
                    g["probability"] = json!(rng.pick(&[0.25, 0.35, 0.5]));
                    ghosts.push(g);
                }
            }
            if !ghosts.is_empty() {
                self.note(format!("{} ghost bass notes", ghosts.len()));
            }
            bass.extend(ghosts);
            bass.sort_by_key(|x| x["step"].as_i64());
        }
        if matches!(c.lead, "arp" | "riff") {
            let beat = 4 / div;
            for x in melody_track.iter_mut() {
                let s_ = x["step"].as_i64().unwrap_or(0);
                if s_ % beat != 0 && x["slide"] != json!(true) && x.get("probability").is_none() {
                    x["probability"] = json!(r3(if c.lead == "arp" {
                        1.0 - 0.4 * c.ctl.complexity
                    } else {
                        1.0 - 0.2 * c.ctl.complexity
                    }));
                }
            }
        }
        if !hats.is_empty() {
            // a roll on one 16th before a backbeat, some passes only
            for b in 0..4 {
                if rng.chance(0.8 * c.ctl.complexity) {
                    let at = b * 16 + rng.pick(&[11, 15]);
                    match hats.iter_mut().find(|x| x["step"].as_i64() == Some(at)) {
                        Some(x) => {
                            x["ratchet"] = json!(rng.pick(&[2, 3]));
                            x["probability"] = json!(0.5);
                        }
                        None => {
                            let mut r = st(at, &[0], 0.4, 0.3, 0.5, false);
                            r["ratchet"] = json!(2);
                            hats.push(r);
                        }
                    }
                }
            }
            hats.sort_by_key(|x| x["step"].as_i64());
        }

        // ---- song form (64-bar loop on seq_drums lanes):
        //   intro 0–8 · build 8–16 (riser, fill, kick drops out for the last two beats) · drop 16–32 ·
        //   breakdown 32–40 (no kick or bass, related key, riser, fill) · drop 40–56 · outro 56–64
        let h = |pts: &[(f64, f64)]| -> Vec<(f64, f64, &'static str)> {
            pts.iter().map(|&(b, x)| (b, x, "hold")).collect()
        };
        let ambient = parent(c.genre) == "Ambient";
        let break_lead = matches!(c.lead, "hook" | "bells" | "counter");
        let kick_l = if ambient {
            h(&[
                (0.0, 0.0),
                (8.0, 6.0),
                (32.0, 0.0),
                (40.0, 6.0),
                (56.0, 0.0),
                (64.0, 0.0),
            ])
        } else {
            h(&[
                (0.0, 10.0),
                (15.5, 0.0),
                (16.0, 10.0),
                (32.0, 0.0),
                (40.0, 10.0),
                (64.0, 10.0),
            ])
        };
        let bass_l = if c.bass_style == "drone" {
            h(&[(0.0, 10.0), (64.0, 10.0)])
        } else {
            h(&[
                (0.0, 0.0),
                (8.0, 10.0),
                (15.5, 0.0),
                (16.0, 10.0),
                (32.0, 0.0),
                (40.0, 10.0),
                (56.0, 0.0),
                (64.0, 0.0),
            ])
        };
        let lead_l = h(&[
            (0.0, 0.0),
            (16.0, 10.0),
            (32.0, if break_lead { 10.0 } else { 0.0 }),
            (40.0, 10.0),
            (56.0, 0.0),
            (64.0, 0.0),
        ]);
        let perc_l = h(&[
            (0.0, 0.0),
            (8.0, 10.0),
            (32.0, 0.0),
            (40.0, 10.0),
            (64.0, 10.0),
        ]);
        let hats_l = h(&[
            (0.0, 6.0),
            (8.0, 10.0),
            (32.0, 3.0),
            (40.0, 10.0),
            (56.0, 6.0),
            (64.0, 6.0),
        ]);
        let sweep: Vec<(f64, f64, &str)> = vec![
            (0.0, -0.9, "hold"),
            (16.0, 0.0, "ease_in"),
            (32.0, 0.2, "linear"),
            (38.0, 0.8, "ease_in"),
            (40.0, 0.1, "hold"),
            (56.0, 0.35, "linear"),
            (64.0, -0.9, "linear"),
        ];
        let base = if c.texture == "Echoing" { 6.0 } else { 4.0 };
        let fx = h(&[
            (0.0, base),
            (15.0, 10.0),
            (16.0, base),
            (31.0, 10.0),
            (32.0, base + 2.0),
            (39.0, 10.0),
            (40.0, base),
            (55.0, 10.0),
            (56.0, base),
            (64.0, base),
        ]);
        let key = h(&[
            (0.0, 0.0),
            (32.0, c.break_shift as f64 / 12.0),
            (40.0, c.drop2_shift as f64 / 12.0),
            (56.0, 0.0),
            (64.0, 0.0),
        ]);
        let riser: Vec<(f64, f64, &str)> = vec![
            (0.0, 0.0, "hold"),
            (12.0, 0.0, "hold"),
            (15.9, 10.0, "ease_in"),
            (16.0, 0.0, "hold"),
            (36.0, 0.0, "hold"),
            (39.9, 10.0, "ease_in"),
            (40.0, 0.0, "hold"),
            (64.0, 0.0, "hold"),
        ];
        let fill = h(&[
            (0.0, 0.0),
            (15.0, 10.0),
            (16.0, 0.0),
            (39.0, 10.0),
            (40.0, 0.0),
            (64.0, 0.0),
        ]);
        if c.break_shift != 0 {
            let name = |n: i64| NOTE[(c.tonic + n).rem_euclid(12) as usize];
            self.note(format!(
                "breakdown in {}{}",
                name(c.break_shift),
                if c.drop2_shift != 0 {
                    ", second drop stays there"
                } else {
                    ""
                }
            ));
        }
        let long_form = c.form_kind == "long";
        let (kick_l, bass_l, lead_l, perc_l, hats_l, sweep, riser, fill) = if long_form {
            // 16-bar sections: the groove holds, layers arrive one at a time and the filter opens over 48 bars
            (
                if ambient {
                    h(&[(0.0, 0.0), (16.0, 6.0), (56.0, 0.0), (64.0, 0.0)])
                } else {
                    h(&[(0.0, 10.0), (64.0, 10.0)])
                },
                if c.bass_style == "drone" {
                    h(&[(0.0, 10.0), (64.0, 10.0)])
                } else {
                    h(&[(0.0, 0.0), (8.0, 10.0), (64.0, 10.0)])
                },
                h(&[(0.0, 0.0), (32.0, 10.0), (56.0, 0.0), (64.0, 0.0)]),
                h(&[(0.0, 0.0), (32.0, 10.0), (56.0, 0.0), (64.0, 0.0)]),
                h(&[
                    (0.0, 0.0),
                    (16.0, 8.0),
                    (32.0, 10.0),
                    (56.0, 6.0),
                    (64.0, 6.0),
                ]),
                vec![
                    (0.0, -1.1, "hold"),
                    (48.0, 0.6, "ease_in_out"),
                    (64.0, -1.1, "linear"),
                ],
                h(&[(0.0, 0.0), (64.0, 0.0)]),
                h(&[(0.0, 0.0), (64.0, 0.0)]),
            )
        } else {
            (kick_l, bass_l, lead_l, perc_l, hats_l, sweep, riser, fill)
        };
        let fx = if long_form {
            h(&[
                (0.0, base + 1.0),
                (15.0, 10.0),
                (16.0, base + 1.0),
                (31.0, 10.0),
                (32.0, base + 1.0),
                (47.0, 10.0),
                (48.0, base + 1.0),
                (63.0, 10.0),
                (64.0, base + 1.0),
            ])
        } else {
            fx
        };
        let mut drops = if long_form {
            vec![32.0, 40.0]
        } else {
            vec![16.0, 40.0]
        };
        let (kick_l, bass_l, lead_l, perc_l, hats_l, sweep, fx, key, riser, fill) =
            match c.form_kind {
                "progressive" => {
                    // the long filter build: kick and bass from the top, the filter opens over 30 bars into the drop at
                    // 32, an 8-bar breakdown at 48, a last drop at 56
                    drops = vec![32.0, 56.0];
                    let ramp = |a: f64, b: f64| -> Vec<(f64, f64, &'static str)> {
                        vec![
                            (0.0, 0.0, "hold"),
                            (a, 0.0, "hold"),
                            (b - 0.1, 10.0, "ease_in"),
                            (b, 0.0, "hold"),
                            (52.0, 0.0, "hold"),
                            (55.9, 10.0, "ease_in"),
                            (56.0, 0.0, "hold"),
                            (64.0, 0.0, "hold"),
                        ]
                    };
                    (
                        h(&[
                            (0.0, if ambient { 6.0 } else { 10.0 }),
                            (31.5, 0.0),
                            (32.0, 10.0),
                            (48.0, 0.0),
                            (56.0, 10.0),
                            (64.0, 10.0),
                        ]),
                        if c.bass_style == "drone" {
                            h(&[(0.0, 10.0), (64.0, 10.0)])
                        } else {
                            h(&[
                                (0.0, 0.0),
                                (8.0, 10.0),
                                (31.5, 0.0),
                                (32.0, 10.0),
                                (48.0, 0.0),
                                (56.0, 10.0),
                                (64.0, 10.0),
                            ])
                        },
                        h(&[
                            (0.0, 0.0),
                            (32.0, 10.0),
                            (48.0, if break_lead { 10.0 } else { 0.0 }),
                            (56.0, 10.0),
                            (64.0, 10.0),
                        ]),
                        h(&[
                            (0.0, 0.0),
                            (16.0, 10.0),
                            (48.0, 0.0),
                            (56.0, 10.0),
                            (64.0, 10.0),
                        ]),
                        h(&[
                            (0.0, 6.0),
                            (8.0, 10.0),
                            (48.0, 3.0),
                            (56.0, 10.0),
                            (64.0, 10.0),
                        ]),
                        vec![
                            (0.0, -1.0, "hold"),
                            (30.0, 0.7, "ease_in"),
                            (32.0, 0.15, "hold"),
                            (46.0, 0.5, "linear"),
                            (48.0, -0.3, "hold"),
                            (55.0, 0.8, "ease_in"),
                            (56.0, 0.2, "hold"),
                            (64.0, -1.0, "linear"),
                        ],
                        h(&[
                            (0.0, base),
                            (31.0, 10.0),
                            (32.0, base),
                            (47.0, 10.0),
                            (48.0, base + 2.0),
                            (55.0, 10.0),
                            (56.0, base),
                            (64.0, base),
                        ]),
                        h(&[
                            (0.0, 0.0),
                            (48.0, c.break_shift as f64 / 12.0),
                            (56.0, c.drop2_shift as f64 / 12.0),
                            (64.0, 0.0),
                        ]),
                        ramp(26.0, 32.0),
                        h(&[
                            (0.0, 0.0),
                            (31.0, 10.0),
                            (32.0, 0.0),
                            (55.0, 10.0),
                            (56.0, 0.0),
                            (64.0, 0.0),
                        ]),
                    )
                }
                "collage" => {
                    // abrupt cuts between scenes of 3–8 bars; at least two full scenes carry the song
                    let mut bars = vec![0i64];
                    while *bars.last().unwrap() < 64 {
                        let n = bars.last().unwrap() + rng.pick(&[3, 4, 4, 6, 8, 8]);
                        bars.push(n.min(64));
                    }
                    if bars[bars.len() - 1] - bars[bars.len() - 2] < 2 {
                        bars.remove(bars.len() - 2);
                    }
                    // scene: kick, bass, lead, perc, hats
                    let scenes: [([f64; 5], f64); 6] = [
                        ([1.0, 1.0, 1.0, 1.0, 1.0], 4.0),
                        ([0.0, 1.0, 1.0, 0.0, 0.0], 2.0),
                        ([1.0, 1.0, 0.0, 1.0, 1.0], 2.0),
                        ([0.0, 0.0, 0.0, 0.0, 0.0], 1.0),
                        ([0.0, 0.0, 1.0, 0.0, 0.3], 1.0),
                        ([1.0, 1.0, 0.0, 0.0, 0.6], 1.5),
                    ];
                    let n = bars.len() - 1;
                    let mut pick: Vec<usize> = (0..n)
                        .map(|i| {
                            if i == 0 {
                                rng.pick(&[1, 3, 4])
                            } else {
                                rng.weighted(
                                    &scenes
                                        .iter()
                                        .enumerate()
                                        .map(|(k, x)| (k, x.1))
                                        .collect::<Vec<_>>(),
                                )
                            }
                        })
                        .collect();
                    // the two longest later scenes become full if fewer than two are
                    let mut by_len: Vec<usize> = (1..n).collect();
                    by_len.sort_by_key(|&i| -(bars[i + 1] - bars[i]));
                    for &i in by_len.iter() {
                        if pick.iter().filter(|&&p| p == 0).count() >= 2 {
                            break;
                        }
                        pick[i] = 0;
                    }
                    let full: Vec<usize> = (0..n).filter(|&i| pick[i] == 0).collect();
                    drops = full.iter().take(2).map(|&i| bars[i] as f64).collect();
                    let lane_of = |k: usize, top: f64| -> Vec<(f64, f64, &'static str)> {
                        let mut v: Vec<(f64, f64, &'static str)> = vec![];
                        for i in 0..n {
                            let x = scenes[pick[i]].0[k] * top;
                            if v.last().is_none_or(|l| l.1 != x) {
                                v.push((bars[i] as f64, x, "hold"));
                            }
                        }
                        v.push((64.0, v[0].1, "hold"));
                        v
                    };
                    let shifted: Vec<bool> = (0..n)
                        .map(|i| c.break_shift != 0 && i > 0 && pick[i] != 0 && rng.chance(0.35))
                        .collect();
                    let mut cut = vec![];
                    let mut fxl = vec![];
                    let mut keyl = vec![];
                    let mut ris = vec![(0.0, 0.0, "hold")];
                    let mut fil = vec![(0.0, 0.0, "hold")];
                    let mut throws = 0;
                    for i in 0..n {
                        let (a, b) = (bars[i] as f64, bars[i + 1] as f64);
                        let v = if pick[i] == 0 {
                            rng.uni(0.1, 0.6)
                        } else {
                            rng.uni(-0.6, 0.3)
                        };
                        cut.push((a, v, "hold"));
                        fxl.push((a, base, "hold"));
                        if throws < 6 && b - a >= 3.0 && rng.chance(0.4) {
                            fxl.push((b - 1.0, 10.0, "hold"));
                            throws += 1;
                        }
                        keyl.push((
                            a,
                            if shifted[i] {
                                c.break_shift as f64 / 12.0
                            } else {
                                0.0
                            },
                            "hold",
                        ));
                        if i + 1 < n && pick[i + 1] == 0 && pick[i] != 0 && b - a >= 3.0 {
                            ris.push(((b - 3.0).max(a), 0.0, "hold"));
                            ris.push((b - 0.1, 10.0, "ease_in"));
                            ris.push((b, 0.0, "hold"));
                            fil.push((b - 1.0, 10.0, "hold"));
                            fil.push((b, 0.0, "hold"));
                        }
                    }
                    cut.push((64.0, cut[0].1, "hold"));
                    fxl.push((64.0, base, "hold"));
                    keyl.push((64.0, 0.0, "hold"));
                    ris.push((64.0, 0.0, "hold"));
                    fil.push((64.0, 0.0, "hold"));
                    (
                        lane_of(0, if ambient { 6.0 } else { 10.0 }),
                        if c.bass_style == "drone" {
                            h(&[(0.0, 10.0), (64.0, 10.0)])
                        } else {
                            lane_of(1, 10.0)
                        },
                        lane_of(2, 10.0),
                        lane_of(3, 10.0),
                        lane_of(4, 10.0),
                        cut,
                        fxl,
                        keyl,
                        ris,
                        fil,
                    )
                }
                _ => (
                    kick_l, bass_l, lead_l, perc_l, hats_l, sweep, fx, key, riser, fill,
                ),
            };
        self.drops = drops;
        type AutomationLane = (&'static str, Vec<(f64, f64, &'static str)>);
        let lanes: Vec<AutomationLane> = vec![
            ("kick", kick_l),
            ("bass", bass_l),
            ("lead", lead_l),
            ("perc", perc_l),
            ("hats", hats_l),
            ("cut", sweep),
            ("fx", fx),
            ("key", key),
            ("riser", riser),
            ("fill", fill),
        ];
        self.lanes = lanes.iter().map(|l| l.0).collect();
        self.scores.insert(
            "seq_drums".into(),
            json!({"version": 1, "seed": self.seed % 1_000_000, "length": 64, "steps_per_beat": 4,
            "tracks": [trk(kick, 1, 0), trk(clap, 1, 0), trk(hats, 1, 0), trk(ohat, 1, 0)],
            "lanes": lanes.iter().map(|l| lane(&l.1)).collect::<Vec<_>>()}),
        );
        let glide = if matches!(
            bstyle,
            "house" | "lofi" | "gen" | "sub" | "garage" | "broken"
        ) {
            60
        } else {
            0
        };
        self.scores.insert("seq_music".into(), json!({"version": 1, "seed": (self.seed + 7) % 1_000_000, "length": 64, "steps_per_beat": 4 / div,
            "tracks": [trk(chords, 4, 0), trk(bass, 1, glide), trk(melody_track, 1, if c.texture == "Liquid" { 60 } else { 20 }), trk(counter, 1, 40)]}));
        let mut fill = vec![];
        let fill_style = rng.idx(3);
        for k in 0..16i64 {
            let on = match fill_style {
                0 => true,
                1 => k >= 8 || k % 2 == 0,
                _ => k % 4 != 1,
            };
            if !on {
                continue;
            }
            let mut s = st(48 + k, &[0], 0.35 + 0.65 * k as f64 / 15.0, 0.3, 1.0, false);
            if k >= 12 && fill_style != 2 {
                s["ratchet"] = json!(2);
            }
            fill.push(s);
        }
        let [p1, p2, p3]: [Vec<Value>; 3] = percs.try_into().unwrap();
        self.scores.insert("seq_perc".into(), json!({"version": 1, "seed": (self.seed + 13) % 1_000_000, "length": 64, "steps_per_beat": 4,
            "tracks": [trk(p1, 1, 0), trk(p2, 1, 0), trk(p3, 1, 0), trk(fill, 1, 0)]}));
        if c.texture == "Glitchy" {
            let stut_pool = [
                (28, 3),
                (44, 2),
                (60, 4),
                (12, 2),
                (22, 2),
                (54, 3),
                (38, 2),
            ];
            let crush_pool = [(14, 2), (36, 4), (54, 2), (6, 2), (46, 4), (30, 2)];
            let mut stutter: Vec<Value> = rng
                .sample(stut_pool.len(), 3)
                .into_iter()
                .flat_map(|i| held(stut_pool[i].0, stut_pool[i].1, &[0], 1.0, 0.9))
                .collect();
            let mut crush: Vec<Value> = rng
                .sample(crush_pool.len(), 3)
                .into_iter()
                .flat_map(|i| held(crush_pool[i].0, crush_pool[i].1, &[0], 1.0, 0.9))
                .collect();
            for s in stutter.iter_mut().take(2).chain(crush.iter_mut().take(2)) {
                s["probability"] = json!(0.6);
            }
            let amt = rng.pick(&[4.0, 6.0]);
            self.scores.insert("seq_glitch".into(), json!({"version": 1, "seed": (self.seed + 99) % 1_000_000, "length": 64, "steps_per_beat": 4,
                "tracks": [trk(stutter, 1, 0), trk(crush, 1, 0)],
                "lanes": [lane(&[(0.0, 0.0, "hold"), (16.0, amt, "hold"), (24.0, 8.0, "hold"), (30.0, 10.0, "hold"), (32.0, 3.0, "hold"), (40.0, 8.0, "hold"), (52.0, 10.0, "hold"), (56.0, 0.0, "hold"), (64.0, 0.0, "hold")])]}));
        }
    }

    // ------------------------------------------------------------ graph
    fn assemble(&mut self, c: &mut Ctx) {
        // sound details follow the parent genre (drum kit, chord voicing, mix); the genre's own gain is fitted
        let (genre, tex) = (parent(c.genre), c.texture);
        let (sp, six) = (c.space, c.six);
        let names = self.chord_names.join(" | ");
        let voice_word = instruments::design(c.voice).map_or("chords", |x| x.words);
        let lead_shape = match c.lead {
            "hook" => "a sung hook",
            "drift" => "a generative arp that loops and slowly mutates",
            "arp" => "an arpeggio",
            "bells" => "sparse bell tones",
            "riff" => "a looping riff",
            "counter" => "a slow counter-melody",
            _ => "",
        };
        let lead_word = &match instruments::design(c.lead_design) {
            Some(x) if c.lead_design != "classic" && !lead_shape.is_empty() => {
                format!(
                    "{lead_shape} on {}",
                    x.words.trim_start_matches("a ").trim_start_matches("an ")
                )
            }
            _ => lead_shape.to_string(),
        };
        let mut extras = vec![];
        if c.pad {
            extras.push("a pad layer".to_string());
        }
        if c.counter && c.lead != "counter" {
            extras.push("a counter-line".into());
        }
        for p in [c.perc, c.perc2].into_iter().flatten() {
            extras.push(format!("{p} percussion"));
        }
        let bass_word = if c.bass_style == "drone" {
            "a deep sine drone".to_string()
        } else {
            format!(
                "a {} bass",
                if c.bass_style == "gen" {
                    "generated"
                } else {
                    c.bass_style
                }
            )
        };
        self.description = format!(
            "{}: {} BPM in {}, {names}. {}{}, {bass_word}{}{}; texture: {}.",
            self.title,
            self.bpm,
            self.key_name,
            voice_word[..1].to_uppercase(),
            &voice_word[1..],
            if lead_word.is_empty() {
                String::new()
            } else {
                format!(", {lead_word}")
            },
            if extras.is_empty() {
                String::new()
            } else {
                format!(", {}", extras.join(", "))
            },
            texture_text(tex)
        );

        // row 0 · mix, row 7 · master
        self.label(
            0,
            format!(
                "ROW 0 · MIX\n\n{} · seed {}\nStudio mixer · A → echo · B → reverb",
                self.title, self.seed
            ),
        );
        self.module("mixer", "studio_mixer", 0, json!({}), "Studio mixer");
        self.label(
            7,
            "ROW 7 · MASTER\n\nGlue 2:1 → limiter → stereo out".into(),
        );
        let bus = if c.glue { "glue" } else { "limit" };
        if c.glue {
            self.module("glue", "sidechain_compressor", 7, json!({"threshold_db": -14, "ratio": 2, "attack_ms": 20, "release_ms": 200, "detector_ms": 10, "knee_db": 8, "makeup_db": 1}), "Glue 2:1");
        }
        self.module("limit", "sidechain_compressor", 7, json!({"threshold_db": -5, "ratio": 20, "attack_ms": 0.1, "release_ms": 80, "detector_ms": 0.1, "knee_db": 2, "makeup_db": 0}), "Limiter");
        // Crisp builds keep brighter, sharper transients: leave them a little more headroom
        let gain = r3(out_gain(c.genre) * if tex == "Crisp" { 0.9 } else { 1.0 });
        self.module(
            "out",
            "audio_stereo",
            7,
            json!({"gain": gain, "dc_block": 1}),
            "Stereo out",
        );
        // spacious and liquid builds (and ambient) widen the mix: mid/side with the side lifted
        let wide = matches!(tex, "Spacious" | "Liquid") || genre == "Ambient";
        if wide {
            self.module(
                "width",
                "mid_side",
                7,
                json!({"encode_width": 1.0, "decode_width": r3(c.rng.uni(1.25, 1.45))}),
                "Stereo width (side lifted)",
            );
            self.con("mixer", "left", "width", "left");
            self.con("mixer", "right", "width", "right");
            self.con("width", "left", bus, "left");
            self.con("width", "right", bus, "right");
        } else {
            self.con("mixer", "left", bus, "left");
            self.con("mixer", "right", bus, "right");
        }
        let (master_l, master_r) = (format!("c{}", self.cid - 1), format!("c{}", self.cid));
        if c.glue {
            self.con("glue", "left", "limit", "left");
            self.con("glue", "right", "limit", "right");
        }
        self.con("limit", "left", "out", "left");
        self.con("limit", "right", "out", "right");
        self.ops.push(Op::Out("out".into()));
        let damp = Self::cut(c.bright, 3200.0, 0.4);
        let predelay = c.rng.pick(&[0.01, 0.02, 0.04]);
        self.module("verb", "stereo_reverb", 0, json!({"decay": r3(0.6 + 0.32 * sp), "size": r3(0.55 + 0.4 * sp), "wet": 1.0, "dry": 0.0, "predelay": predelay, "damping": damp}), "Reverb");
        self.con("mixer", "send_b_left", "verb", "left");
        self.con("mixer", "send_b_right", "verb", "right");
        self.con("verb", "left", "mixer", "return_b");
        self.con_mode("verb", "right", "mixer", "return_b", true);
        let (mut dl, mut dr): (f64, f64) =
            c.rng
                .pick(&[(3.0, 4.0), (3.0, 6.0), (2.0, 3.0), (6.0, 8.0), (4.0, 6.0)]);
        if genre == "Ambient" {
            dl *= 2.0;
            dr *= 2.0;
        }
        while six * dl.max(dr) > 1.9 {
            dl /= 2.0;
            dr /= 2.0;
        }
        let fb = if tex == "Echoing" {
            0.62
        } else {
            0.35 + 0.2 * sp
        };
        // dusty and some lo-fi, dub and collage builds use a mono tape echo whose time wobbles on the motion LFO
        let tape = tex == "Dusty"
            || (matches!(genre, "Lo-fi Hip Hop" | "Dub Techno" | "Downtempo") && c.rng.chance(0.4))
            || (c.form_kind == "collage" && c.rng.chance(0.5));
        if tape {
            let ms = six * dl.max(dr) * 1000.0;
            self.module(
                "echo",
                "stock_delay",
                0,
                json!({"time": r3(ms.log10() / 4.0), "feedback": r3(fb * 0.9), "tone": r3(0.35 + 0.15 * c.bright),
                       "mix": 1.0, "time_cv_amount": 0.004}),
                "Tape echo (wobbling time)",
            );
            self.note(format!("tape echo {}/16", dl.max(dr)));
            self.con("mixer", "send_a_left", "echo", "signal");
            self.con_mode("mixer", "send_a_right", "echo", "signal", true);
            self.con("echo", "wet", "mixer", "return_a");
        } else {
            self.module("echo", "stereo_delay", 0, json!({"time_left": r3(six * dl * 1000.0) / 1000.0, "time_right": r3(six * dr * 1000.0) / 1000.0, "feedback": r3(fb),
            "crossfeed": if tex == "Liquid" { 0.7 } else { 0.45 }, "wet": 1.0, "dry": 0.0, "low_cut": 300, "high_cut": Self::cut(c.bright, 2600.0, 0.4),
            "mod_depth": if tex == "Liquid" { 0.08 } else { 0.03 }, "mod_rate": 0.3}), "Echo");
            self.note(format!("echo {dl}/16 · {dr}/16"));
            self.con("mixer", "send_a_left", "echo", "left");
            self.con("mixer", "send_a_right", "echo", "right");
            self.con("echo", "left", "mixer", "return_a");
            self.con_mode("echo", "right", "mixer", "return_a", true);
        }

        // row 1 · sequencing
        let div = c.g.div;
        self.label(1, format!("ROW 1 · SEQUENCING\n\nseq_drums = master clock ({} BPM, swing {}%) + song-form lanes ({}) → seq_music ({names}{}) · seq_perc",
                              self.bpm, self.swing, c.form, if div == 2 { ", 2 bars each" } else { "" }));
        self.module(
            "seq_drums",
            "score_sequencer",
            1,
            json!({"tempo": self.bpm, "swing": self.swing, "running": 1}),
            "Drums + master clock",
        );
        self.module("lanes", "poly_split", 1, json!({}), "Song-form lanes");
        self.con("seq_drums", "automation", "lanes", "signal");
        let fx = self.lane_out("fx");
        self.con("lanes", &fx, "mixer", "send_a_cv");
        let mut clk = ("seq_drums", "clock");
        if div == 2 {
            self.module("clk_div", "gates", 1, json!({"length": 0.05}), "Clock ÷2");
            self.con("seq_drums", "clock", "clk_div", "in");
            self.con("seq_drums", "reset", "clk_div", "reset");
            clk = ("clk_div", "flip");
        }
        self.module(
            "seq_music",
            "score_sequencer",
            1,
            json!({"tempo": self.bpm, "running": 1}),
            "Harmony: chords · bass · melody · counter",
        );
        self.con(clk.0, clk.1, "seq_music", "clock");
        self.con("seq_drums", "reset", "seq_music", "reset");
        self.module(
            "seq_perc",
            "score_sequencer",
            1,
            json!({"tempo": self.bpm, "running": 1}),
            "Percussion + gates",
        );
        self.con("seq_drums", "clock", "seq_perc", "clock");
        self.con("seq_drums", "reset", "seq_perc", "reset");
        let lfo_hz = if tex == "Liquid" {
            0.25
        } else {
            r3(c.rng.uni(0.04, 0.09))
        };
        self.module(
            "motion",
            "multi_lfo",
            1,
            json!({"frequency": lfo_hz}),
            "Slow motion LFO",
        );
        if self.modules["echo"].kind == "stock_delay" {
            self.con("motion", "sine", "echo", "time_cv");
        }

        // row 2 · drums
        let drum_notes: Vec<String> = self
            .notes
            .iter()
            .filter(|n| ["kick", "hats", "E("].iter().any(|w| n.contains(w)))
            .cloned()
            .collect();
        self.label(
            2,
            format!("ROW 2 · DRUMS\n\n{genre} kit · {}", drum_notes.join(" · ")),
        );
        let (kp, kd, kdr, kt) = c.g.kick;
        let crisp = tex == "Crisp";
        // tune the kick to the key: the tonic (or its fifth) in the genre's kick register
        let kick_hz = {
            let mut best = kp;
            for pc in [c.tonic, c.tonic + 7] {
                let mut hz = 261.6256 * 2f64.powf(pc as f64 / 12.0);
                while hz > kp * 1.2 {
                    hz /= 2.0;
                }
                if (hz - kp).abs() < (best - kp).abs() || best == kp {
                    best = hz;
                }
            }
            best
        };
        let kick_p = json!({"mode": 0, "pitch": r3(kick_hz), "decay": r3(kd * c.rng.uni(0.85, 1.15)), "drive": r3((kdr + c.rng.uni(-0.1, 0.1)).max(0.0)), "tone": kt, "level": 2.0});
        self.module("kick", "drum_voice", 2, kick_p, "Kick");
        self.con("seq_drums", "gate1", "kick", "trigger");
        self.module(
            "kick_vca",
            "stock_vca",
            2,
            json!({"level": 1.0}),
            "Kick lane VCA",
        );
        self.con("kick", "out", "kick_vca", "signal");
        let kl = self.lane_out("kick");
        self.con("lanes", &kl, "kick_vca", "cv");
        self.strip(
            "kick",
            "kick_vca",
            "out",
            if genre == "Ambient" { 1.2 } else { 1.6 },
            0.0,
            20.0,
            0.0,
            0.0,
            0.0,
        );
        if c.clap {
            let snare = matches!(
                genre,
                "Lo-fi Hip Hop" | "Synthwave" | "Downtempo" | "UK Garage"
            );
            let p = json!({"mode": 1, "pitch": c.rng.int(850, 1200), "decay": r3(if genre == "Synthwave" { 0.3 } else { 0.2 } * c.rng.uni(0.85, 1.2)), "tone": if crisp { 0.4 } else { 0.25 }, "level": 1.4});
            self.module(
                "clap",
                "drum_voice",
                2,
                p,
                if genre == "Dub Techno" {
                    "Rim"
                } else if snare {
                    "Snare"
                } else {
                    "Clap"
                },
            );
            self.con("seq_drums", "gate2", "clap", "trigger");
            self.con("seq_drums", "velocity2", "clap", "accent");
            self.strip(
                "clap",
                "clap",
                "out",
                if genre == "Dub Techno" { 1.2 } else { 1.5 },
                0.0,
                200.0,
                if genre == "Dub Techno" { 0.08 } else { 0.0 },
                if genre == "Synthwave" { 0.45 } else { 0.25 },
                -3.0,
            );
        }
        let hp = json!({"mode": 2, "pitch": if crisp { c.rng.int(3000, 3300) } else { c.rng.int(2600, 3100) },
                        "decay": r3(if genre == "Lo-fi Hip Hop" { 0.05 } else { 0.03 } * c.rng.uni(0.8, 1.3)), "tone": if crisp { 0.35 } else { 0.2 }, "level": 1.0});
        self.module("hats", "drum_voice", 2, hp, "Hats");
        self.con("seq_drums", "gate3", "hats", "trigger");
        self.con("seq_drums", "velocity3", "hats", "accent");
        self.module(
            "hats_vca",
            "stock_vca",
            2,
            json!({"level": 1.0}),
            "Hats lane VCA",
        );
        self.con("hats", "out", "hats_vca", "signal");
        let hl = self.lane_out("hats");
        self.con("lanes", &hl, "hats_vca", "cv");
        let pan = c.rng.pick(&[-0.2, 0.2]);
        self.strip("hats", "hats_vca", "out", 1.3, pan, 500.0, 0.0, 0.08, -8.0);
        if self.scores["seq_drums"]["tracks"][3]["steps"]
            .as_array()
            .is_some_and(|s| !s.is_empty())
        {
            self.module("ohat", "drum_voice", 2, json!({"mode": 2, "pitch": 2300, "decay": if crisp { 0.08 } else { 0.11 }, "tone": 0.12, "level": 0.9}), "Open hat");
            self.con("seq_drums", "gate4", "ohat", "trigger");
            self.strip(
                "open hat", "ohat", "out", 0.7, -0.15, 400.0, 0.0, 0.08, -9.0,
            );
        }
        let pl = self.lane_out("perc");
        for (slot, kind) in [c.perc, c.perc2].into_iter().enumerate() {
            let Some(kind) = kind else { continue };
            let id = if slot == 0 { "perc" } else { "perc2" };
            let pc = (c.tonic + c.rng.pick(&[0, 7])).rem_euclid(12);
            let mut hz = 261.62558 * 2f64.powf(pc as f64 / 12.0);
            while hz > 400.0 {
                hz /= 2.0;
            }
            while hz < 180.0 {
                hz *= 2.0;
            }
            let p = match kind {
                "tom" => {
                    json!({"mode": 0, "pitch": r3(hz / 2.0), "decay": 0.25, "drive": 0.0, "tone": 0.15, "level": 1.3})
                }
                "wood" => {
                    json!({"mode": 0, "pitch": r3(hz * 2.0), "decay": 0.05, "drive": 0.0, "tone": 0.1, "level": 1.1})
                }
                "shaker" => {
                    json!({"mode": 2, "pitch": 2300, "decay": 0.07, "tone": 0.1, "level": 0.8})
                }
                "rim" => {
                    json!({"mode": 1, "pitch": 1500, "decay": 0.05, "tone": 0.2, "level": 1.0})
                }
                _ => {
                    json!({"mode": 0, "pitch": r3(hz), "decay": 0.12, "drive": 0.0, "tone": 0.15, "level": 1.3})
                }
            };
            let tuned = matches!(kind, "tom" | "wood" | "conga");
            self.module(
                id,
                "drum_voice",
                2,
                p,
                format!(
                    "{}{}",
                    kind[..1].to_uppercase() + &kind[1..],
                    if tuned {
                        format!(" ({})", NOTE[pc as usize])
                    } else {
                        String::new()
                    }
                ),
            );
            self.con("seq_perc", &format!("gate{}", slot + 1), id, "trigger");
            self.con("seq_perc", &format!("velocity{}", slot + 1), id, "accent");
            let vca = format!("{id}_vca");
            self.module(
                &vca,
                "stock_vca",
                2,
                json!({"level": 1.0}),
                "Percussion lane VCA",
            );
            self.con(id, "out", &vca, "signal");
            self.con("lanes", &pl, &vca, "cv");
            let pan = c.rng.pick(&[-0.35, 0.35]);
            self.strip(
                id,
                &vca,
                "out",
                if kind == "shaker" { 0.97 } else { 0.73 },
                pan,
                120.0,
                0.15,
                0.2,
                -2.0,
            );
        }

        // drum fill into each drop (seq_perc track 4, opened by the fill lane); long-form genres have none
        if !matches!(genre, "Dub Techno" | "Techno" | "Ambient" | "Minecraft") {
            let fl = self.lane_out("fill");
            self.module("fill", "drum_voice", 2, json!({"mode": 1, "pitch": c.rng.int(900, 1300), "decay": 0.12, "tone": 0.3, "level": 1.3}), "Fill snare");
            self.con("seq_perc", "gate4", "fill", "trigger");
            self.con("seq_perc", "velocity4", "fill", "accent");
            self.module(
                "fill_vca",
                "stock_vca",
                2,
                json!({"level": 1.0}),
                "Fill lane VCA",
            );
            self.con("fill", "out", "fill_vca", "signal");
            self.con("lanes", &fl, "fill_vca", "cv");
            self.strip("fill", "fill_vca", "out", 2.5, 0.1, 200.0, 0.1, 0.3, -3.0);
        }

        // row 3 · bass
        let bstyle = c.bass_style;
        let drone = bstyle == "drone";
        let bass_kind = if drone { "sine" } else { c.bass_design };
        self.label(
            3,
            format!(
                "ROW 3 · BASS\n\n{} · {}{}",
                instruments::design(bass_kind).map_or("sine drone on the roots", |x| x.words),
                self.notes
                    .iter()
                    .find(|n| n.contains("bass") || n.contains("drone"))
                    .cloned()
                    .unwrap_or_default(),
                if c.g.pump {
                    " · ducked by the kick"
                } else {
                    ""
                }
            ),
        );
        self.module("bass_osc", "multi_vco", 3, json!({"frequency": 261.62558, "pulse_width": if bass_kind == "pwm" { 0.4 } else { 0.5 }, "pwm_amount": if bass_kind == "pwm" { 0.3 } else { 0.0 }}), "Bass oscillator");
        let (bp, bo) = self.key_src(c, "pitch2");
        self.con(&bp, &bo, "bass_osc", "pitch");
        if bass_kind == "pwm" {
            self.con("motion", "sine", "bass_osc", "pwm");
        }
        let env = if drone {
            json!({"attack": 0.5, "decay": 0.25, "sustain": 0.8, "release": 1.2})
        } else {
            json!({"attack": 0.004, "decay": r3(0.25 * c.rng.uni(0.8, 1.3)), "sustain": 0.45, "release": 0.09})
        };
        self.module("bass_env", "adsr", 3, env, "Bass envelope");
        self.con("seq_music", "gate2", "bass_env", "gate");
        self.module(
            "bass_gate",
            "stock_vca",
            3,
            json!({"level": 1.0}),
            "Envelope × song-form lane",
        );
        self.con("bass_env", "out", "bass_gate", "signal");
        let bl = self.lane_out("bass");
        self.con("lanes", &bl, "bass_gate", "cv");
        self.kinds.insert("bass", bass_kind.into());
        let main = self.bass_design(c, bass_kind, &(bp.clone(), bo.clone()));
        let filtered = main.is_some() || !matches!(bass_kind, "sine" | "sub");
        if filtered && main.is_none() {
            let wave = match bass_kind {
                "square" | "pwm" => "square",
                "round" => "triangle",
                _ => "saw",
            };
            self.module(
                "bass_fcv",
                "cv_mix",
                3,
                json!({"level1": 0.5, "level2": r3(c.rng.uni(0.15, 0.26)), "level3": 0.0}),
                "Filter CV: sweep + envelope",
            );
            let cl = self.lane_out("cut");
            self.con("lanes", &cl, "bass_fcv", "in1");
            self.con("bass_env", "out", "bass_fcv", "in2");
            let base = if bstyle == "octave" {
                380.0
            } else if bass_kind == "round" {
                500.0
            } else {
                260.0
            };
            let cutoff = Self::cut(c.bright, base * c.rng.uni(0.8, 1.25), 0.5);
            let bold = matches!(genre, "Techno" | "UK Garage" | "Synthwave" | "Dub Techno");
            let (res, drive) = if bold {
                (c.rng.uni(0.25, 0.4), c.rng.uni(0.2, 0.4))
            } else {
                (c.rng.uni(0.15, 0.3), 0.0)
            };
            self.module("bass_vcf", "stock_vcf", 3, json!({"frequency": vcf_knob(cutoff), "resonance": r3(res), "drive": r3(drive), "cutoff_cv_amount": 1.0}), format!("Four-pole low-pass ({wave}){}", if bold { " · driven" } else { "" }));
            self.con("bass_osc", wave, "bass_vcf", "signal");
            self.con("bass_fcv", "out", "bass_vcf", "cutoff_cv");
            self.module(
                "bass_vca",
                "stock_vca",
                3,
                json!({"level": 1.0}),
                "Bass VCA",
            );
            self.con("bass_vcf", "lowpass", "bass_vca", "signal");
            self.con("bass_gate", "out", "bass_vca", "cv");
        }
        self.module("sub_vca", "stock_vca", 3, json!({"level": 1.0}), "Sub VCA");
        self.con("bass_osc", "sine", "sub_vca", "signal");
        self.con("bass_gate", "out", "sub_vca", "cv");
        let main = main.unwrap_or_else(|| {
            (
                if filtered { "bass_vca" } else { "sub_vca" }.into(),
                "out".into(),
            )
        });
        let (bass_src, sub_src) = if c.g.pump {
            self.module("low_pump", "sidechain_compressor", 3, json!({"threshold_db": -26, "ratio": 5, "attack_ms": 0.5, "release_ms": 120, "detector_ms": 2, "knee_db": 4, "makeup_db": 0}), "Kick ducks the bass");
            self.con(&main.0, &main.1, "low_pump", "left");
            self.con("sub_vca", "out", "low_pump", "right");
            self.con("kick_vca", "out", "low_pump", "sidechain");
            (
                ("low_pump".to_string(), "left".to_string()),
                ("low_pump", "right"),
            )
        } else {
            (main, ("sub_vca", "out"))
        };
        if filtered {
            let level = if bstyle == "octave" { 1.2 } else { 1.3 }
                * 10f64.powf(trim_db(&format!("bass:{bass_kind}")) / 20.0);
            self.strip(
                "bass",
                &bass_src.0,
                &bass_src.1,
                level,
                0.0,
                40.0,
                0.0,
                0.0,
                -2.0,
            );
        }
        let sub_level = match (drone, bass_kind) {
            (true, _) => 0.31,
            (_, "sub") => 1.35,
            _ => 1.12,
        };
        self.strip(
            "sub", sub_src.0, sub_src.1, sub_level, 0.0, 20.0, 0.0, 0.0, 0.0,
        );

        // row 4 · chords (+ optional pad layer)
        self.label(
            4,
            format!(
                "ROW 4 · CHORDS\n\n{}{} · 4 voices · low-pass on the sweep lane{}{}",
                voice_word[..1].to_uppercase(),
                &voice_word[1..],
                if c.pad {
                    " · plus a held pad layer"
                } else {
                    ""
                },
                if c.g.pump {
                    " · pumped by the kick"
                } else {
                    ""
                }
            ),
        );
        let pitch = self.key_src(c, "pitch1");
        let cl = self.lane_out("cut");
        let mut fcv = ("lanes".to_string(), cl.clone());
        if tex == "Liquid" {
            self.module(
                "tide",
                "cv_mix",
                4,
                json!({"level1": 1.0, "level2": 0.25, "level3": 0.0}),
                "Filter tide: sweep + LFO",
            );
            self.con("lanes", &cl, "tide", "in1");
            self.con("motion", "sine", "tide", "in2");
            fcv = ("tide".into(), "out".into());
        }
        self.kinds.insert("chords", c.voice.into());
        self.kinds.insert("lead", c.lead.into());
        let chord_out = self.chord_voice(c, "chord", c.voice, &pitch, ("seq_music", "gate1"), &fcv);
        self.module(
            "chord_sum",
            "poly_sum",
            4,
            json!({"level": 1.0}),
            "Sum voices",
        );
        self.con(&chord_out.0, &chord_out.1, "chord_sum", "signal");
        let mut chord_src = ("chord_sum".to_string(), "out".to_string());
        if tex == "Pulsing" {
            self.module(
                "chop_env",
                "adsr",
                4,
                json!({"attack": 0.003, "decay": 0.1, "sustain": 1.0, "release": r3(c.six * 0.6)}),
                "Trance-gate envelope",
            );
            self.con("seq_perc", "gate3", "chop_env", "gate");
            self.module("chop", "stock_vca", 4, json!({"level": 1.0}), "Trance gate");
            self.con("chord_sum", "out", "chop", "signal");
            self.con("chop_env", "out", "chop", "cv");
            chord_src = ("chop".into(), "out".into());
        }
        if c.pad {
            self.module(
                "pad_hold",
                "constant",
                4,
                json!({"value": 10.0}),
                "Pad hold (always on)",
            );
            // the held layer needs a sustaining pad design that contrasts with the chord voice
            let pad_kind = choose(
                &mut c.srng,
                c.ctl.weirdness,
                Role::Chords,
                c.genre,
                &self.mood,
                c.texture,
                &["pad_saw", "pad_square"],
                &[c.voice],
                |x| x.sustain && x.tags.contains(&"pad"),
            );
            let pad_kind = if c.light {
                light_of(pad_kind)
            } else {
                pad_kind
            };
            self.kinds.insert("pad", pad_kind.into());
            let p = self.chord_voice(c, "pad", pad_kind, &pitch, ("pad_hold", "out"), &fcv);
            self.module(
                "pad_sum",
                "poly_sum",
                4,
                json!({"level": 1.0}),
                "Sum pad voices",
            );
            self.con(&p.0, &p.1, "pad_sum", "signal");
        }
        let (csrc, psrc) = if c.g.pump || tex == "Pulsing" {
            let release = c.rng.pick(&[180, 220, 260]);
            self.module("pump", "sidechain_compressor", 4, json!({"threshold_db": if tex == "Pulsing" { -30 } else { -24 }, "ratio": 3, "attack_ms": 2, "release_ms": release, "detector_ms": 6, "knee_db": 8, "makeup_db": 1}), "Kick pump: chords (L) · pad (R)");
            self.con(&chord_src.0, &chord_src.1, "pump", "left");
            self.con(
                if c.pad { "pad_sum" } else { "chord_sum" },
                "out",
                "pump",
                "right",
            );
            self.con("kick_vca", "out", "pump", "sidechain");
            (
                ("pump".to_string(), "left".to_string()),
                ("pump".to_string(), "right".to_string()),
            )
        } else {
            (chord_src, ("pad_sum".to_string(), "out".to_string()))
        };
        let dub = genre == "Dub Techno" || tex == "Minimal" || tex == "Echoing";
        let voice_pad = instruments::design(c.voice).is_some_and(|x| x.tags.contains(&"pad"));
        let lvl = match c.voice {
            "pad_saw" | "pad_square" => 0.6,
            "keys" | "piano" => 0.9,
            "organ" => 0.7,
            "pluck" => 0.8,
            "stab" => 0.9,
            _ => 0.8,
        } * 10f64.powf(trim_db(c.voice) / 20.0);
        self.strip(
            "chords",
            &csrc.0,
            &csrc.1,
            lvl,
            0.0,
            150.0,
            if dub {
                0.6
            } else if voice_pad {
                0.1
            } else {
                0.35
            },
            0.25 + 0.2 * sp,
            -3.0,
        );
        if c.pad {
            let pad_trim = trim_db(&format!("pad:{}", self.kinds["pad"]));
            self.strip(
                "pad",
                &psrc.0,
                &psrc.1,
                r3(0.22 * 10f64.powf(pad_trim / 20.0)),
                0.0,
                150.0,
                0.05,
                0.35 + 0.2 * sp,
                -4.0,
            );
        }

        // row 5 · melody (lead + counter-line)
        let has_lead = c.lead != "none";
        let has_counter = c.counter && c.lead != "counter";
        if has_lead || has_counter {
            let mel_notes: Vec<String> = self
                .notes
                .iter()
                .filter(|n| {
                    ["arp", "hook", "bell", "riff", "counter"]
                        .iter()
                        .any(|w| n.contains(w))
                })
                .cloned()
                .collect();
            self.label(
                5,
                format!(
                    "ROW 5 · MELODY\n\n{} built from the chord tones (always in key) · {}",
                    if has_lead {
                        lead_word
                    } else {
                        "a counter-line"
                    },
                    mel_notes.join(" · ")
                ),
            );
            let ll = self.lane_out("lead");
            if has_lead {
                let (osc_wave, base, env, level) = match c.lead {
                    "arp" | "drift" => (
                        c.rng.pick(&["saw", "square", "triangle"]),
                        800.0,
                        json!({"attack": 0.003, "decay": r3(0.18 * c.rng.uni(0.7, 1.5)), "sustain": 0.2, "release": 0.12}),
                        0.7,
                    ),
                    "riff" => (
                        c.rng.pick(&["saw", "square"]),
                        700.0,
                        json!({"attack": 0.003, "decay": 0.15, "sustain": 0.3, "release": 0.1}),
                        0.7,
                    ),
                    "bells" => (
                        "triangle",
                        1800.0,
                        json!({"attack": 0.005, "decay": 1.2, "sustain": 0.0, "release": r3(1.5 * c.rng.uni(0.7, 1.3))}),
                        1.0,
                    ),
                    "counter" => (
                        "triangle",
                        1200.0,
                        json!({"attack": 0.12, "decay": 0.5, "sustain": 0.8, "release": 0.6}),
                        0.8,
                    ),
                    _ => (
                        c.rng.pick(&["triangle", "triangle", "square", "sine"]),
                        1500.0,
                        json!({"attack": 0.03, "decay": 0.4, "sustain": 0.7, "release": r3(0.35 * c.rng.uni(0.7, 1.4))}),
                        0.9,
                    ),
                };
                let base = if osc_wave == "square" {
                    base * 0.6
                } else {
                    base
                };
                let env = if genre == "Minecraft" {
                    json!({"attack": 0.005, "decay": r3(c.rng.uni(1.4, 2.2)), "sustain": 0.0, "release": 1.2})
                } else {
                    env
                };
                self.kinds.insert("lead_design", c.lead_design.into());
                let trim_key = if c.lead_design == "classic" {
                    format!("lead:{}", if c.lead == "drift" { "arp" } else { c.lead })
                } else {
                    format!("lead:{}", c.lead_design)
                };
                let level = level
                    * 10f64.powf(
                        // ambient lifts the lead over a soft kick; the catalog designs are already trimmed hotter
                        (trim_db(&trim_key)
                            + match (genre, c.lead_design) {
                                ("Ambient", "classic") => 9.0,
                                ("Ambient", _) => 4.5,
                                _ => 0.0,
                            })
                            / 20.0,
                    );
                let design = self.lead_design(c, c.lead_design, &env, &ll);
                let classic = design.is_none();
                let (lead_node, lead_port) = design.unwrap_or_else(|| {
                    self.mono_voice(c, "lead", "pitch3", "gate3", osc_wave, base, env, &ll);
                    ("lead_vca".into(), "out".into())
                });
                let pan = c.rng.pick(&[-0.1, 0.1]);
                self.strip(
                    "lead",
                    &lead_node,
                    &lead_port,
                    level
                        * if classic && osc_wave == "square" {
                            0.85
                        } else {
                            1.0
                        },
                    pan,
                    200.0,
                    0.45,
                    0.35,
                    -3.0,
                );
            }
            if has_counter {
                let wave = c.rng.pick(&["triangle", "sine", "square"]);
                self.mono_voice(
                    c,
                    "counter",
                    "pitch4",
                    "gate4",
                    wave,
                    1000.0,
                    json!({"attack": 0.12, "decay": 0.5, "sustain": 0.8, "release": 0.6}),
                    &ll,
                );
                self.strip(
                    "counter",
                    "counter_vca",
                    "out",
                    0.118,
                    0.25,
                    200.0,
                    0.3,
                    0.4,
                    -4.0,
                );
            }
        }

        // row 6 · texture
        self.label(
            6,
            format!(
                "ROW 6 · TEXTURE · {}\n\n{}{}",
                tex.to_uppercase(),
                texture_text(tex)[..1].to_uppercase(),
                &texture_text(tex)[1..]
            ),
        );
        // riser: pink noise through a rising high-pass into each drop (song-form lane drives cutoff and level)
        // the long layering form has no risers or fills; Ambient never does
        let drops = c.form_kind != "long" && genre != "Ambient";
        if tex == "Dusty" || tex == "Spacious" {
            self.module(
                "noise",
                "colored_noise",
                6,
                json!({"level": 1, "seed": self.seed % 100_000}),
                "Noise",
            );
        }
        if drops {
            // riser: a saw that climbs three octaves from the key's root into each drop, softened by a one-pole low-pass
            let rl = self.lane_out("riser");
            let root_hz = 261.6256 * 2f64.powf(((c.tonic + 12) % 12 - 24) as f64 / 12.0);
            self.module(
                "riser_scale",
                "cv_mix",
                6,
                json!({"level1": 0.3, "level2": 0.0, "level3": 0.0}),
                "Riser pitch (lane × 3 octaves)",
            );
            self.con("lanes", &rl, "riser_scale", "in1");
            self.module(
                "riser_osc",
                "multi_vco",
                6,
                json!({"frequency": r3(root_hz)}),
                "Riser tone",
            );
            self.con("riser_scale", "out", "riser_osc", "pitch");
            self.module(
                "riser_lp",
                "lowpass",
                6,
                json!({"cutoff": 2500}),
                "Riser softening (one-pole)",
            );
            self.con("riser_osc", "saw", "riser_lp", "signal");
            self.module(
                "riser_vca",
                "stock_vca",
                6,
                json!({"level": 1.0}),
                "Riser level (lane)",
            );
            self.con("riser_lp", "out", "riser_vca", "signal");
            self.con("lanes", &rl, "riser_vca", "cv");
            self.strip(
                "riser",
                "riser_vca",
                "out",
                0.05,
                0.0,
                300.0,
                0.15,
                0.45,
                -5.0,
            );
        }
        if tex == "Dusty" || tex == "Spacious" {
            if tex == "Dusty" {
                self.module(
                    "hiss",
                    "lowpass",
                    6,
                    json!({"cutoff": 2200}),
                    "Hiss low-pass (one-pole)",
                );
                self.con("noise", "pink", "hiss", "signal");
                self.strip(
                    "tape hiss",
                    "hiss",
                    "out",
                    0.018,
                    0.0,
                    300.0,
                    0.0,
                    0.0,
                    -4.0,
                );
            } else {
                self.module(
                    "wind",
                    "svf_filter",
                    6,
                    json!({"cutoff": 600, "resonance": 0.15, "cutoff_cv_amount": 1.0}),
                    "Wind band-pass",
                );
                self.con("noise", "white", "wind", "signal");
                self.con("motion", "triangle", "wind", "cutoff_cv");
                self.strip("wind", "wind", "band", 0.076, -0.3, 150.0, 0.0, 0.3, -4.0);
            }
        }
        if tex == "Glitchy" {
            self.glitch_bus(c, &master_l, &master_r);
        }

        let about = format!(
            "PATCH ENGINE · {} · seed {}\n\n{}\n\n{}\n\ngenome {}",
            self.title.to_uppercase(),
            self.seed,
            self.description,
            self.notes.join(" · "),
            self.genome
        );
        self.modules.insert(
            "about".into(),
            Module {
                kind: "notes",
                row: 7,
                params: json!({}),
                label: "About".into(),
                content: Some(json!({"type": "notes", "text": about})),
            },
        );
        self.ops.push(Op::Add("about".into()));

        // mixer parameters
        let mut p = Map::new();
        p.insert("channels".into(), json!(self.strips.len()));
        p.insert("master".into(), json!(0.8));
        p.insert("return_a".into(), json!(0.7));
        p.insert("return_b".into(), json!(r3(0.6 + 0.25 * sp)));
        for (i, s) in self.strips.iter().enumerate() {
            let n = i + 1;
            // the strip gives at most ×4 (trim 2 × level 2)
            let (trim, level) = if s.level > 2.0 {
                ((s.level / 2.0).min(2.0), 2.0)
            } else {
                (1.0, s.level)
            };
            for (k, v) in [
                ("trim", trim),
                ("level", level),
                ("pan", s.pan),
                ("hpf", s.hpf),
                ("send_a", s.send_a),
                ("send_b", s.send_b),
                ("high_db", s.high_db),
            ] {
                p.insert(format!("{k}{n}"), json!(r3(v)));
            }
        }
        let legend = self
            .strips
            .iter()
            .enumerate()
            .map(|(i, s)| format!("{} {}", i + 1, s.name))
            .collect::<Vec<_>>()
            .join(" · ");
        if let Some(m) = self.modules.get_mut("mixer") {
            m.params = Value::Object(p);
            m.label = format!("Studio mixer ({} strips)", self.strips.len());
        }
        if let Some(Value::Object(content)) = self
            .modules
            .get_mut("label_0")
            .and_then(|m| m.content.as_mut())
        {
            let text = content["text"].as_str().unwrap_or_default().to_string();
            content.insert("text".into(), json!(format!("{text}\n\n{legend}")));
        }
    }

    /// A polyphonic chord voice. Returns the poly output (before summing).
    fn chord_voice(
        &mut self,
        c: &mut Ctx,
        id: &str,
        kind: &str,
        pitch: &(String, String),
        gate: (&str, &str),
        fcv: &(String, String),
    ) -> (String, String) {
        if let Some(out) = self.chord_design(c, id, kind, pitch, gate, fcv) {
            return out;
        }
        let row = 4;
        let detune = |cents: f64| r3(261.62558 * 2f64.powf(cents / 1200.0));
        let osc = format!("{id}_osc");
        let mut src: (String, String);
        match kind {
            "pad_saw" | "stab" => {
                let cents = c.rng.uni(3.0, 6.0);
                self.module(
                    &osc,
                    "analog_vco",
                    row,
                    json!({"frequency": detune(cents), "waveform": 0}),
                    "Saw A",
                );
                self.con(&pitch.0, &pitch.1, &osc, "pitch");
                let b = format!("{id}_osc2");
                let (b_kind, b_params, b_out) = if kind == "stab" {
                    (
                        "multi_vco",
                        json!({"frequency": detune(-cents), "pulse_width": 0.45}),
                        "square",
                    )
                } else {
                    (
                        "analog_vco",
                        json!({"frequency": detune(-cents), "waveform": 0}),
                        "out",
                    )
                };
                self.module(
                    &b,
                    b_kind,
                    row,
                    b_params,
                    if kind == "stab" {
                        "Square B"
                    } else {
                        "Saw B (detuned)"
                    },
                );
                self.con(&pitch.0, &pitch.1, &b, "pitch");
                let mix = format!("{id}_mix");
                self.module(
                    &mix,
                    "cv_mix",
                    row,
                    json!({"level1": 0.5, "level2": 0.5, "level3": 0.0}),
                    "Blend oscillators",
                );
                self.con(&osc, "out", &mix, "in1");
                self.con(&b, b_out, &mix, "in2");
                src = (mix, "out".into());
            }
            "pad_square" => {
                self.module(
                    &osc,
                    "multi_vco",
                    row,
                    json!({"frequency": 261.62558, "pulse_width": r3(c.rng.uni(0.3, 0.5))}),
                    "Square",
                );
                self.con(&pitch.0, &pitch.1, &osc, "pitch");
                src = (osc.clone(), "square".into());
            }
            "organ" => {
                self.module(
                    &osc,
                    "multi_vco",
                    row,
                    json!({"frequency": 261.62558}),
                    "Sine 8′",
                );
                self.con(&pitch.0, &pitch.1, &osc, "pitch");
                let b = format!("{id}_osc2");
                self.module(
                    &b,
                    "multi_vco",
                    row,
                    json!({"frequency": 523.25116}),
                    "Sine 4′ (octave)",
                );
                self.con(&pitch.0, &pitch.1, &b, "pitch");
                let mix = format!("{id}_mix");
                self.module(
                    &mix,
                    "cv_mix",
                    row,
                    json!({"level1": 0.6, "level2": r3(c.rng.uni(0.25, 0.45)), "level3": 0.0}),
                    "Drawbars",
                );
                self.con(&osc, "sine", &mix, "in1");
                self.con(&b, "sine", &mix, "in2");
                src = (mix, "out".into());
            }
            "keys" | "piano" => {
                self.module(
                    &osc,
                    "multi_vco",
                    row,
                    json!({"frequency": 261.62558}),
                    if kind == "piano" {
                        "Piano oscillator"
                    } else {
                        "Keys oscillator"
                    },
                );
                self.con(&pitch.0, &pitch.1, &osc, "pitch");
                let mix = format!("{id}_mix");
                self.module(
                    &mix,
                    "cv_mix",
                    row,
                    json!({"level1": 0.55, "level2": 0.45, "level3": 0.0}),
                    "Triangle + sine",
                );
                self.con(&osc, "triangle", &mix, "in1");
                self.con(&osc, "sine", &mix, "in2");
                src = (mix, "out".into());
            }
            _ => {
                // pluck
                self.module(
                    &osc,
                    "analog_vco",
                    row,
                    json!({"frequency": 261.62558, "waveform": 0}),
                    "Saw",
                );
                self.con(&pitch.0, &pitch.1, &osc, "pitch");
                src = (osc.clone(), "out".into());
            }
        }
        let (a, d, s, r) = match kind {
            "pad_saw" | "pad_square" => (c.rng.uni(0.25, 0.8), 0.8, 0.85, c.rng.uni(0.8, 1.6)),
            "organ" => (0.005, 0.2, 1.0, 0.08),
            "keys" => (0.003, c.rng.uni(0.6, 1.2), 0.15, 0.4),
            "piano" => (0.003, c.rng.uni(1.8, 3.0), 0.0, 1.6),
            "stab" => (0.003, c.rng.uni(0.15, 0.3), 0.0, 0.15),
            _ => (0.002, c.rng.uni(0.2, 0.45), 0.0, 0.25),
        };
        let env = format!("{id}_env");
        self.module(
            &env,
            "adsr",
            row,
            json!({"attack": r3(a), "decay": r3(d), "sustain": s, "release": r3(r)}),
            "Envelope",
        );
        self.con(gate.0, gate.1, &env, "gate");
        let vcf = format!("{id}_vcf");
        let pluck = kind == "pluck" || kind == "stab";
        let base = match kind {
            "pad_saw" => 800.0,
            "pad_square" => 700.0,
            "organ" => 2200.0,
            "keys" => 1800.0,
            "stab" => 900.0,
            _ => 600.0,
        };
        let cutoff = Self::cut(c.bright, base * c.rng.uni(0.8, 1.2), 0.6);
        if kind == "organ" || kind == "keys" || kind == "piano" {
            // sine/triangle voices are already mellow: no filter needed (saves a four-pole VCF per build)
        } else if id == "pad" {
            // the held pad layer sits underneath: two one-pole low-passes (12 dB/oct) cost a tenth of a swept VCF
            let hz = r3(cutoff * 0.9);
            self.module(
                &vcf,
                "lowpass",
                row,
                json!({"cutoff": hz}),
                "Low-pass (one-pole)",
            );
            self.con(&src.0, &src.1, &vcf, "signal");
            let vcf2 = format!("{id}_vcf2");
            self.module(
                &vcf2,
                "lowpass",
                row,
                json!({"cutoff": hz}),
                "Low-pass (one-pole)",
            );
            self.con(&vcf, "out", &vcf2, "signal");
            src = (vcf2, "out".into());
        } else if pluck {
            // per-voice filter envelope: sweep lane + envelope
            let fm = format!("{id}_fcv");
            self.module(
                &fm,
                "cv_mix",
                row,
                json!({"level1": 1.0, "level2": r3(c.rng.uni(0.2, 0.35)), "level3": 0.0}),
                "Filter CV: sweep + envelope",
            );
            self.con(&fcv.0, &fcv.1, &fm, "in1");
            self.con(&env, "out", &fm, "in2");
            self.module(&vcf, "stock_vcf", row, json!({"frequency": vcf_knob(cutoff), "resonance": r3(c.rng.uni(0.1, 0.25)), "cutoff_cv_amount": 1.0}), "Four-pole low-pass");
            self.con(&src.0, &src.1, &vcf, "signal");
            self.con(&fm, "out", &vcf, "cutoff_cv");
        } else {
            // the four-pole slope is steeper than the old two-pole SVF: open it a little to keep the same brightness
            self.module(&vcf, "stock_vcf", row, json!({"frequency": vcf_knob(cutoff * 1.3), "resonance": 0.05, "cutoff_cv_amount": 1.0}), "Four-pole low-pass");
            self.con(&src.0, &src.1, &vcf, "signal");
            self.con(&fcv.0, &fcv.1, &vcf, "cutoff_cv");
        }
        if kind != "organ" && kind != "keys" && kind != "piano" && id != "pad" {
            src = (vcf.clone(), "lowpass".into());
        }
        let vca = format!("{id}_vca");
        self.module(&vca, "stock_vca", row, json!({"level": 1.0}), "VCA");
        self.con(&src.0, &src.1, &vca, "signal");
        self.con(&env, "out", &vca, "cv");
        (vca, "out".into())
    }

    #[allow(clippy::too_many_arguments)]
    fn mono_voice(
        &mut self,
        c: &mut Ctx,
        id: &str,
        pitch: &str,
        gate: &str,
        wave: &str,
        base: f64,
        env: Value,
        lane_port: &str,
    ) {
        let osc = format!("{id}_osc");
        self.module(
            &osc,
            "multi_vco",
            5,
            json!({"frequency": 261.62558}),
            format!(
                "{} oscillator ({wave})",
                if id == "lead" {
                    "Melody"
                } else {
                    "Counter-line"
                }
            ),
        );
        let (pp, po) = self.key_src(c, pitch);
        self.con(&pp, &po, &osc, "pitch");
        // triangle and sine are already soft; only saw/square melodies get a (moving) four-pole filter
        let filtered = matches!(wave, "saw" | "square");
        let vcf = format!("{id}_vcf");
        if filtered {
            let cutoff = Self::cut(c.bright, base * c.rng.uni(0.8, 1.2), 0.6);
            self.module(&vcf, "stock_vcf", 5, json!({"frequency": vcf_knob(cutoff * 1.3), "resonance": r3(c.rng.uni(0.05, 0.15)), "cutoff_cv_amount": 1.0}), "Four-pole low-pass");
            self.con(&osc, wave, &vcf, "signal");
            self.con("motion", "sine", &vcf, "cutoff_cv");
        }
        let e = format!("{id}_env");
        self.module(&e, "adsr", 5, env, "Envelope");
        let (gn, gp) = self.gate_src(c, gate);
        self.con(&gn, &gp, &e, "gate");
        let g = format!("{id}_gate");
        self.module(
            &g,
            "stock_vca",
            5,
            json!({"level": 1.0}),
            "Envelope × song-form lane",
        );
        self.con(&e, "out", &g, "signal");
        self.con("lanes", lane_port, &g, "cv");
        let vca = format!("{id}_vca");
        self.module(&vca, "stock_vca", 5, json!({"level": 1.0}), "VCA");
        if filtered {
            self.con(&vcf, "lowpass", &vca, "signal");
        } else {
            self.con(&osc, wave, &vca, "signal");
        }
        self.con(&g, "out", &vca, "cv");
    }

    fn glitch_bus(&mut self, c: &mut Ctx, master_l: &str, master_r: &str) {
        let r = 6;
        let six = c.six;
        self.module(
            "seq_glitch",
            "score_sequencer",
            r,
            json!({"tempo": self.bpm, "running": 1}),
            "Glitch sequencer: stutter · crush + amount lane",
        );
        self.con("seq_drums", "clock", "seq_glitch", "clock");
        self.con("seq_drums", "reset", "seq_glitch", "reset");
        self.module(
            "g_in",
            "poly_merge",
            r,
            json!({"channels": 2}),
            "Mix → stereo bus",
        );
        self.con("mixer", "left", "g_in", "in1");
        self.con("mixer", "right", "g_in", "in2");
        let rep = c.rng.pick(&[1.0, 1.0, 2.0]);
        let len = (six * rep * 100_000.0).round() / 100_000.0;
        self.module(
            "cap_gate",
            "gates",
            r,
            json!({"length": len}),
            format!("Capture window (1/{})", 16.0 / rep),
        );
        self.con("seq_glitch", "gate1", "cap_gate", "in");
        self.module(
            "cap_vca",
            "stock_vca",
            r,
            json!({"level": 1.0}),
            "Capture the slice",
        );
        self.con("g_in", "out", "cap_vca", "signal");
        self.con("cap_gate", "gate", "cap_vca", "cv");
        self.module("cap_split", "poly_split", r, json!({}), "Split L/R");
        self.con("cap_vca", "out", "cap_split", "signal");
        self.module("loop", "stereo_delay", r, json!({"time_left": len, "time_right": len, "feedback": 0.92, "crossfeed": 0.0, "wet": 1.0, "dry": 0.0,
                                                  "low_cut": 10, "high_cut": 20000, "mod_depth": 0.0, "mod_rate": 0.0}), format!("Stutter loop (1/{})", 16.0 / rep));
        self.con("cap_split", "out1", "loop", "left");
        self.con("cap_split", "out2", "loop", "right");
        self.module(
            "loop_m",
            "poly_merge",
            r,
            json!({"channels": 2}),
            "Loop → stereo",
        );
        self.con("loop", "left", "loop_m", "in1");
        self.con("loop", "right", "loop_m", "in2");
        self.module(
            "g_amt",
            "poly_merge",
            r,
            json!({"channels": 2}),
            "Switches: stutter · crush",
        );
        self.con("cap_gate", "delay", "g_amt", "in1");
        self.con("seq_glitch", "gate2", "g_amt", "in2");
        self.module(
            "g_lane",
            "stock_vca",
            r,
            json!({"level": 1.0}),
            "Switches × amount lane",
        );
        self.con("g_amt", "out", "g_lane", "signal");
        self.con("seq_glitch", "automation", "g_lane", "cv");
        self.module("g_sw", "poly_split", r, json!({}), "Split switches");
        self.con("g_lane", "out", "g_sw", "signal");
        self.module(
            "stut_fade",
            "fade",
            r,
            json!({"position": 0.0, "cv_amount": 1.0, "law": 0}),
            "Dry ↔ stutter",
        );
        self.con("g_in", "out", "stut_fade", "a");
        self.con("loop_m", "out", "stut_fade", "b");
        self.con("g_sw", "out1", "stut_fade", "cv");
        let mask = c.rng.pick(&[65, 273]);
        self.module(
            "crush",
            "quantizer",
            r,
            json!({"scale_mask": mask}),
            "Bit-crush (coarse voltage grid)",
        );
        self.con("stut_fade", "out", "crush", "pitch");
        self.module(
            "crush_lp",
            "lowpass",
            r,
            json!({"cutoff": 3000}),
            "Crush low-pass 3 kHz (one-pole)",
        );
        self.con("crush", "out", "crush_lp", "signal");
        self.module(
            "crush_fade",
            "fade",
            r,
            json!({"position": 0.0, "cv_amount": 1.0, "law": 0}),
            "Dry ↔ crushed",
        );
        self.con("stut_fade", "out", "crush_fade", "a");
        self.con("crush_lp", "out", "crush_fade", "b");
        self.con("g_sw", "out2", "crush_fade", "cv");
        self.module(
            "g_out",
            "poly_split",
            r,
            json!({}),
            "Glitch bus out → master",
        );
        self.con("crush_fade", "out", "g_out", "signal");
        self.ops
            .push(Op::Move(master_l.into(), "g_out".into(), "out1".into()));
        self.ops
            .push(Op::Move(master_r.into(), "g_out".into(), "out2".into()));
        self.note(format!("stutter 1/{}", 16.0 / rep));
    }

    // ------------------------------------------------------------ outputs
    fn node_json(&self, id: &str) -> Value {
        let m = &self.modules[id];
        let mut n = json!({"id": id, "kind": m.kind, "params": m.params});
        if let Some(content) = &m.content {
            n["content"] = content.clone();
        }
        if let Some(score) = self.scores.get(id) {
            n["score"] = score.clone();
        }
        n
    }

    /// The finished patch (engine v2) for offline rendering.
    pub fn patch(&self) -> Value {
        let mut nodes = vec![];
        let mut cables: Vec<Value> = vec![];
        let mut output = Value::Null;
        for op in &self.ops {
            match op {
                Op::Add(id) => nodes.push(self.node_json(id)),
                Op::Con(id, f, o, t, i, _) => {
                    cables.push(json!({"id": id, "from": f, "output": o, "to": t, "input": i}))
                }
                Op::Move(id, node, port) => {
                    for cab in cables.iter_mut().filter(|cab| cab["id"] == *id.as_str()) {
                        cab["from"] = json!(node);
                        cab["output"] = json!(port);
                    }
                }
                Op::Out(id) => output = json!(id),
            }
        }
        json!({"version": 2, "sample_rate": 48000, "output": output, "nodes": nodes, "cables": cables})
    }

    fn edit(&self, op: &Op) -> Value {
        match op {
            Op::Add(id) => {
                let m = &self.modules[id];
                let mut a = json!({"id": id, "kind": m.kind, "row": m.row});
                if m.params.as_object().is_some_and(|p| !p.is_empty()) {
                    a["params"] = m.params.clone();
                }
                if let Some(content) = &m.content {
                    a["content"] = content.clone();
                }
                if let Some(score) = self.scores.get(id) {
                    a["score"] = score.clone();
                }
                json!({"tool": "add_module", "arguments": a})
            }
            Op::Con(id, f, o, t, i, stack) => {
                let mut a = json!({"cable_id": id, "from": f, "output": o, "to": t, "input": i});
                if *stack {
                    a["mode"] = json!("stack");
                }
                json!({"tool": "connect", "arguments": a})
            }
            Op::Out(id) => json!({"tool": "set_output", "arguments": {"id": id}}),
            Op::Move(id, node, port) => {
                json!({"tool": "move_cables", "arguments": {"cable_ids": [id], "end": "output", "node": node, "port": port}})
            }
        }
    }

    /// Ops reordered so every cable lands right after the module that completes it.
    fn ordered(&self) -> Vec<Op> {
        let mut added = std::collections::HashSet::new();
        let (mut pending, mut out): (Vec<Op>, Vec<Op>) = (vec![], vec![]);
        let ready = |op: &Op, added: &std::collections::HashSet<String>| match op {
            Op::Con(_, f, _, t, _, _) => added.contains(f) && added.contains(t),
            Op::Move(_, node, _) => added.contains(node),
            Op::Out(id) | Op::Add(id) => added.contains(id),
        };
        for op in &self.ops {
            if let Op::Add(id) = op {
                out.push(op.clone());
                added.insert(id.clone());
                let (now, rest): (Vec<Op>, Vec<Op>) =
                    pending.into_iter().partition(|p| ready(p, &added));
                out.extend(now);
                pending = rest;
            } else if ready(op, &added) {
                out.push(op.clone());
            } else {
                pending.push(op.clone());
            }
        }
        debug_assert!(pending.is_empty());
        out
    }

    /// Every edit in dependency order (for instant builds; chunk to ≤256 per apply_edits).
    pub fn edits(&self) -> Vec<Value> {
        self.ordered().iter().map(|op| self.edit(op)).collect()
    }

    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    /// Recipe card for UIs and tools.
    pub fn info(&self) -> Value {
        json!({"title": self.title, "sentence": self.sentence, "description": self.description, "notes": self.notes,
               "genre": self.genre, "mood": self.mood, "texture": self.texture, "seed": self.seed, "bpm": self.bpm, "swing": self.swing,
               "key": self.key_name, "chords": self.chord_names, "modules": self.modules.len(), "cables": self.cid,
               "strips": self.strips.iter().map(|s| s.name.clone()).collect::<Vec<_>>(), "kinds": self.kinds, "drops": self.drops, "cpu": self.cpu_estimate(),
               "controls": {"energy": self.controls.energy, "complexity": self.controls.complexity, "weirdness": self.controls.weirdness,
                            "space": self.controls.space, "length": self.controls.length},
               "genome": self.genome.to_string()})
    }
    pub fn cable_count(&self) -> usize {
        self.cid
    }

    /// Reveal: all modules (placed in their rows) for one instant transaction, then cable steps grouped by section.
    pub fn reveal(&self) -> (Vec<Value>, Vec<Value>) {
        const ORDER: [u8; 8] = [0, 7, 1, 2, 3, 4, 5, 6];
        let rank = |r: u8| ORDER.iter().position(|x| *x == r).unwrap();
        let mut mods = vec![];
        let mut by_row: BTreeMap<usize, Vec<Value>> = BTreeMap::new();
        for op in self.ordered() {
            match &op {
                Op::Add(_) => mods.push(self.edit(&op)),
                Op::Con(_, f, _, t, _, _) => {
                    let r = rank(self.modules[f].row).max(rank(self.modules[t].row));
                    by_row
                        .entry(if r <= 1 { 0 } else { r })
                        .or_default()
                        .push(self.edit(&op));
                }
                Op::Out(_) => by_row.entry(0).or_default().push(self.edit(&op)),
                Op::Move(..) => by_row.entry(rank(6)).or_default().push(self.edit(&op)),
            }
        }
        let names = [
            "Master + returns",
            "Master + returns",
            "Clock + sequencers",
            "Drums",
            "Bass",
            "Chords",
            "Melody",
            "Texture",
        ];
        let mut steps = vec![];
        for (r, edits) in by_row {
            let chunks: Vec<&[Value]> = edits.chunks(8).collect();
            for (k, chunk) in chunks.iter().enumerate() {
                let label = if chunks.len() > 1 {
                    format!("{} ({})", names[r], k + 1)
                } else {
                    names[r].to_string()
                };
                steps.push(json!({"label": label, "edits": chunk}));
            }
        }
        (mods, steps)
    }

    /// Showtime: one module per step with the cables it completes, pairing modules when the plan would exceed 64 steps.
    pub fn showtime(&self) -> Vec<Value> {
        let mut groups: Vec<Vec<Op>> = vec![];
        for op in self.ordered() {
            if matches!(op, Op::Add(_)) || groups.is_empty() {
                groups.push(vec![op]);
            } else {
                groups.last_mut().unwrap().push(op);
            }
        }
        let mut steps: Vec<(String, Vec<Value>)> = vec![];
        for g in &groups {
            let label = match &g[0] {
                Op::Add(id) => self.modules[id].label.clone(),
                _ => "Patch cables".into(),
            };
            let edits: Vec<Value> = g.iter().map(|op| self.edit(op)).collect();
            for (k, chunk) in edits.chunks(8).enumerate() {
                steps.push((
                    if k == 0 {
                        label.clone()
                    } else {
                        format!("{label} (cables)")
                    },
                    chunk.to_vec(),
                ));
            }
        }
        // merge neighbours until the plan fits the 64-step cap
        while steps.len() > 64 {
            let mut merged: Vec<(String, Vec<Value>)> = vec![];
            for s in steps.drain(..) {
                if let Some(last) = merged.last_mut() {
                    if last.1.len() + s.1.len() <= 8 && !last.0.contains(" + ") {
                        last.0 = format!("{} + {}", last.0, s.0);
                        last.1.extend(s.1);
                        continue;
                    }
                }
                merged.push(s);
            }
            if merged.len() == steps.len() {
                break;
            }
            steps = merged;
            if steps.len() > 64 {
                for s in steps.iter_mut() {
                    s.0 = s.0.replace(" + ", " · ");
                }
            }
        }
        steps
            .into_iter()
            .map(|(label, edits)| {
                let label: String = label.chars().take(150).collect();
                json!({"label": label, "edits": edits})
            })
            .collect()
    }
}
