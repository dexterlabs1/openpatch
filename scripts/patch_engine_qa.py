"""Offline QA for the Patch Engine: renders random word/seed combinations with the release CLI.

usage: patch_engine_qa.py stems N      solo every mixer strip of N random builds → per-voice level table
       patch_engine_qa.py trims FILE   summarise a stems table: median level of each trim key relative to the kick
       patch_engine_qa.py mix N        full mix of N random builds → loudness / peak / clipping / brightness
       patch_engine_qa.py gate N       full mix of N builds in both drops; exits 1 on clipping, overs, silence or
                                       loudness outside the window
       patch_engine_qa.py variety K    K word combinations × 6 seeds; exits 1 when two seeds of the same words
                                       render near-identical feature vectors

PE_CLI overrides the binary (compare against another build); PE_OFFSET starts mix renders N bars into the form;
PE_STRIPS limits stems to the named strips.
"""
import array, concurrent.futures as cf, json, math, os, random, subprocess, sys, tempfile, wave

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CLI = os.environ.get("PE_CLI") or os.path.join(ROOT, "target/release/openpatch")
GENRES = ["Deep House", "Dub Techno", "Techno", "Ambient", "Lo-fi Hip Hop", "Synthwave", "Downtempo", "UK Garage"]
MOODS = ["Dreamy", "Dark", "Euphoric", "Melancholy", "Hypnotic", "Warm", "Mysterious", "Playful"]
TEXTURES = ["Glitchy", "Dusty", "Spacious", "Crisp", "Liquid", "Minimal", "Echoing", "Pulsing"]
SECS = 12
TMP = tempfile.mkdtemp(prefix="patch-engine-qa-")


def engine(g, m, t, seed, *flags):
    return json.loads(subprocess.run([CLI, "patch-engine", g, m, t, "--seed", str(seed), *flags], check=True, capture_output=True, text=True).stdout)


def onepole_hp(x, sr, fc):
    k = 1 - math.exp(-2 * math.pi * fc / sr); y = 0.0; out = []
    for v in x:
        y += k * (v - y); out.append(v - y)
    return out


def features(mono, side, sr):
    """Timbre/rhythm fingerprint of a render: loudness, brightness, low weight, noisiness, onset rate, dynamics, width."""
    x = mono[::4]; fs = sr / 4
    db = lambda v: 20 * math.log10(max(v, 1e-9))
    rms = lambda v: math.sqrt(sum(a * a for a in v) / max(1, len(v)))
    lo = x[:]; k = 1 - math.exp(-2 * math.pi * 200 / fs); y = 0.0
    for i, v in enumerate(lo):
        y += k * (v - y); lo[i] = y
    hi = x
    for _ in range(4): hi = onepole_hp(hi, fs, 3000)
    zc = sum(1 for i in range(1, len(x)) if (x[i] >= 0) != (x[i - 1] >= 0)) / (len(x) / fs)
    hop = int(fs * 0.025); env = [rms(x[i:i + hop]) for i in range(0, len(x) - hop, hop)]
    flux = [max(0.0, db(env[i]) - db(env[i - 1])) for i in range(1, len(env))]
    onsets = sum(1 for i in range(1, len(flux) - 1) if flux[i] > 4 and flux[i] >= flux[i - 1] and flux[i] >= flux[i + 1]) / (len(x) / fs)
    edb = [db(e) for e in env]; mu = sum(edb) / len(edb)
    return [db(rms(x)), db(rms(hi)) - db(rms(x)), db(rms(lo)) - db(rms(x)), math.log(1 + zc), onsets,
            math.sqrt(sum((e - mu) ** 2 for e in edb) / len(edb)), db(rms(side)) - db(rms(x))]


def render(patch, tag, feat=False):
    j, w = os.path.join(TMP, tag + ".json"), os.path.join(TMP, tag + ".wav")
    json.dump(patch, open(j, "w"))
    if os.path.exists(w): os.remove(w)
    subprocess.run([CLI, "render", j, w, str(SECS)], check=True, capture_output=True)
    f = wave.open(w); ch = f.getnchannels(); sr = f.getframerate()
    a = array.array("h", f.readframes(f.getnframes()))
    os.remove(w); os.remove(j)
    skip = sr * 2 * ch
    a = a[skip:]
    peak = max(abs(v) for v in a) / 32768
    mono = [(a[i] + a[i + 1]) / 65536 for i in range(0, len(a) - 1, ch)]
    side = [(a[i] - a[i + 1]) / 65536 for i in range(0, len(a) - 1, ch)]
    rms = math.sqrt(sum(v * v for v in mono) / max(1, len(mono)))
    hi = mono[::4]
    for _ in range(4): hi = onepole_hp(hi, sr / 4, 4000)
    hrms = math.sqrt(sum(v * v for v in hi) / max(1, len(hi)))
    db = lambda v: 20 * math.log10(max(v, 1e-9))
    out = {"rms": db(rms), "peak": peak, "clip": sum(1 for v in a if abs(v) >= 32767), "hi": db(hrms) - db(rms)}
    if feat: out["features"] = features(mono, side, sr)
    return out


