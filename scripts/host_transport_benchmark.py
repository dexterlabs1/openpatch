#!/usr/bin/env python3
"""Paired v2 host-transport overhead measurement using existing release CLIs.

Never builds source, runs an SDK, opens a device, or changes the DSP workload
between builds. All four workloads also exist on baseline main 5844d26. Signal
statistics must agree exactly before timing begins. Timings are offline work,
not live callback deadlines or a performance comparison with another synth.

Example (build binaries separately first):
  python3 scripts/host_transport_benchmark.py --baseline /path/to/main/openpatch \
    --candidate /path/to/candidate/openpatch --candidate-source SOURCE_ID \
    --toolchain 'RECORDED BUILD TOOLCHAIN' --output /path/to/new-report.json
"""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import random
import statistics
import sys
import tempfile
from typing import Any

from compare_engines import MCPClient, cpu_name, save_result, utc_now


SAMPLE_RATE = 48_000
SECONDS = 5.0
FRAMES = 240_000
ROUNDS = 5
ANALYSIS_SECONDS = 0.5


class RecordingClient(MCPClient):
    """Reuse the existing MCP protocol and preserve every actual wire response."""

    def __init__(self, binary: Path, workspace: Path, audit: list[dict]) -> None:
        self.audit = audit
        super().__init__(binary, workspace)

    def _write(self, message: dict) -> None:
        # Freeze the request before the caller can reuse any nested structures.
        self.audit.append({"direction": "request", "utc": utc_now(),
                           "message": json.loads(json.dumps(message, allow_nan=False))})
        super()._write(message)

    def _read_stdout(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            self.audit.append({"direction": "response", "utc": utc_now(),
                               "raw_jsonl": line.rstrip("\n")})
            self.lines.put(line)
        self.lines.put(None)


def node(identifier: str, kind: str, **parameters: float) -> dict[str, Any]:
    return {"id": identifier, "kind": kind, "params": parameters}


def cable(identifier: str, source: str, destination: str,
          input_port: str = "signal", output_port: str = "out") -> dict[str, str]:
    return {"id": identifier, "from": source, "output": output_port,
            "to": destination, "input": input_port}


def bus(identifier: str, values: list[float]) -> dict[str, Any]:
    return node(identifier, "poly_constant", channels=len(values),
                **{f"v{lane + 1}": value for lane, value in enumerate(values)})


def patch(nodes: list[dict], cables: list[dict], output: str) -> dict[str, Any]:
    return {"version": 2, "sample_rate": SAMPLE_RATE, "nodes": nodes,
            "cables": cables, "output": output, "output_port": "out"}


def workloads() -> dict[str, dict[str, Any]]:
    result = {}
    for width in (1, 16):
        nodes = [bus("a", [0.125 + lane / 256 for lane in range(width)]),
                 bus("b", [-0.0625 + lane / 512 for lane in range(width)])]
        nodes.extend(node(f"gain{index}", "gain", gain=1.0) for index in range(1, 17))
        nodes.append(node("out", "poly_sum", level=1.0))
        cables = [cable("a_first", "a", "gain1"), cable("b_first", "b", "gain1")]
        cables.extend(cable(f"chain{index}", f"gain{index}", f"gain{index + 1}")
                      for index in range(1, 16))
        cables.append(cable("last_sum", "gain16", "out"))
        result[f"legacy_routing_{width}"] = {
            "description": "Two deterministic buses stacked into sixteen unity-gain stages, then mono SUM.",
            "voices": width, "patch": patch(nodes, cables, "out"),
            "analysis_endpoints": [{"id": "gain16", "port": "out"},
                                   {"id": "out", "port": "out"}],
        }

        nodes = [bus("pitch", [lane / 12 for lane in range(width)]),
                 node("vco", "stock_vco", frequency=-12.0, pulse_width=0.37),
                 node("filter", "ladder_vcf", cutoff=1800.0, resonance=0.35, drive=1.0),
                 node("clock", "gate", frequency=1.5, duty=0.45),
                 node("envelope", "adsr", attack=0.015, decay=0.18, sustain=0.4, release=0.25),
                 node("vca", "stock_vca", level=0.7, response=1.0),
                 node("sum", "poly_sum", level=1.0), node("out", "audio_out")]
        cables = [cable("pitch_vco", "pitch", "vco", "pitch"),
                  cable("vco_filter", "vco", "filter", output_port="saw"),
                  cable("filter_vca", "filter", "vca"),
                  cable("clock_envelope", "clock", "envelope", "gate"),
                  cable("envelope_vca", "envelope", "vca", "cv"),
                  cable("vca_sum", "vca", "sum"), cable("sum_out", "sum", "out")]
        result[f"mixed_stock_analog_{width}"] = {
            "description": "Stock VCO saw, analog ladder, stock VCA; analog RC envelope driven by a fixed gate; SUM to audio adapter.",
            "voices": width, "patch": patch(nodes, cables, "out"),
            "analysis_endpoints": [{"id": "vca", "port": "out"},
                                   {"id": "out", "port": "out"}],
        }
    return result


def encoded(value: Any) -> str:
    # No tolerances or rounding. Also distinguishes reported +0.0 and -0.0.
    return json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False)


