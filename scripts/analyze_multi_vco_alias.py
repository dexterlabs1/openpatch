#!/usr/bin/env python3
"""Measure off-harmonic energy of coherent VCO tones. Requires NumPy.

This isolates a lower bound on alias energy without confusing the curved
waveforms' intended harmonics with distortion. It does not establish parity.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

import numpy as np

PERIOD = 8192
RATE = 48000
WAVES = ["sine", "triangle", "saw", "square"]


def measure(binary: Path, output: Path, harmonic_bin: int, width: float) -> dict:
    frequency = harmonic_bin * RATE / PERIOD
    pitch = 0 if frequency <= 5919 else 2
    name = f"bin{harmonic_bin}_pw{width}"
    case = {
        "rate": RATE,
        "frames": 4 * PERIOD,
        "events": [{
            "frame": 0,
            "params": {"frequency": frequency / 2**pitch, "pulse_width": width},
            "inputs": {"pitch": [pitch]},
        }],
    }
    case_path = output / f"{name}.json"
    case_path.write_text(json.dumps(case, indent=2) + "\n")
    trace_path = output / f"{name}.f32"
    trace_path.unlink(missing_ok=True)
    environment = dict(
        os.environ,
        OPENPATCH_VCO_CASE=str(case_path),
        OPENPATCH_VCO_OUTPUT=str(trace_path),
    )
    subprocess.run(
        [str(binary), "multi_vco::tests::export_reference_case", "--ignored", "--exact"],
        env=environment,
        check=True,
        timeout=60,
        stdout=subprocess.DEVNULL,
    )
    expected_bytes = case["frames"] * 16 * 4 * 4
    if trace_path.stat().st_size != expected_bytes:
        raise ValueError(f"Invalid trace length: {trace_path}")
    data = np.fromfile(trace_path, dtype="<f4").reshape(-1, 16, 4)
    if not np.isfinite(data).all():
        raise ValueError(f"Nonfinite output: {trace_path}")
    wave = data[-PERIOD:, 0].astype(float)
    spectrum = np.fft.rfft(wave, axis=0) / PERIOD
    wanted = np.zeros(PERIOD // 2 + 1, dtype=bool)
    wanted[0] = True
    wanted[np.arange(harmonic_bin, PERIOD // 2 + 1, harmonic_bin)] = True
    # Parseval: real-signal positive-frequency bins occur in pairs, except
    # DC and Nyquist. No window is necessary for exact coherent excitation.
    energy = abs(spectrum) ** 2
    energy[1:-1] *= 2
    unwanted_rms = np.sqrt(energy[~wanted].sum(axis=0))
    decibels = 20 * np.log10(np.maximum(unwanted_rms, 1e-15) / 5)
    trace_path.unlink()
    print(name, [round(float(value), 1) for value in decibels], flush=True)
    return {
        "frequency": frequency,
        "pulse_width": width,
        "unwanted_rms_volts": unwanted_rms.tolist(),
        "unwanted_db_relative_5v_peak": decibels.tolist(),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-test-bin", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    binary = args.rust_test_bin.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    report = {
        "method": (
            "Coherent 8192-sample FFT at 48 kHz, last of four periods. "
            "Energy outside DC and all in-band harmonic bins. Can underestimate "
            "aliases landing in legitimate harmonic bins. This is not a "
            "harmonic waveform/phase fidelity or dynamic-modulation test."
        ),
        "wave_order": WAVES,
        "rust_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "results": [
            measure(binary, output, harmonic_bin, width)
            for harmonic_bin in [19, 137, 509, 819, 1707, 2867]
            for width in [0.01, 0.1, 0.5, 0.9, 0.99]
        ],
    }
    (output / "summary.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
