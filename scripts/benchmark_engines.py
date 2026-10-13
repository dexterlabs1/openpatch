#!/usr/bin/env python3
"""Compare OpenPatch engine workloads through MCP; never opens audio devices.

Run after `cargo build --release -p openpatch`. Reports repeated offline DSP
throughput, not callback deadlines or performance relative to VCV Rack.
"""
import argparse
import copy
import json
import os
from pathlib import Path
import platform
import statistics
import subprocess
import tempfile


def main():
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=root / "target/release" / ("openpatch.exe" if os.name == "nt" else "openpatch"))
    parser.add_argument("--seconds", type=float, default=1.0)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--stacked", action="store_true", help="also measure stacked routing at 1/16 voices and 1/16/1024 cables")
    parser.add_argument("--oscillators", action="store_true", help="also measure four-output VCOs at 1/16 voices and the synced waveform example")
    args = parser.parse_args()
    if not 0.01 <= args.seconds <= 5 or not 1 <= args.runs <= 20:
        parser.error("seconds must be 0.01..5 and runs 1..20")
    legacy = json.loads((root / "examples/reference-voice.json").read_text())
    v2 = copy.deepcopy(legacy)
    v2["version"] = 2
    chord = json.loads((root / "examples/polyphonic-voice.json").read_text())
    sixteen = copy.deepcopy(chord)
    sixteen["nodes"][0]["params"] = {"channels": 16, **{f"v{i + 1}": i / 12 for i in range(16)}}
    scenarios = [("legacy_reference_1_voice", legacy), ("v2_reference_1_voice", v2), ("v2_chord_3_voices", chord), ("v2_chord_16_voices", sixteen)]
    if args.stacked:
        for width in [1, 16]:
            for count in [1, 16, 1024]:
                routing = {"version": 2, "sample_rate": 48000, "nodes": [
                    {"id": "source", "kind": "poly_constant", "params": {"channels": width, **{f"v{i + 1}": 0.125 for i in range(width)}}},
                    {"id": "out", "kind": "poly_sum"},
                ], "cables": [{"id": f"route_{i + 1}", "from": "source", "to": "out", "input": "signal"} for i in range(count)], "output": "out"}
                scenarios.append((f"v2_routing_{count}_cables_{width}_voices", routing))
        scenarios.append(("v2_stacked_pitch_voice", json.loads((root / "examples/stacked-pitch-voice.json").read_text())))
    if args.oscillators:
        for width in [1, 16]:
            oscillator = {"version": 2, "sample_rate": 48000, "nodes": [
                {"id": "pitch", "kind": "poly_constant", "params": {"channels": width, **{f"v{i + 1}": i / 12 for i in range(width)}}},
                {"id": "vco", "kind": "multi_vco"},
            ], "cables": [{"from": "pitch", "to": "vco", "input": "pitch"}], "output": "vco", "output_port": "saw"}
            scenarios.append((f"v2_multi_vco_{width}_voices", oscillator))
        scenarios.append(("v2_synced_waveform_voice", json.loads((root / "examples/synced-waveform-voice.json").read_text())))
    with tempfile.TemporaryDirectory(prefix="openpatch-benchmark-") as workspace:
        with subprocess.Popen([str(args.binary), "mcp", "--workspace", workspace], stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True) as process:
            request_id = 0

            def request(method, params):
                nonlocal request_id
                request_id += 1
                process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}) + "\n")
                process.stdin.flush()
                line = process.stdout.readline()
                if not line:
                    raise RuntimeError("MCP server exited before responding")
                response = json.loads(line)
                if response.get("id") != request_id or "error" in response:
                    raise RuntimeError(response)
                return response["result"]

            def tool(name, arguments):
                result = request("tools/call", {"name": name, "arguments": arguments})
                if result.get("isError"):
                    raise RuntimeError(result)
                return result["structuredContent"]

            request("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "engine-benchmark", "version": "1"}})
            process.stdin.write('{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            process.stdin.flush()
            results = []
            for name, patch in scenarios:
                tool("replace_patch", {"patch": patch})
                runs = [tool("benchmark", {"seconds": args.seconds}) for _ in range(args.runs)]
                factors = [run["realtime_factor"] for run in runs]
                results.append({"scenario": name, "patch": patch, "median_realtime_factor": statistics.median(factors), "min_realtime_factor": min(factors), "max_realtime_factor": max(factors), "runs": runs})
            process.stdin.close()
            process.wait(timeout=10)
    print(json.dumps({"platform": platform.platform(), "seconds_per_run": args.seconds, "note": "Offline throughput only; not live deadlines or a VCV Rack comparison", "scenarios": results}, indent=2))


if __name__ == "__main__":
    main()
