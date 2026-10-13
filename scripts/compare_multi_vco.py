#!/usr/bin/env python3
"""Original numeric comparison transport; no third-party DSP source is included.

Build the test exporter with `cargo test -p openpatch-core --lib --no-run`.
Pass its executable and an external oracle matching docs/AUDIO_OSCILLATOR.md.
Artifacts are measurements, not an automatic declaration of module parity.
"""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import math
import os
import platform
import subprocess
from pathlib import Path
from typing import Any

import numpy as np


MAX_VOICES = 16
WAVE_ORDER = ["sine", "triangle", "saw", "square"]
DISCARD_FRAMES = 512
C4_HZ = 261.6255653005986
FUNDAMENTAL_COMMIT = "e0ade9a23a9b41124ea3f4ec9aa2cb0b98c2529e"
Case = dict[str, Any]


def build_cases() -> dict[str, Case]:
    """Keep the reference matrix and its iteration order in one place."""
    cases: dict[str, Case] = {}

    def add(
        name: str,
        rate: int = 48000,
        frames: int = 8192,
        events: list[dict[str, Any]] | None = None,
    ) -> None:
        cases[name] = {"rate": rate, "frames": frames, "events": events or []}

    for rate in [44100, 48000, 96000]:
        add("default_" + str(rate), rate)
    for frequency in [110, 1000, 5000]:
        add("hz_" + str(frequency), events=[{
            "frame": 0, "params": {"frequency": frequency},
        }])
    for soft in [0, 1]:
        events = [{
            "frame": 0,
            "params": {"frequency": 392.4384, "soft_sync": soft},
            "inputs": {"sync": [-1]},
        }]
        for frame in range(101, 8192, 181):
            events.extend([
                {"frame": frame, "inputs": {"sync": [3]}},
                {"frame": frame + 1, "inputs": {"sync": [-1]}},
            ])
        add("sync_" + str(soft), events=events)
    add("linear_stop", events=[{
        "frame": 0,
        "params": {"linear_fm": 1, "fm_amount": 1},
        "inputs": {"fm": [-2]},
    }])
    add("poly16", events=[{
        "frame": 0,
        "params": {"fm_amount": 0.25, "pwm_amount": 0.5},
        "inputs": {
            "pitch": [i / 12 for i in range(16)],
            "fm": [0.3],
            "pwm": [-1, 1],
        },
    }])
    for width in [0.01, 0.1, 0.9, 0.99]:
        add("width_" + str(width), events=[{
            "frame": 0, "params": {"pulse_width": width},
        }])
    add("pwm_audio", events=[{
        "frame": i,
        **({"params": {"pwm_amount": 1}} if i == 0 else {}),
        "inputs": {"pwm": [4 * math.sin(2 * math.pi * 379 * i / 48000)]},
    } for i in range(8192)])
    add("sync_disconnect", frames=2048, events=[
        {"frame": 0, "inputs": {"sync": [-1]}},
        {"frame": 211, "inputs": {"sync": []}},
        {"frame": 400, "inputs": {"sync": [1]}},
        {"frame": 700, "inputs": {"sync": [0]}},
        {"frame": 800, "inputs": {"sync": [1]}},
    ])
    add("mode_switch", frames=4096, events=[
        {"frame": 0, "params": {"soft_sync": 1}, "inputs": {"sync": [-1]}},
        {"frame": 211, "inputs": {"sync": [3]}},
        {"frame": 312, "params": {"soft_sync": 0}},
        {"frame": 800, "params": {"linear_fm": 1, "fm_amount": 0.5}, "inputs": {"fm": [-1]}},
    ])
    add("voice_retirement", frames=4096, events=[
        {"frame": 0, "inputs": {"pitch": [i / 12 for i in range(8)]}},
        {"frame": 500, "inputs": {"pitch": [0]}},
        {"frame": 1000, "inputs": {"pitch": [i / 12 for i in range(8)]}},
    ])
    # Deliberate pulse-event correction: the pinned reference can lose an edge
    # when a narrow pulse and wrap occur inside the same sample.
    for frequency, pitch, width in [(802.734375, 0, 0.99), (4798.828125, 0, 0.99), (16798.828125, 2, 0.9)]:
        add(f"narrow_pulse_{frequency}_{width}", events=[{
            "frame": 0,
            "params": {"frequency": frequency / 2**pitch, "pulse_width": width},
            "inputs": {"pitch": [pitch]},
        }])
    return cases


