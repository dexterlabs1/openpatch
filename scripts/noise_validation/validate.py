#!/usr/bin/env python3
"""Independent statistical check of production seven-color DSP; needs NumPy/SciPy.

Use a release openpatch-core test executable with the ignored exporter.
Budgets declared before data collection; see docs/COLORED_NOISE.md.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import numpy as np
from scipy.signal import welch

COLORS = ['white', 'pink', 'red', 'violet', 'blue', 'gray', 'black']
RATES = [44100, 48000, 96000]
SEEDS = [1, 7391, 16777215]
SECONDS = 12
WARMUP = 2
BUDGETS = {'rms_relative': .06, 'red_rms_relative': .12,
           'slope_db_per_octave': .5, 'gray_shape_db': 2.5}


def inverse_a_db(f):
    # Standard A-weighting rational magnitude, used independently of the IIR fit.
    square = f * f
    amplitude = 12194.0**2 * square**2 / (
        (square + 20.6**2) * np.sqrt((square + 107.7**2) * (square + 737.9**2))
        * (square + 12194.0**2))
    return -20 * np.log10(amplitude) - 2


def measure(binary, output, rate):
    powers, moments, individuals, peaks = [], [], [], []
    for seed in SEEDS:
        path = output / f'{rate}-{seed}.f32'
        env = dict(os.environ, OPENPATCH_NOISE_RATE=str(rate), OPENPATCH_NOISE_SEED=str(seed),
                   OPENPATCH_NOISE_SECONDS=str(SECONDS), OPENPATCH_NOISE_WARMUP=str(WARMUP),
                   OPENPATCH_NOISE_OUTPUT=str(path.resolve()))
        subprocess.run([str(binary), 'colored_noise::tests::export_colored_noise_samples',
                        '--ignored', '--exact'], env=env, check=True, timeout=90,
                       stdout=subprocess.DEVNULL)
        assert path.stat().st_size == rate * SECONDS * 7 * 4
        data = np.fromfile(path, dtype='<f4').reshape(-1, 7).astype(float)
        assert np.isfinite(data).all()
        means = np.mean(data, axis=0)
        m2 = np.mean(data**2, axis=0)
        moments.append(m2)
        rms = np.sqrt(m2)
        centered = data - means
        kurtosis = np.mean(centered**4, axis=0) / np.mean(centered**2, axis=0)**2
        correlation = np.corrcoef(data, rowvar=False)
        peaks.append(np.max(np.abs(data), axis=0))
        individuals.append({'seed': seed, 'rms_volts': rms.tolist(), 'mean_volts': means.tolist(),
                            'kurtosis': kurtosis.tolist(), 'correlation': correlation.tolist()})
        frequency, psd = welch(data, fs=rate, window='hann', nperseg=32768,
                               noverlap=16384, detrend=False, axis=0, scaling='density')
        powers.append(psd)
        path.unlink()
    psd = np.mean(powers, axis=0)
    rms = np.sqrt(np.mean(moments, axis=0))
    nominal = np.array([5 / np.sqrt(2)] * 6 + [5 / np.sqrt(3)])
    rms_error = np.abs(rms / nominal - 1)
    rms_budget = np.array([BUDGETS['rms_relative']] * 7)
    rms_budget[2] = BUDGETS['red_rms_relative']
    # Log-spaced bins give octaves comparable weight. Fit log power, not amplitude.
    edges = np.geomspace(200, min(5000, .15 * rate), 33)
    x, y = [], []
    for low, high in zip(edges[:-1], edges[1:]):
        band = (frequency >= low) & (frequency < high)
        x.append(np.mean(np.log2(frequency[band])))
        y.append(10 * np.log10(psd[band].mean(axis=0)))
    slopes = np.polyfit(x, np.array(y), 1)[0]
    slope_targets = np.array([0, -3, -6, 6, 3, 0, 0])
    slope_error = np.abs(slopes - slope_targets)
    slope_ok = all(slope_error[i] <= BUDGETS['slope_db_per_octave'] for i in [0, 1, 2, 3, 4, 6])
    # Average powers over one-octave bands; compare to equally averaged ideal power.
    centers = np.geomspace(100 * np.sqrt(2), min(16000, .35 * rate) / np.sqrt(2), 24)
    residual = []
    for center in centers:
        band = (frequency >= center / np.sqrt(2)) & (frequency <= center * np.sqrt(2))
        target_power = 10**(inverse_a_db(frequency[band]) / 10)
        residual.append(10 * np.log10(psd[band, 5].mean() / target_power.mean()))
    residual = np.array(residual)
    residual -= residual.mean()
    gray_error = float(np.max(np.abs(residual)))
    peak = np.max(peaks, axis=0)
    passed = bool(np.all(rms_error <= rms_budget) and slope_ok
                  and gray_error <= BUDGETS['gray_shape_db'] and peak[6] <= 5)
    result = {'rate': rate, 'passed': passed, 'pooled_rms_volts': rms.tolist(),
              'rms_relative_error': rms_error.tolist(), 'slopes_db_per_octave': slopes.tolist(),
              'gray_max_smoothed_shape_error_db': gray_error, 'peak_volts': peak.tolist(),
              'seeds': individuals}
    # Keep reduced spectra as portable evidence, not large raw sample files.
    indices = np.unique(np.geomspace(1, len(frequency)-1, 700).astype(int))
    np.savetxt(output / f'spectrum-{rate}.csv', np.column_stack([frequency[indices], psd[indices]]),
               delimiter=',', header='frequency_hz,' + ','.join(COLORS), comments='')
    print(json.dumps({k:v for k,v in result.items() if k != 'seeds'}), flush=True)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--rust-test-bin', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    binary = args.rust_test_bin.resolve(strict=True)
    args.output.mkdir(parents=True, exist_ok=True)
    sources = [Path(__file__), Path('crates/openpatch-core/src/colored_noise.rs'),
               Path('crates/openpatch-core/src/colored_noise_gray.rs'),
               Path('crates/openpatch-core/src/colored_noise/tests.rs')]
    report = {'method': 'Production DSP f32, pooled 3 seeds, 12s after 2s warmup; Welch Hann32768/50% overlap. No clipping. See docs/COLORED_NOISE.md.',
              'budgets': BUDGETS, 'colors': COLORS, 'rates': RATES, 'seeds': SEEDS,
              'source_sha256': {str(p):hashlib.sha256(p.read_bytes()).hexdigest() for p in sources},
              'rust_binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
              'results': [measure(binary, args.output, rate) for rate in RATES]}
    report['passed'] = all(row['passed'] for row in report['results'])
    (args.output / 'spectral-validation.json').write_text(json.dumps(report, indent=2) + '\n')
    if not report['passed']:
        raise SystemExit('Noise spectral contract failed; inspect report without changing gates.')


if __name__ == '__main__':
    main()
