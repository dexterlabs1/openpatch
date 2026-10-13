#!/usr/bin/env python3
"""Compare two existing OpenPatch release MCP binaries; never builds source.

Uses only the Python standard library. Timings are offline throughput, not live
audio deadlines. The JSON result includes complete deterministic workloads,
binary hashes, every randomized pair, and exact checksum/channel-stat checks.
"""
from __future__ import annotations

import argparse
from collections import deque
import datetime as dt
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import queue
import random
import statistics
import subprocess
import sys
import tempfile
import threading
from typing import Any


PROTOCOL = "2025-11-25"
SAMPLE_RATE = 48_000


def node(identifier: str, kind: str, **parameters: float) -> dict[str, Any]:
    return {"id": identifier, "kind": kind, "params": parameters}


def cable(source: str, destination: str, input_port: str = "signal") -> dict[str, str]:
    return {"from": source, "output": "out", "to": destination, "input": input_port}


def patch(nodes: list[dict], cables: list[dict], output: str) -> dict[str, Any]:
    return {"version": 2, "sample_rate": SAMPLE_RATE, "nodes": nodes,
            "cables": cables, "output": output, "output_port": "out"}


def voltage_bus(identifier: str, voices: int, values: list[float]) -> dict[str, Any]:
    return node(identifier, "poly_constant", channels=voices,
                **{f"v{i + 1}": value for i, value in enumerate(values)})


def workloads() -> dict[str, dict[str, Any]]:
    """Six main cases plus four audio-rate-CV cases; all values are volts."""
    result = {}
    for voices in (1, 16):
        fm = voltage_bus("fm", voices, [float(i) for i in range(voices)])
        sine = node("sine", "oscillator", frequency=220.0, amplitude=1.0)
        scale = node("scale", "gain", gain=2.0)
        signal_nodes = [fm, sine, scale]
        signal_edges = [cable("fm", "sine", "fm"), cable("sine", "scale")]
        ladder = node("target", "ladder_vcf", cutoff=1200.0, resonance=0.3, drive=1.0)
        vca = node("target", "analog_vca", gain=1.0)
        result[f"ladder_{voices}"] = {
            "voices": voices, "description": "+/-2 V sine at 220+i Hz; ladder pole 1200 Hz, resonance 0.3, drive 1; zero cutoff CV.",
            "patch": patch(signal_nodes + [ladder], signal_edges + [cable("scale", "target")], "target")}
        result[f"vca_{voices}"] = {
            "voices": voices, "description": "+/-2 V sine at 220+i Hz; OTA gain 1, unpatched CV normalizes to 10 V.",
            "patch": patch(signal_nodes + [vca], signal_edges + [cable("scale", "target")], "target")}
        pitch = voltage_bus("pitch", voices, [(i % 12) / 12.0 for i in range(voices)])
        full_nodes = [pitch, node("vco", "analog_vco", frequency=130.8128),
                      node("filter", "ladder_vcf", cutoff=1800.0, resonance=0.45),
                      node("clock", "gate", frequency=1.5, duty=0.45),
                      node("envelope", "adsr", attack=0.015, decay=0.18, sustain=0.4, release=0.25),
                      node("vca", "analog_vca", gain=1.0), node("out", "audio_out")]
        full_edges = [cable("pitch", "vco", "pitch"), cable("vco", "filter"),
                      cable("filter", "vca"), cable("clock", "envelope", "gate"),
                      cable("envelope", "vca", "cv"), cable("vca", "out")]
        result[f"full_voice_{voices}"] = {
            "voices": voices, "description": "Reference saw/ladder/OTA voice; semitone pitch bus; one 1.5 Hz clock/RC envelope broadcasts to all voices; audio adapter selects channel 1 but all voices process.",
            "patch": patch(full_nodes, full_edges, "out")}
        cutoff_control = node("cutoff_control", "oscillator", frequency=997.0, amplitude=0.25)
        result[f"ladder_audio_cv_{voices}"] = {
            "voices": voices, "description": "+/-2 V sine at 220+i Hz; cutoff CV +/-0.25 V at 997+i Hz, one V/oct; pole 1200 Hz, resonance 0.7, drive 1.5.",
            "patch": patch(signal_nodes + [cutoff_control, node("target", "ladder_vcf", cutoff=1200.0, resonance=0.7, drive=1.5)],
                           signal_edges + [cable("fm", "cutoff_control", "fm"), cable("scale", "target"), cable("cutoff_control", "target", "cutoff_cv")], "target")}
        cv_nodes = [node("cv_sine", "oscillator", frequency=997.0, amplitude=0.9),
                    node("cv_scale", "gain", gain=5.0), node("cv_offset", "constant", value=5.0),
                    node("cv_sum", "mixer")]
        cv_edges = [cable("fm", "cv_sine", "fm"), cable("cv_sine", "cv_scale"),
                    cable("cv_scale", "cv_sum", "a"), cable("cv_offset", "cv_sum", "b")]
        result[f"vca_audio_cv_{voices}"] = {
            "voices": voices, "description": "+/-2 V sine at 220+i Hz; OTA CV 0.5..9.5 V at 997+i Hz, gain 1; signal/control are reconstructed together.",
            "patch": patch(signal_nodes + cv_nodes + [vca], signal_edges + cv_edges + [cable("scale", "target"), cable("cv_sum", "target", "cv")], "target")}
    return result