def trim_key(strip, kinds):
    """The trim_db key a strip's level is set by (see patch_engine.rs)."""
    if strip == "chords": return kinds.get("chords", "")
    if strip == "pad": return "pad:" + kinds.get("pad", "")
    if strip == "bass": return "bass:" + kinds.get("bass", "")
    if strip == "lead":
        d = kinds.get("lead_design", "classic")
        return "lead:" + (kinds.get("lead", "") if d == "classic" else d)
    return strip


def stems_job(args):
    g, m, t, seed = args
    info = engine(g, m, t, seed, "--info"); patch = engine(g, m, t, seed)
    for node in patch["nodes"]:      # calibrate with every song-form layer switched on
        if node["id"] == "seq_drums":
            for i, ln in enumerate(node["score"]["lanes"]):
                if i in (5, 6, 7): continue          # keep the filter sweep, echo throws and key lanes as composed
                top = max(p["value"] for p in ln["points"])
                ln["points"] = [{"bar": 0, "value": top, "curve": "hold"}, {"bar": 32, "value": top, "curve": "hold"}]
    out = []
    only = [x for x in os.environ.get("PE_STRIPS", "").split(",") if x]  # e.g. kick,chords,pad,bass,lead for trims
    for n, name in enumerate(info["strips"], 1):
        if only and name not in only: continue
        p = json.loads(json.dumps(patch))
        for node in p["nodes"]:
            if node["id"] == "mixer": node["params"][f"solo{n}"] = 1
        r = render(p, f"s{seed}_{n}_{os.getpid()}")
        out.append({"combo": f"{g}|{m}|{t}|{seed}", "genre": g, "strip": name, "kind": trim_key(name, info["kinds"]), **r})
    return out


OFFSET = int(os.environ.get("PE_OFFSET", "0"))  # start the render this many bars into the 64-bar song form (rotates every arrangement lane)


def ev(pts, x):
    prev = pts[0]
    for p in pts[1:]:
        if p["bar"] > x:
            if p["curve"] == "hold" or p["bar"] == prev["bar"]: return prev["value"]
            t = (x - prev["bar"]) / (p["bar"] - prev["bar"]); return prev["value"] + (p["value"] - prev["value"]) * t
        prev = p
    return prev["value"]


def rotate(lane, off):
    L = lane["length_bars"]; pts = lane["points"]; v = ev(pts, off)
    sh = sorted([dict(p, bar=p["bar"] - off) for p in pts if off < p["bar"] < L] + [dict(p, bar=p["bar"] + L - off) for p in pts if p["bar"] < off], key=lambda p: p["bar"])
    return dict(lane, points=[{"bar": 0, "value": v, "curve": "hold"}] + [p for p in sh if 0 < p["bar"] < L] + [{"bar": L, "value": v, "curve": "hold"}])


def at_bar(patch, off):
    if off:
        for node in patch["nodes"]:
            if node.get("score", {}).get("lanes"): node["score"]["lanes"] = [rotate(l, off) for l in node["score"]["lanes"]]
    return patch


def mix_job(args):
    g, m, t, seed = args
    r = render(at_bar(engine(g, m, t, seed), OFFSET), f"m{seed}_{os.getpid()}")
    return {"combo": f"{g}|{m}|{t}|{seed}", "genre": g, **r}


# loudness window (dBFS RMS of the drop, after the 2 s settle) and per-render limits for `gate`
GATE = {"rms_min": -30.0, "rms_max": -8.0, "peak_max": 0.98, "silence": -45.0}


