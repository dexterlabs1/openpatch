"""Per-module DSP cost for the Patch Engine's `module_cost` table.

usage: patch_engine_costs.py KIND[:ENGINE],...   (e.g. stock_vcf,macro_voice:2,modal_resonator)

Renders N live instances of each kind (pitch from a poly constant, triggers from a clock, signal inputs from a VCO)
for 30 s with the release CLI, subtracts a baseline with the same plumbing and no instances, and prints % of one
core per instance, mono and 4-voice: the units of `module_cost`. Best of two runs. Offline throughput only.
"""
import json, os, subprocess, sys, tempfile, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
CLI = os.environ.get("PE_CLI") or os.path.join(ROOT, "target/release/openpatch")
SECS, N = 30, 6
TMP = tempfile.mkdtemp(prefix="patch-engine-costs-")
SPEC = {m["kind"]: m for m in json.loads(subprocess.run([CLI, "catalog"], check=True, capture_output=True, text=True).stdout)["modules"]}


def patch(kind, poly, params=None):
    nodes = [{"id": "pc", "kind": "poly_constant", "params": {"channels": 4 if poly else 1, "v1": 0, "v2": 0.25, "v3": 0.33, "v4": 0.58}},
             {"id": "clk", "kind": "gate", "params": {"frequency": 4, "duty": 0.3}},
             {"id": "src", "kind": "analog_vco", "params": {}},
             {"id": "mx", "kind": "mix6", "params": {"level": 0.1}}, {"id": "out", "kind": "output", "params": {}}]
    cables = [("pc", "out", "src", "pitch"), ("mx", "out", "out", "signal")]
    for i in range(N):
        sid = f"s{i}"
        nodes.append({"id": sid, "kind": "poly_sum", "params": {}})
        cables.append((sid, "out", "mx", f"in{i + 1}"))
        if kind is None:
            cables.append(("src", "out", sid, "signal"))
            continue
        x, m = f"x{i}", SPEC[kind]
        nodes.append({"id": x, "kind": kind, "params": params or {}})
        for p in m.get("inputs", []):
            if p == "pitch": cables.append(("pc", "out", x, p))
            elif p in ("trigger", "strike", "gate", "clock"): cables.append(("clk", "out", x, p))
            elif p in ("signal", "left", "in", "in1"): cables.append(("src", "out", x, p))
        cables.append((x, m.get("outputs", ["out"])[0], sid, "signal"))
    return {"version": 2, "sample_rate": 48000, "output": "out", "nodes": nodes,
            "cables": [{"id": f"c{k}", "from": a, "output": b, "to": c, "input": d} for k, (a, b, c, d) in enumerate(cables)]}


def seconds(p, tag):
    j, w = os.path.join(TMP, tag + ".json"), os.path.join(TMP, tag + ".wav")
    json.dump(p, open(j, "w"))
    best = 1e9
    for _ in range(2):
        if os.path.exists(w): os.remove(w)
        t0 = time.perf_counter()
        subprocess.run([CLI, "render", j, w, str(SECS)], check=True, capture_output=True)
        best = min(best, time.perf_counter() - t0)
    return best


if __name__ == "__main__":
    rows = []
    for poly in (False, True):
        base = seconds(patch(None, poly), "base")
        for k in sys.argv[1].split(","):
            name, _, eng = k.partition(":")
            v = seconds(patch(name, poly, {"engine": int(eng)} if eng else None), name + eng)
            rows.append({"kind": k, "voices": 4 if poly else 1, "percent_core": round(100 * (v - base) / N / SECS, 2)})
            print(json.dumps(rows[-1]), flush=True)