class MCPClient:
    def __init__(self, executable: Path, workspace: Path, timeout: float = 180.0) -> None:
        self.process = subprocess.Popen([str(executable), "mcp", "--workspace", str(workspace)],
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, text=True, encoding="utf-8", bufsize=1)
        self.lines: queue.Queue[str | None] = queue.Queue()
        self.errors: deque[str] = deque(maxlen=100)
        self.timeout = timeout
        self.next_id = 1
        threading.Thread(target=self._read_stdout, daemon=True).start()
        threading.Thread(target=self._read_stderr, daemon=True).start()
        try:
            handshake = self.request("initialize", {"protocolVersion": PROTOCOL, "capabilities": {},
                                                     "clientInfo": {"name": "openpatch-engine-comparison", "version": "1"}})
            self.server_info = handshake.get("serverInfo", {})
            self._write({"jsonrpc": "2.0", "method": "notifications/initialized"})
        except BaseException:
            self.close()
            raise

    def _read_stdout(self) -> None:
        assert self.process.stdout is not None
        for line in self.process.stdout:
            self.lines.put(line)
        self.lines.put(None)

    def _read_stderr(self) -> None:
        assert self.process.stderr is not None
        for line in self.process.stderr:
            self.errors.append(line.rstrip())

    def _write(self, message: dict) -> None:
        assert self.process.stdin is not None
        self.process.stdin.write(json.dumps(message, separators=(",", ":")) + "\n")
        self.process.stdin.flush()

    def request(self, method: str, parameters: dict) -> dict:
        identifier = self.next_id
        self.next_id += 1
        self._write({"jsonrpc": "2.0", "id": identifier, "method": method, "params": parameters})
        try:
            line = self.lines.get(timeout=self.timeout)
        except queue.Empty as error:
            raise RuntimeError(f"MCP response timed out; stderr: {list(self.errors)}") from error
        if line is None:
            raise RuntimeError(f"MCP closed stdout (exit={self.process.poll()}); stderr: {list(self.errors)}")
        response = json.loads(line)
        if response.get("id") != identifier or "error" in response:
            raise RuntimeError(f"Unexpected MCP response: {response}")
        return response["result"]

    def tool(self, name: str, arguments: dict) -> dict:
        result = self.request("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise RuntimeError(f"{name}: {result}")
        text = next(item["text"] for item in result["content"] if item.get("type") == "text")
        return json.loads(text)

    def close(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
        try:
            self.process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=3)
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            if stream is not None:
                stream.close()


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def cpu_name() -> str:
    if sys.platform == "darwin":
        try:
            return subprocess.check_output(["sysctl", "-n", "machdep.cpu.brand_string"], text=True).strip()
        except (OSError, subprocess.CalledProcessError):
            pass
    if sys.platform.startswith("linux"):
        try:
            for line in Path("/proc/cpuinfo").read_text().splitlines():
                if line.startswith("model name"):
                    return line.partition(":")[2].strip()
        except OSError:
            pass
    return platform.processor() or os.environ.get("PROCESSOR_IDENTIFIER", "unknown")


def save_result(path: Path | None, result: dict) -> None:
    if path is None:
        return
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(json.dumps(result, indent=2, allow_nan=False) + "\n", encoding="utf-8")
    temporary.replace(path)


def compiler_info() -> dict[str, Any]:
    """Observe the current toolchain without pretending it is embedded metadata."""
    try:
        verbose = subprocess.check_output(["rustc", "-Vv"], stderr=subprocess.DEVNULL, text=True).strip()
    except (OSError, subprocess.CalledProcessError):
        verbose = None
    return {"current_rustc_verbose": verbose,
            "provenance": "Current shell toolchain, not extracted from the executables; callers must record which compiler built each source ID."}


def compare(arguments: argparse.Namespace) -> dict:
    definitions = workloads()
    binaries = {"baseline": arguments.baseline.resolve(), "candidate": arguments.candidate.resolve()}
    result: dict[str, Any] = {
        "format_version": 1, "started_utc": utc_now(),
        "binaries": {name: {"filename": binary.name, "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                            "source_id": getattr(arguments, name + "_source"), "bytes": binary.stat().st_size}
                     for name, binary in binaries.items()},
        "host": {"cpu": cpu_name(), "platform": platform.platform(), "logical_cpus": os.cpu_count(),
                 "initial_load": list(os.getloadavg()) if hasattr(os, "getloadavg") else None},
        "compiler": compiler_info(),
        "method": {"runs": arguments.runs, "seconds_per_call": arguments.seconds, "seed": arguments.seed,
                   "patch_version": 2, "sample_rate": SAMPLE_RATE, "block_size": 128, "benchmark_internal_warmup_frames": 2048,
                   "separate_warmup_seconds": min(0.05, arguments.seconds), "analysis_seconds": 0.5,
                   "order": "shuffled workloads each round; randomized baseline/candidate order within each pair",
                   "timing_scope": "offline DSP plus checksum reduction; compilation/RPC excluded",
                   "output_adapter": "channel zero; all active voices still process",
                   "limitations": "single host; no live callback/device/UI/GPU measurements, core affinity, thermal or power control; no VCV comparison"},
        "workloads": definitions, "pairs": {name: [] for name in definitions}}
    save_result(arguments.output, result)
    clients = {}
    with tempfile.TemporaryDirectory(prefix="openpatch-engine-compare-") as directory:
        try:
            for name, binary in binaries.items():
                clients[name] = MCPClient(binary, Path(directory) / name)
                result["binaries"][name]["server_info"] = clients[name].server_info
            for name, definition in definitions.items():
                for client in clients.values():
                    client.tool("replace_patch", {"patch": definition["patch"]})
                    warmup = client.tool("benchmark", {"seconds": min(0.05, arguments.seconds)})
                    if warmup["debug_build"]:
                        raise RuntimeError("Both binaries must be release builds")
            rng = random.Random(arguments.seed)
            for repeat in range(arguments.runs):
                order = list(definitions)
                rng.shuffle(order)
                for name in order:
                    builds = list(clients)
                    rng.shuffle(builds)
                    pair = {"repeat": repeat, "order": builds}
                    for build in builds:
                        client = clients[build]
                        client.tool("replace_patch", {"patch": definitions[name]["patch"]})
                        timing = client.tool("benchmark", {"seconds": arguments.seconds})
                        timing["ns_per_frame"] = timing["elapsed_seconds"] * 1e9 / timing["frames"]
                        timing["completed_utc"] = utc_now()
                        pair[build] = timing
                    pair["checksum_equal"] = pair["baseline"]["checksum"] == pair["candidate"]["checksum"]
                    pair["speedup"] = pair["baseline"]["elapsed_seconds"] / pair["candidate"]["elapsed_seconds"]
                    result["pairs"][name].append(pair)
                    save_result(arguments.output, result)
                print(f"Paired round {repeat + 1}/{arguments.runs} complete", file=sys.stderr, flush=True)
            result["timing_finished_utc"] = utc_now()
            print("Timed phase complete; checking exact channel statistics", file=sys.stderr, flush=True)
            result["analysis"] = {}
            for name, definition in definitions.items():
                stats = {}
                for build, client in clients.items():
                    client.tool("replace_patch", {"patch": definition["patch"]})
                    stats[build] = client.tool("analyze_signal", {"id": definition["patch"]["output"], "seconds": 0.5})
                differences = {key: {"baseline": stats["baseline"].get(key), "candidate": stats["candidate"].get(key)}
                               for key in stats["baseline"].keys() | stats["candidate"].keys()
                               if key != "revision" and stats["baseline"].get(key) != stats["candidate"].get(key)}
                result["analysis"][name] = {**stats, "differences_excluding_revision": differences}
        finally:
            for client in clients.values():
                client.close()
    result["summary"] = {}
    for name, pairs in result["pairs"].items():
        baseline = [pair["baseline"]["ns_per_frame"] for pair in pairs]
        candidate = [pair["candidate"]["ns_per_frame"] for pair in pairs]
        ratios = [pair["speedup"] for pair in pairs]
        result["summary"][name] = {
            "pairs": len(pairs), "baseline_median_ns_per_frame": statistics.median(baseline),
            "candidate_median_ns_per_frame": statistics.median(candidate),
            "baseline_min_max_ns_per_frame": [min(baseline), max(baseline)],
            "candidate_min_max_ns_per_frame": [min(candidate), max(candidate)],
            "paired_median_speedup": statistics.median(ratios), "paired_min_max_speedup": [min(ratios), max(ratios)],
            "candidate_time_reduction_percent": 100 * (1 - statistics.median(candidate) / statistics.median(baseline)),
            "all_checksums_equal": all(pair["checksum_equal"] for pair in pairs)}
    result["validation"] = {
        "all_paired_checksums_equal": all(row["all_checksums_equal"] for row in result["summary"].values()),
        "all_channel_statistics_equal": all(not row["differences_excluding_revision"] for row in result["analysis"].values()),
        "all_solver_failure_counts_zero": all(row[build]["solver_failures"] == 0 for row in result["analysis"].values() for build in binaries)}
    result["finished_utc"] = utc_now()
    save_result(arguments.output, result)
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", type=Path, required=True, help="Existing baseline release CLI executable")
    parser.add_argument("--candidate", type=Path, required=True, help="Existing candidate release CLI executable")
    parser.add_argument("--runs", type=int, default=5, help="Paired repetitions (default: 5)")
    parser.add_argument("--seconds", type=float, default=2.0, help="Audio seconds per timed call, 0.01..5 (default: 2)")
    parser.add_argument("--seed", type=int, default=2026101302, help="Deterministic randomized order seed")
    parser.add_argument("--baseline-source", default="unspecified", help="Recorded source commit/tag; not inferred from binary")
    parser.add_argument("--candidate-source", default="unspecified", help="Recorded source commit/tag; not inferred from binary")
    parser.add_argument("--output", type=Path, help="Write JSON here; otherwise JSON is printed to stdout")
    arguments = parser.parse_args()
    if arguments.runs < 1:
        parser.error("--runs must be positive")
    if not math.isfinite(arguments.seconds) or not 0.01 <= arguments.seconds <= 5.0:
        parser.error("--seconds must be finite and between 0.01 and 5")
    for binary in (arguments.baseline, arguments.candidate):
        if not binary.is_file():
            parser.error(f"Executable does not exist: {binary}")
    result = compare(arguments)
    if arguments.output is None:
        print(json.dumps(result, indent=2, allow_nan=False))
    else:
        print(f"Results: {arguments.output}", file=sys.stderr)
    return 0 if all(result["validation"].values()) else 2


if __name__ == "__main__":
    raise SystemExit(main())