def statistics_without_revision(value: dict) -> dict:
    return {key: item for key, item in value.items() if key != "revision"}


def structural_diff(before: Any, after: Any, path: str = "") -> list[dict]:
    """Record exact metadata changes; do not impose metadata-identity gates."""
    if encoded(before) == encoded(after):
        return []
    if isinstance(before, dict) and isinstance(after, dict):
        changes = []
        for key in sorted(before.keys() | after.keys()):
            child = path + "/" + key.replace("~", "~0").replace("/", "~1")
            if key not in before:
                changes.append({"path": child, "change": "added", "candidate": after[key]})
            elif key not in after:
                changes.append({"path": child, "change": "removed", "baseline": before[key]})
            else:
                changes.extend(structural_diff(before[key], after[key], child))
        return changes
    return [{"path": path or "/", "change": "changed", "baseline": before, "candidate": after}]


def check_artifacts(result: dict, binaries: dict[str, Path]) -> None:
    """Reject shared-target/stale artifacts before analysis or benchmark calls."""
    modules = {}
    for build, catalog in result["catalogs"].items():
        values = catalog.get("modules")
        if not isinstance(values, list) or any(not isinstance(module, dict)
                                              or not isinstance(module.get("kind"), str)
                                              for module in values):
            raise RuntimeError(f"Malformed actual {build} catalog")
        modules[build] = {module["kind"]: module for module in values}
    baseline, candidate = modules["baseline"], modules["candidate"]
    expected = result["artifact_guard_inputs"]
    baseline_count = expected["baseline_kind_count"]
    candidate_count = expected["candidate_kind_count"]
    catalog_policy = expected["catalog_policy"]
    added_kind = expected["added_kind"]
    added = sorted(candidate.keys() - baseline.keys())
    removed = sorted(baseline.keys() - candidate.keys())
    result["catalog_diff"] = {
        "added_kinds": added, "removed_kinds": removed,
        "added_modules": {kind: candidate[kind] for kind in added},
        "removed_modules": {kind: baseline[kind] for kind in removed},
        "changed_existing_modules": {
            kind: changes for kind in sorted(baseline.keys() & candidate.keys())
            if (changes := structural_diff(baseline[kind], candidate[kind]))
        },
        "top_level_metadata_changes": structural_diff(
            {key: value for key, value in result["catalogs"]["baseline"].items() if key != "modules"},
            {key: value for key, value in result["catalogs"]["candidate"].items() if key != "modules"}),
        "module_order": {build: [module["kind"] for module in catalog["modules"]]
                         for build, catalog in result["catalogs"].items()},
        "catalog_policy": catalog_policy,
        "policy": ("Exact actual catalog JSON equality; no catalog additions, removals, ordering changes, or metadata changes are permitted"
                   if catalog_policy == "unchanged" else
                   "Only the expected new kind is required; changes to existing functional metadata are recorded, not rejected"),
    }
    guards = {
        "distinct_binary_sha256": result["binaries"]["baseline"]["sha256"] != result["binaries"]["candidate"]["sha256"],
        "binary_paths_still_match_recorded_sha256": all(
            hashlib.sha256(path.read_bytes()).hexdigest() == result["binaries"][build]["sha256"]
            for build, path in binaries.items()),
    }
    if catalog_policy == "unchanged":
        guards.update({
            "same_positive_catalog_kind_counts": (
                baseline_count > 0 and candidate_count > 0
                and baseline_count == candidate_count
                and len(baseline) == len(result["catalogs"]["baseline"]["modules"]) == baseline_count
                and len(candidate) == len(result["catalogs"]["candidate"]["modules"]) == candidate_count
            ),
            "actual_catalogs_exactly_equal": encoded(result["catalogs"]["baseline"])
                == encoded(result["catalogs"]["candidate"]),
            "no_added_kinds": not added,
            "no_removed_kinds": not removed,
        })
    elif catalog_policy == "added-kind":
        guards.update({
            f"baseline_has_exactly_{baseline_count}_unique_kinds": len(baseline) == len(result["catalogs"]["baseline"]["modules"]) == baseline_count,
            f"candidate_has_exactly_{candidate_count}_unique_kinds": len(candidate) == len(result["catalogs"]["candidate"]["modules"]) == candidate_count,
            f"{added_kind}_absent_from_baseline": added_kind not in baseline,
            f"{added_kind}_present_in_candidate": added_kind in candidate,
            f"only_added_kind_is_{added_kind}": added == [added_kind],
            "no_removed_kinds": not removed,
        })
    else:
        raise RuntimeError(f"Unknown catalog policy: {catalog_policy!r}")
    result["artifact_guards"] = guards
    failed = [name for name, passed in guards.items() if not passed]
    if failed:
        raise RuntimeError("Artifact provenance guard failed before analysis/timing: " + ", ".join(failed))