def reference_rows(case: Case) -> list[dict[str, Any]]:
    """Translate exporter events into the independent oracle's CSV schema."""
    rows = []
    parameter_names = {"linear_fm": "linear", "soft_sync": "soft"}
    for event in case["events"]:
        row = {"frame": event["frame"]}
        for key, value in event.get("params", {}).items():
            if key == "frequency":
                row["freq_semitones"] = 12 * math.log2(value / C4_HZ)
            else:
                row[parameter_names.get(key, key)] = value
        for key, values in event.get("inputs", {}).items():
            row[key + "_channels"] = len(values)
            for index, value in enumerate(values):
                row[key + "_" + str(index + 1)] = value
        rows.append(row)
    return rows


def write_case_inputs(output: Path, name: str, case: Case) -> tuple[Path, Path]:
    case_path = output / (name + ".case.json")
    case_path.write_text(json.dumps(case))
    rows = reference_rows(case)
    headers = ["frame"] + sorted({key for row in rows for key in row if key != "frame"})
    csv_path = output / (name + ".inputs.csv")
    with csv_path.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, headers)
        writer.writeheader()
        writer.writerows(rows)
    return case_path, csv_path


def run_exporter(
    command: list[str],
    label: str,
    timeout: float,
    environment: dict[str, str] | None = None,
) -> None:
    try:
        subprocess.run(
            command,
            env=environment,
            check=True,
            stdout=subprocess.DEVNULL,
            timeout=timeout,
        )
    except subprocess.TimeoutExpired as error:
        raise RuntimeError(f"{label} exceeded its {timeout:g}-second timeout") from error
    except subprocess.CalledProcessError as error:
        raise RuntimeError(f"{label} failed with exit code {error.returncode}") from error


def read_trace(path: Path, frames: int) -> np.ndarray:
    """Validate the fixed frame/lane/wave layout before computing residuals."""
    expected_values = frames * MAX_VOICES * len(WAVE_ORDER)
    expected_bytes = expected_values * np.dtype("<f4").itemsize
    actual_bytes = path.stat().st_size
    if actual_bytes != expected_bytes:
        raise ValueError(f"{path}: expected {expected_bytes} bytes, got {actual_bytes}")
    values = np.fromfile(path, dtype="<f4")
    if values.size != expected_values:
        raise ValueError(f"{path}: trace changed size while reading")
    if not np.isfinite(values).all():
        raise ValueError(f"{path}: trace contains nonfinite float32 values")
    return values.reshape(frames, MAX_VOICES, len(WAVE_ORDER)).astype(float)


def compared_voices(name: str) -> int:
    if name == "poly16":
        return 16
    if name == "voice_retirement":
        return 8
    return 1


