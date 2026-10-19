"""Offline QA for the Patch Engine: renders random word/seed combinations with the release CLI.

usage: patch_engine_qa.py stems N      solo every mixer strip of N random builds → per-voice level table
       patch_engine_qa.py mix N        full mix of N random builds → loudness / peak / clipping / brightness
"""
import array, concurrent.futures as cf, json, math, os, random, subprocess, sys, tempfile, wave

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CLI = os.path.join(ROOT, "target/release/openpatch")
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


def render(patch, tag):
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
    rms = math.sqrt(sum(v * v for v in mono) / max(1, len(mono)))
    hi = mono[::4]
    for _ in range(4): hi = onepole_hp(hi, sr / 4, 4000)
    hrms = math.sqrt(sum(v * v for v in hi) / max(1, len(hi)))
    db = lambda v: 20 * math.log10(max(v, 1e-9))
    return {"rms": db(rms), "peak": peak, "clip": sum(1 for v in a if abs(v) >= 32767), "hi": db(hrms) - db(rms)}


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
    for n, name in enumerate(info["strips"], 1):
        p = json.loads(json.dumps(patch))
        for node in p["nodes"]:
            if node["id"] == "mixer": node["params"][f"solo{n}"] = 1
        r = render(p, f"s{seed}_{n}_{os.getpid()}")
        kind = info["kinds"].get({"chords": "chords", "bass": "bass", "lead": "lead"}.get(name, ""), "")
        out.append({"combo": f"{g}|{m}|{t}|{seed}", "genre": g, "strip": name, "kind": kind, **r})
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


def mix_job(args):
    g, m, t, seed = args
    patch = engine(g, m, t, seed)
    if OFFSET:
        for node in patch["nodes"]:
            if node.get("score", {}).get("lanes"): node["score"]["lanes"] = [rotate(l, OFFSET) for l in node["score"]["lanes"]]
    r = render(patch, f"m{seed}_{os.getpid()}")
    return {"combo": f"{g}|{m}|{t}|{seed}", "genre": g, **r}


if __name__ == "__main__":
    mode, n = sys.argv[1], int(sys.argv[2])
    rng = random.Random(int(sys.argv[3]) if len(sys.argv) > 3 and sys.argv[3].isdigit() else 5)
    combos = [(GENRES[i % 8], rng.choice(MOODS), rng.choice(TEXTURES), rng.randrange(1, 99999)) for i in range(n)]
    job = stems_job if mode == "stems" else mix_job
    rows = []
    with cf.ProcessPoolExecutor(max_workers=max(2, (os.cpu_count() or 4) - 2)) as ex:
        for res in ex.map(job, combos):
            for r in (res if isinstance(res, list) else [res]):
                rows.append(r); print(json.dumps(r), flush=True)
    json.dump(rows, open(os.path.join(TMP, f"{mode}.json"), "w"))
    print("ROWS", os.path.join(TMP, f"{mode}.json"), flush=True)