def validate_timing(value: dict, expected_nodes: int) -> None:
    required = {"frames": FRAMES, "sample_rate": SAMPLE_RATE, "block_size": 128,
                "nodes": expected_nodes, "debug_build": False}
    for key, expected in required.items():
        if value.get(key) != expected:
            raise RuntimeError(f"Unexpected benchmark {key}: {value.get(key)!r}; expected {expected!r}")
    if not math.isfinite(value["elapsed_seconds"]) or value["elapsed_seconds"] <= 0:
        raise RuntimeError("Invalid elapsed time")
    if not math.isfinite(value["checksum"]):
        raise RuntimeError("Nonfinite benchmark checksum")


def run(arguments: argparse.Namespace) -> dict:
    definitions = workloads()
    binaries = {"baseline": arguments.baseline.resolve(), "candidate": arguments.candidate.resolve()}
    result: dict[str, Any] = {
        "format_version": 1, "status": "running", "started_utc": utc_now(),
        "purpose": "Incremental OpenPatch v2 canonical host-transport overhead against main, offline only",
        "invocation": sys.argv,
        "binaries": {name: {"path": str(binary), "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                            "bytes": binary.stat().st_size, "source": getattr(arguments, f"{name}_source"),
                            "build_command": getattr(arguments, f"{name}_build_command")}
                     for name, binary in binaries.items()},
        "source_provenance": {"toolchain": arguments.toolchain, "rustflags": arguments.rustflags,
                              "profile": arguments.profile,
                              "authority": "Caller-supplied build provenance, not inferred from binary or current compiler",
                              "build_isolation_requirement": "Build baseline, candidate and final shipping artifact in separate target directories. A successful or fresh Cargo result against a shared target does not establish which source produced the executable; validate binary hashes and actual catalogs before analysis/timing.",
                              "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
                              "mcp_client_sha256": hashlib.sha256(Path(__file__).with_name("compare_engines.py").read_bytes()).hexdigest()},
        "host": {"cpu": cpu_name(), "platform": platform.platform(), "logical_cpus": os.cpu_count(),
                 "initial_load": list(os.getloadavg()) if hasattr(os, "getloadavg") else None},
        "method": {"sample_rate": SAMPLE_RATE, "block_size": 128, "timed_audio_seconds": SECONDS,
                   "timed_frames": FRAMES, "rounds": ROUNDS, "seed": arguments.seed,
                   "internal_warmup_frames_per_call": 2048, "unmeasured_full_warmup_calls_per_build_per_case": 1,
                   "analysis_seconds": ANALYSIS_SECONDS,
                   "order": "Deterministically shuffled workload order each round; baseline first in even zero-based rounds, candidate first in odd rounds",
                   "timing_scope": "Server benchmark Instant measures DSP and checksum reduction; excludes compilation, patch replacement and RPC",
                   "compile_latency": {"measured": False, "reason": "Existing MCP benchmark compiles before its timer; this harness adds no compilation timer or custom Core driver"},
                   "correctness": "Before timing, exact reported analyze_signal JSON statistics excluding revision only; every warmup and timed checksum must match exactly",
                   "limitations": "Four fixed workloads on one host; no callback/device/UI/SDK measurements, affinity/thermal/power control or full sample-trace equivalence; no performance advantage claim"},
        "workloads": definitions, "rpc": {name: [] for name in binaries},
        "catalogs": {}, "artifact_guards": {},
        "artifact_guard_inputs": {"baseline_kind_count": arguments.baseline_kind_count,
                                 "candidate_kind_count": arguments.candidate_kind_count,
                                 "catalog_policy": arguments.catalog_policy,
                                 "added_kind": arguments.added_kind},
        "analysis": {}, "warmup": {}, "pairs": {name: [] for name in definitions},
    }
    clients: dict[str, RecordingClient] = {}
    save_result(arguments.output, result)
    try:
        with tempfile.TemporaryDirectory(prefix="openpatch-host-transport-benchmark-") as directory:
            try:
                for name, binary in binaries.items():
                    clients[name] = RecordingClient(binary, Path(directory) / name, result["rpc"][name])
                    result["binaries"][name]["server_info"] = clients[name].server_info
                for build, client in clients.items():
                    result["catalogs"][build] = client.tool("catalog", {})
                    save_result(arguments.output, result)
                check_artifacts(result, binaries)
                save_result(arguments.output, result)
                # Complete all correctness checks before any measured benchmark.
                valid = True
                for name, definition in definitions.items():
                    checks = []
                    result["analysis"][name] = checks
                    for build, client in clients.items():
                        client.tool("replace_patch", {"patch": definition["patch"]})
                    for endpoint in definition["analysis_endpoints"]:
                        row: dict[str, Any] = {"endpoint": endpoint}
                        for build, client in clients.items():
                            row[build] = client.tool("analyze_signal", {**endpoint, "seconds": ANALYSIS_SECONDS})
                        row["exact_statistics_equal"] = encoded(statistics_without_revision(row["baseline"])) == encoded(statistics_without_revision(row["candidate"]))
                        row["solver_failures_zero"] = all(row[build]["solver_failures"] == 0 for build in binaries)
                        checks.append(row)
                        valid &= row["exact_statistics_equal"] and row["solver_failures_zero"]
                        save_result(arguments.output, result)
                if not valid:
                    raise RuntimeError("Pre-timing signal statistics differ or a solver failure occurred; timing not started")
                for name, definition in definitions.items():
                    result["warmup"][name] = {}
                    for build, client in clients.items():
                        client.tool("replace_patch", {"patch": definition["patch"]})
                        warmup = client.tool("benchmark", {"seconds": SECONDS})
                        result["warmup"][name][build] = warmup
                        validate_timing(warmup, len(definition["patch"]["nodes"]))
                    if encoded(result["warmup"][name]["baseline"]["checksum"]) != encoded(result["warmup"][name]["candidate"]["checksum"]):
                        raise RuntimeError(f"Warmup checksum differs: {name}")
                    save_result(arguments.output, result)
                rng = random.Random(arguments.seed)
                result["timing_started_utc"] = utc_now()
                for repeat in range(ROUNDS):
                    order = list(definitions)
                    rng.shuffle(order)
                    builds = ["baseline", "candidate"] if repeat % 2 == 0 else ["candidate", "baseline"]
                    for name in order:
                        definition = definitions[name]
                        pair: dict[str, Any] = {"round": repeat, "workload_order": order.copy(), "build_order": builds.copy()}
                        result["pairs"][name].append(pair)
                        for build in builds:
                            client = clients[build]
                            client.tool("replace_patch", {"patch": definition["patch"]})
                            timing = client.tool("benchmark", {"seconds": SECONDS})
                            pair[build] = {"raw_result": timing, "ns_per_frame": timing["elapsed_seconds"] * 1e9 / timing["frames"], "completed_utc": utc_now()}
                            save_result(arguments.output, result)
                            validate_timing(timing, len(definition["patch"]["nodes"]))
                        pair["exact_checksum_equal"] = encoded(pair["baseline"]["raw_result"]["checksum"]) == encoded(pair["candidate"]["raw_result"]["checksum"])
                        pair["candidate_over_baseline_time"] = pair["candidate"]["raw_result"]["elapsed_seconds"] / pair["baseline"]["raw_result"]["elapsed_seconds"]
                        save_result(arguments.output, result)
                        if not pair["exact_checksum_equal"]:
                            raise RuntimeError(f"Measured checksum differs: round {repeat}, {name}")
                        print(f"Round {repeat + 1}/{ROUNDS}: {name}", file=sys.stderr, flush=True)
                result["timing_finished_utc"] = utc_now()
            finally:
                for name, client in clients.items():
                    client.close()
                    result["binaries"][name]["stderr"] = list(client.errors)
        result["summary"] = {}
        for name, pairs in result["pairs"].items():
            baseline = [pair["baseline"]["ns_per_frame"] for pair in pairs]
            candidate = [pair["candidate"]["ns_per_frame"] for pair in pairs]
            ratios = [pair["candidate_over_baseline_time"] for pair in pairs]
            result["summary"][name] = {
                "pairs": len(pairs), "baseline_median_ns_per_frame": statistics.median(baseline),
                "candidate_median_ns_per_frame": statistics.median(candidate),
                "baseline_min_max_ns_per_frame": [min(baseline), max(baseline)],
                "candidate_min_max_ns_per_frame": [min(candidate), max(candidate)],
                "paired_median_candidate_over_baseline_time": statistics.median(ratios),
                "paired_min_max_candidate_over_baseline_time": [min(ratios), max(ratios)],
                "paired_median_candidate_overhead_percent": (statistics.median(ratios) - 1) * 100,
                "all_checksums_equal": all(pair["exact_checksum_equal"] for pair in pairs),
            }
        result["status"] = "passed"
    except Exception as error:
        result["status"] = "failed"
        result["error"] = {"type": type(error).__name__, "message": str(error)}
    finally:
        result["finished_utc"] = utc_now()
        save_result(arguments.output, result)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True)
    parser.add_argument("--candidate", type=Path, required=True)
    parser.add_argument("--baseline-source", default="5844d26")
    parser.add_argument("--candidate-source", required=True)
    parser.add_argument("--baseline-kind-count", type=int, default=73,
                        help="Required baseline catalog size (unique kinds; default: 73)")
    parser.add_argument("--candidate-kind-count", type=int, default=74,
                        help="Required candidate catalog size (unique kinds; default: 74)")
    parser.add_argument("--catalog-policy", choices=("added-kind", "unchanged"), default="added-kind",
                        help="Catalog comparison policy (default: added-kind)")
    parser.add_argument("--added-kind",
                        help="Only permitted added catalog kind under added-kind policy (default: stock_mid_side)")
    parser.add_argument("--baseline-build-command", default="not supplied")
    parser.add_argument("--candidate-build-command", default="not supplied")
    parser.add_argument("--toolchain", default="not supplied")
    parser.add_argument("--rustflags", default="not supplied")
    parser.add_argument("--profile", default="release; caller must verify identical build flags")
    parser.add_argument("--seed", type=int, default=20261016)
    parser.add_argument("--output", type=Path, required=True)
    arguments = parser.parse_args()
    if arguments.catalog_policy == "added-kind":
        if arguments.baseline_kind_count < 1 or arguments.candidate_kind_count != arguments.baseline_kind_count + 1:
            parser.error("added-kind policy requires a positive baseline count and exactly one additional candidate kind")
        if arguments.added_kind is None:
            arguments.added_kind = "stock_mid_side"
        if not arguments.added_kind or arguments.added_kind != arguments.added_kind.strip():
            parser.error("Added kind must be a nonempty name without surrounding whitespace")
    else:
        if (arguments.baseline_kind_count < 1 or arguments.candidate_kind_count < 1
                or arguments.candidate_kind_count != arguments.baseline_kind_count):
            parser.error("unchanged policy requires equal positive baseline and candidate catalog counts")
        if arguments.added_kind is not None:
            parser.error("--added-kind can only be used with --catalog-policy added-kind")
        arguments.added_kind = None
    for binary in (arguments.baseline, arguments.candidate):
        if not binary.is_file():
            parser.error(f"Executable does not exist: {binary}")
    if arguments.output.exists():
        parser.error("Output exists; choose a new report path to preserve earlier attempts")
    result = run(arguments)
    print(f"{result['status']}: {arguments.output}", file=sys.stderr)
    return 0 if result["status"] == "passed" else 2


if __name__ == "__main__":
    raise SystemExit(main())