def gate_job(args):
    g, m, t, seed = args
    patch = engine(g, m, t, seed)
    rows = []
    for off in (16, 40):
        r = render(at_bar(json.loads(json.dumps(patch)), off), f"g{seed}_{off}_{os.getpid()}")
        bad = []
        if r["clip"]: bad.append(f"{r['clip']} clipped samples")
        if r["peak"] > GATE["peak_max"]: bad.append(f"peak {r['peak']:.3f}")
        if r["rms"] < GATE["silence"]: bad.append("silent")
        elif not GATE["rms_min"] <= r["rms"] <= GATE["rms_max"]: bad.append(f"rms {r['rms']:.1f} dB")
        rows.append({"combo": f"{g}|{m}|{t}|{seed}", "bar": off, **r, "fail": bad})
    return rows


def variety_job(args):
    g, m, t, seed = args
    return {"words": f"{g}|{m}|{t}", "seed": seed, **render(at_bar(engine(g, m, t, seed), 16), f"v{seed}_{os.getpid()}", feat=True)}


def variety_report(rows):
    """Normalise every feature across all renders, then find the closest pair of seeds within each word combination."""
    feats = [r["features"] for r in rows]
    n = len(feats[0])
    mu = [sum(f[i] for f in feats) / len(feats) for i in range(n)]
    sd = [max(1e-6, math.sqrt(sum((f[i] - mu[i]) ** 2 for f in feats) / len(feats))) for i in range(n)]
    z = [[(f[i] - mu[i]) / sd[i] for i in range(n)] for f in feats]
    groups = {}
    for r, v in zip(rows, z): groups.setdefault(r["words"], []).append((r["seed"], v))
    closest, mean = [], []
    for w, items in groups.items():
        ds = [(math.dist(a[1], b[1]), a[0], b[0]) for i, a in enumerate(items) for b in items[i + 1:]]
        closest.append((min(ds), w)); mean.append(sum(d[0] for d in ds) / len(ds))
    closest.sort()
    return closest, sum(mean) / len(mean)


VARIETY_MIN = 0.35  # normalised feature distance below which two seeds of the same words count as near-duplicates


def trims_report(path):
    rows = json.load(open(path))
    kick = {}
    for r in rows:
        if r["strip"] == "kick": kick[r["combo"]] = r["rms"]
    by = {}
    for r in rows:
        if r["combo"] in kick and r["strip"] in ("chords", "pad", "bass", "lead") and r["rms"] > -90:
            by.setdefault(r["kind"], []).append(r["rms"] - kick[r["combo"]])
    for k, v in sorted(by.items()):
        v.sort(); print(f"{k:28s} n={len(v):3d}  median {v[len(v) // 2]:+6.1f} dB vs kick  (range {v[0]:+.1f} … {v[-1]:+.1f})")


if __name__ == "__main__":
    if sys.argv[1] == "trims":
        trims_report(sys.argv[2]); sys.exit(0)
    mode, n = sys.argv[1], int(sys.argv[2])
    rng = random.Random(int(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[3].isdigit() else 5)
    combos = [(GENRES[i % 8], rng.choice(MOODS), rng.choice(TEXTURES), rng.randrange(1, 99999)) for i in range(n)]
    if mode == "variety":
        combos = [(g, m, t, rng.randrange(1, 99999)) for g, m, t, _ in combos for _ in range(6)]
    job = {"stems": stems_job, "gate": gate_job, "variety": variety_job}.get(mode, mix_job)
    rows = []
    with cf.ProcessPoolExecutor(max_workers=max(2, (os.cpu_count() or 4) - 2)) as ex:
        for res in ex.map(job, combos):
            for r in (res if isinstance(res, list) else [res]):
                rows.append(r); print(json.dumps(r), flush=True)
    json.dump(rows, open(os.path.join(TMP, f"{mode}.json"), "w"))
    print("ROWS", os.path.join(TMP, f"{mode}.json"), flush=True)
    if mode == "gate":
        bad = [r for r in rows if r["fail"]]
        for r in bad: print("FAIL", r["combo"], "bar", r["bar"], ", ".join(r["fail"]))
        print(f"GATE {len(rows) - len(bad)}/{len(rows)} renders pass")
        sys.exit(1 if bad else 0)
    if mode == "variety":
        closest, mean = variety_report(rows)
        for d, w in closest[:5]: print(f"closest seeds {d:.2f}  {w}")
        print(f"VARIETY mean within-words distance {mean:.2f}; minimum {closest[0][0]:.2f} (threshold {VARIETY_MIN})")
        sys.exit(1 if closest[0][0] < VARIETY_MIN else 0)
