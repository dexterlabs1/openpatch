#!/usr/bin/env python3
"""Independent continuous-drive / discrete-host-SVF spectral oracle.

No production DSP or traced waveform is used to construct the reference. The
source is an analytic sine; the static curve is integrated by periodic FFTs at
32, 64 and 128 times the host rate. Frequencies at/above host Nyquist are removed
before applying an independently derived trapezoidal-integrator SVF transfer.
Fixed two-sample source delay (engine cables or explicit component delay) and
declared candidate latency are applied without
phase fitting. Traces are read-only and reports are created exclusively.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np

N = 8192
PORTS = ("low", "band", "high")
FACTORS = (32, 64, 128)
REFERENCE_VOLTS = 5.0


def db5(value):
    return 20.0 * math.log10(max(float(value), 1e-300) / REFERENCE_VOLTS)


def f32(value):
    return float(np.float32(value))


def driven_spectrum(bin_number, amplitude, drive, factor):
    """Continuous periodic source: deliberately no f32 source quantization."""
    size = N * factor
    phase = (np.arange(size, dtype=np.float64) * bin_number) % size
    x = amplitude * np.sin(2.0 * np.pi * phase / size)
    if drive > 0.0:
        gain = 1.0 + 8.0 * drive
        saturated = 5.0 * np.tanh(x * gain / 5.0) / np.tanh(gain)
        t = min(drive / 0.1, 1.0)
        blend = t * t * (3.0 - 2.0 * t)
        x = x + blend * (saturated - x)
    # Complex Fourier-series coefficients, independent of the FFT length.
    coefficients = np.fft.rfft(x)[: N // 2 + 1] / size
    coefficients[-1] = 0.0  # ideal strict host-Nyquist bandlimit
    return coefficients


def host_transfer(case):
    """Derive from trapezoidal integrators, not the runtime recurrence.

    q=(1-z^-1)/(1+z^-1)=j*tan(w/2). The two integrator equations
    q*L=g*B and q*B=g*(X-k*B-L) give the three transfer functions.
    """
    rate = case["rate"]
    cutoff = min(max(f32(case["cutoff_hz"]), 1.0), 0.45 * rate)
    g = math.tan(math.pi * cutoff / rate)
    k = 2.0 - 1.95 * f32(case["resonance"])
    w = 2.0 * np.pi * np.arange(N // 2 + 1) / N
    q = 1j * np.tan(w[:-1] / 2.0)
    denominator = q * q + k * g * q + g * g
    response = np.empty((N // 2 + 1, 3), dtype=np.complex128)
    response[:-1] = np.column_stack((g * g / denominator,
                                    g * q / denominator,
                                    q * q / denominator))
    response[-1] = (0.0, 0.0, 1.0)
    return response


def synthesize(coefficients):
    return np.fft.irfft(coefficients * N, n=N, axis=0)


def spectral_rms(coefficients, mask):
    weights = np.full(N // 2 + 1, 2.0)
    weights[[0, -1]] = 1.0
    return np.sqrt(np.sum(weights[mask, None] * np.abs(coefficients[mask]) ** 2,
                          axis=0))


def analyze(case, traces, delay, cache):
    bin_number = int(case["bin"])
    amplitude = f32(case["amplitude_volts_peak"])
    drive = f32(case["drive"])
    key = (bin_number, amplitude, drive)
    if key not in cache:
        cache[key] = [driven_spectrum(*key, factor) for factor in FACTORS]
    transfer = host_transfer(case)
    phase_delay = np.exp(-2j * np.pi * np.arange(N // 2 + 1)
                         * (2 + delay) / N)
    references = [s[:, None] * transfer * phase_delay[:, None]
                  for s in cache[key]]
    reference = references[-1]
    waveform = synthesize(reference)
    actual = np.fromfile(traces / case["trace"], dtype="<f4").astype(np.float64)
    if actual.size != 2 * N * 3:
        raise ValueError(f"{case['trace']}: expected {2*N*3} f32 values, got {actual.size}")
    actual = actual.reshape(2, N, 3)
    if not np.isfinite(actual).all():
        raise ValueError(f"{case['trace']}: nonfinite trace")
    actual_spectrum = np.mean(np.fft.rfft(actual, axis=1), axis=0) / N
    error = actual - waveform[None, :, :]
    spectral_error = actual_spectrum - reference
    harmonics = [(h, h * bin_number) for h in range(1, N // (2 * bin_number) + 1, 2)
                 if h * bin_number < N // 2]
    legal = np.zeros(N // 2 + 1, dtype=bool)
    legal[[b for _, b in harmonics]] = True
    legal_rms = spectral_rms(spectral_error, legal)
    other_rms = spectral_rms(spectral_error, ~legal)
    audible = np.arange(N // 2 + 1) * case["rate"] / N <= 18000.0
    audible_rms = spectral_rms(spectral_error, audible)
    audible_legal_rms = spectral_rms(spectral_error, audible & legal)
    audible_other_rms = spectral_rms(spectral_error, audible & ~legal)
    convergence = []
    convergence_passed = True
    for a, b, fa, fb in zip(references, references[1:], FACTORS, FACTORS[1:]):
        delta = synthesize(b - a)
        peak = np.max(np.abs(delta), axis=0)
        rms = np.sqrt(np.mean(delta * delta, axis=0))
        convergence_passed &= bool(np.all(peak < 5e-4))
        convergence.append({"factors": [fa, fb],
                            "peak_error_db5": dict(zip(PORTS, map(db5, peak))),
                            "rms_error_db5": dict(zip(PORTS, map(db5, rms)))})
    output_metrics = {}
    magnitude_passed = True
    for port_index, port in enumerate(PORTS):
        measurements = []
        gated = []
        for h, b in harmonics:
            ref = reference[b, port_index]
            measured = actual_spectrum[b, port_index]
            ref_amplitude = 2.0 * abs(ref)
            measured_amplitude = 2.0 * abs(measured)
            significant = bool(ref_amplitude > 0.005)  # -60 dB relative to 5 V peak
            is_gated = bool(b * case["rate"] / N <= 18000.0
                            and (drive == 0.0 and h == 1 or drive > 0.0 and significant))
            meaningful = ref_amplitude > 5e-10
            magnitude_db = (20.0 * math.log10(max(measured_amplitude, 1e-300)
                                             / ref_amplitude) if meaningful else None)
            phase_degrees = (float(np.angle(measured / ref, deg=True))
                             if meaningful else None)
            relative_complex_error = (abs(measured - ref) / abs(ref)
                                      if meaningful else None)
            limit = 0.1 if drive == 0.0 else 0.25
            passed = bool(not is_gated or abs(magnitude_db) <= limit)
            magnitude_passed &= passed
            measurement = {"harmonic": h, "frequency_hz": b * case["rate"] / N,
                           "reference_peak_db5": db5(ref_amplitude),
                           "actual_peak_db5": db5(measured_amplitude),
                           "magnitude_error_db": magnitude_db,
                           "phase_error_degrees": phase_degrees,
                           "complex_error_relative": relative_complex_error,
                           "complex_error_peak_db5": db5(2.0 * abs(measured-ref)),
                           "gated": is_gated, "magnitude_passed": passed}
            measurements.append(measurement)
            if is_gated:
                gated.append(measurement)
        output_metrics[port] = {
            "waveform_rms_error_db5": db5(np.sqrt(np.mean(error[:, :, port_index] ** 2))),
            "waveform_peak_error_db5": db5(np.max(np.abs(error[:, :, port_index]))),
            "period_difference_rms_db5": db5(np.sqrt(np.mean((actual[1, :, port_index]
                                                             - actual[0, :, port_index]) ** 2))),
            "legal_bin_residual_rms_db5": db5(legal_rms[port_index]),
            "other_bin_residual_rms_db5": db5(other_rms[port_index]),
            "through18k_residual_rms_db5": db5(audible_rms[port_index]),
            "through18k_legal_bin_residual_rms_db5": db5(audible_legal_rms[port_index]),
            "through18k_other_bin_residual_rms_db5": db5(audible_other_rms[port_index]),
            "gated_max_abs_magnitude_error_db": max([abs(m["magnitude_error_db"]) for m in gated], default=0.0),
            "gated_max_abs_phase_error_degrees": max([abs(m["phase_error_degrees"]) for m in gated], default=0.0),
            "gated_max_complex_error_relative": max([m["complex_error_relative"] for m in gated], default=0.0),
            "harmonics": measurements,
        }
    return {**case, "candidate_delay_host_samples": delay,
            "total_delay_host_samples": 2 + delay,
            "reference_convergence_passed": convergence_passed,
            "reference_convergence": convergence,
            "magnitude_passed": magnitude_passed,
            "outputs": output_metrics}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--traces", type=Path, required=True)
    parser.add_argument("--delay", type=int, required=True,
                        help="Declared extra candidate delay, excluding two source samples")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--baseline-reference", type=Path,
                        help="Matching baseline oracle JSON for the 40 dB improvement gate")
    args = parser.parse_args()
    if args.delay < 0:
        parser.error("--delay must be nonnegative")
    if args.output.exists():
        parser.error("--output already exists; reports are never overwritten")
    cases = json.loads((args.traces / "cases.json").read_text())
    cache = {}
    results = [analyze(case, args.traces, args.delay, cache) for case in cases]
    linear = [r for r in results if f32(r["drive"]) == 0.0]
    driven = [r for r in results if f32(r["drive"]) > 0.0]
    def summary(subset):
        if not subset:
            return {"case_count": 0, "status": "not_measured",
                    "failed_magnitude_cases": [], "alias_floor_failed_port_cases": []}
        return {"case_count": len(subset),
                "max_other_bin_residual_rms_db5": max([m["other_bin_residual_rms_db5"]
                    for r in subset for m in r["outputs"].values()], default=-6000.0),
                "alias_floor_failed_port_cases": [
                    {"trace": r["trace"], "port": port, "residual_db5": m["other_bin_residual_rms_db5"]}
                    for r in subset for port, m in r["outputs"].items()
                    if m["other_bin_residual_rms_db5"] > -60.0],
                "failed_magnitude_cases": [r["trace"] for r in subset if not r["magnitude_passed"]],
                "max_gated_abs_magnitude_error_db": max([m["gated_max_abs_magnitude_error_db"]
                    for r in subset for m in r["outputs"].values()], default=0.0),
                "max_gated_abs_phase_error_degrees": max([m["gated_max_abs_phase_error_degrees"]
                    for r in subset for m in r["outputs"].values()], default=0.0),
                "max_gated_complex_error_relative": max([m["gated_max_complex_error_relative"]
                    for r in subset for m in r["outputs"].values()], default=0.0),
                "max_waveform_rms_error_db5": max([m["waveform_rms_error_db5"]
                    for r in subset for m in r["outputs"].values()], default=-6000.0),
                "max_through18k_residual_rms_db5": max([m["through18k_residual_rms_db5"]
                    for r in subset for m in r["outputs"].values()], default=-6000.0),
                "max_through18k_other_bin_residual_rms_db5": max([m["through18k_other_bin_residual_rms_db5"]
                    for r in subset for m in r["outputs"].values()], default=-6000.0)}
    report = {
        "method": "analytic continuous sine; periodic 32/64/128x drive FFT; ideal strict host-Nyquist bandlimit; independent host trapezoidal-SVF transfer; fixed declared phase",
        "source_quantization": "continuous source oracle excludes oscillator/gain f32 quantization; traces retain it",
        "gates": {"off_harmonic_rms_db5": -60,
                  "improvement_db_when_baseline_above_minus40": 40,
                  "linear_magnitude_db": 0.1, "driven_magnitude_db": 0.25,
                  "driven_significance_peak_db5": -60, "maximum_gated_frequency_hz": 18000,
                  "reference_convergence_peak_db5": -80,
                  "phase_and_waveform": "reported; no unspecified phase/RMS threshold imposed"},
        "traces": str(args.traces.resolve()), "delay_host_samples": args.delay,
        "linear": summary(linear), "driven": summary(driven),
        "reference_convergence_failed_cases": [r["trace"] for r in results if not r["reference_convergence_passed"]],
        "max_reference_convergence_peak_db5": max([v for r in results for c in r["reference_convergence"]
                                                     for v in c["peak_error_db5"].values()]),
        "cases": results,
    }
    if args.baseline_reference:
        baseline_bytes = args.baseline_reference.read_bytes()
        baseline = json.loads(baseline_bytes)["cases"]
        if len(baseline) != len(results):
            parser.error("Baseline and candidate case counts differ")
        improvements = []
        identity = ("rate", "bin", "amplitude_volts_peak", "drive", "cutoff_hz", "resonance")
        for before, after in zip(baseline, results):
            if any(before[key] != after[key] for key in identity):
                parser.error("Baseline and candidate stimuli or ordering differ")
            if after["drive"] == 0:
                continue
            for port in PORTS:
                previous = before["outputs"][port]["other_bin_residual_rms_db5"]
                current = after["outputs"][port]["other_bin_residual_rms_db5"]
                if previous > -40:
                    improvements.append(previous-current)
        report["baseline_comparison"] = {
            "baseline_report_sha256": hashlib.sha256(baseline_bytes).hexdigest(),
            "eligible_port_cases": len(improvements),
            "minimum_improvement_db": min(improvements) if improvements else None,
            "failed_port_cases": sum(value < 40 for value in improvements),
        }
    serialized = json.dumps(report, indent=2, allow_nan=False)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as output:
        output.write(serialized + "\n")
    print(json.dumps({key: report[key] for key in ("linear", "driven", "max_reference_convergence_peak_db5",
                                                  "reference_convergence_failed_cases")}, indent=2))


if __name__ == "__main__":
    main()