def calculate_metrics(actual: np.ndarray, reference: np.ndarray, voices: int) -> dict[str, Any]:
    if actual.shape[0] <= DISCARD_FRAMES:
        raise ValueError(f"comparison needs more than {DISCARD_FRAMES} frames")
    actual_window = actual[DISCARD_FRAMES:, :voices]
    reference_window = reference[DISCARD_FRAMES:, :voices]
    residual = actual_window - reference_window
    return {
        "rms_error_volts": np.sqrt(np.mean(residual ** 2, axis=(0, 1))).tolist(),
        "max_error_volts": np.max(abs(residual), axis=(0, 1)).tolist(),
        "rust_mean_volts": actual_window.mean(axis=(0, 1)).tolist(),
        "reference_mean_volts": reference_window.mean(axis=(0, 1)).tolist(),
    }


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def compare_case(
    name: str,
    case: Case,
    output: Path,
    rust_binary: Path,
    reference_binary: Path,
    timeout: float,
) -> dict[str, Any]:
    case_path, csv_path = write_case_inputs(output, name, case)
    reference_path = output / (name + ".reference.f32")
    actual_path = output / (name + ".rust.f32")
    # A successful process that did not export must never reuse an older trace.
    reference_path.unlink(missing_ok=True)
    actual_path.unlink(missing_ok=True)
    run_exporter([
        str(reference_binary), "--rate", str(case["rate"]), "--frames", str(case["frames"]),
        "--input", str(csv_path), "--f32", str(reference_path),
        "--csv", str(output / (name + ".reference.csv")),
    ], f"{name}: reference exporter", timeout)
    environment = dict(os.environ, OPENPATCH_VCO_CASE=str(case_path), OPENPATCH_VCO_OUTPUT=str(actual_path))
    run_exporter([
        str(rust_binary), "multi_vco::tests::export_reference_case", "--ignored", "--exact",
    ], f"{name}: Rust test exporter", timeout, environment)
    actual = read_trace(actual_path, case["frames"])
    reference = read_trace(reference_path, case["frames"])
    voices = compared_voices(name)
    metrics = calculate_metrics(actual, reference, voices)
    metrics["validation"] = {
        "sample_rate": case["rate"],
        "frames": case["frames"],
        "raw_shape": [case["frames"], MAX_VOICES, len(WAVE_ORDER)],
        "expected_bytes_per_trace": case["frames"] * MAX_VOICES * len(WAVE_ORDER) * 4,
        "finite_float32_traces": True,
        "discarded_frames": DISCARD_FRAMES,
        "compared_frames": case["frames"] - DISCARD_FRAMES,
        "compared_voices": voices,
        "case_sha256": sha256(case_path),
        "reference_input_sha256": sha256(csv_path),
        "rust_trace_sha256": sha256(actual_path),
        "reference_trace_sha256": sha256(reference_path),
    }
    return metrics


def positive_timeout(value: str) -> float:
    timeout = float(value)
    if not math.isfinite(timeout) or timeout <= 0:
        raise argparse.ArgumentTypeError("timeout must be a finite positive number of seconds")
    return timeout


def parse_arguments(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=(
        "Compare OpenPatch's test-only VCO export with an independently built pinned "
        "Fundamental oracle. Requires NumPy. Residuals are diagnostics, not a parity pass."
    ))
    parser.add_argument("--rust-test-bin", type=Path, required=True)
    parser.add_argument("--reference-bin", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--timeout", type=positive_timeout, default=120.0,
                        help="maximum seconds per exporter subprocess (default: 120)")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_arguments(argv)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    rust_binary = args.rust_test_bin.resolve()
    reference_binary = args.reference_bin.resolve()
    summary = {}
    for name, case in build_cases().items():
        summary[name] = compare_case(name, case, output, rust_binary, reference_binary, args.timeout)
        print(name, "RMS", *[round(value, 5) for value in summary[name]["rms_error_volts"]], flush=True)
    report = {
        "reference": {"fundamental_commit": FUNDAMENTAL_COMMIT, "rack_sdk": "2.6.4"},
        "method": "Direct simultaneous four-jack voltage comparison, no cable delay, discard first 512 samples. Includes real passband/phase differences and accumulated tuning drift; these residuals are not alias-only measurements.",
        "measurement_kind": "diagnostic_residuals_not_parity_pass",
        "wave_order": WAVE_ORDER,
        "platform": platform.platform(),
        "binary_sha256": {"rust": sha256(rust_binary), "reference": sha256(reference_binary)},
        "exporter_timeout_seconds": args.timeout,
        "cases": summary,
    }
    (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
